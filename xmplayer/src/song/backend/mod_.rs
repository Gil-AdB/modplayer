use crate::pattern::NoteAction;
use crate::song::backend::{
    alloc_voice, apply_flow_control_effect, apply_porta_retrig_if_needed,
    bind_voice_for_channel, cut_or_nna_existing_voice, dispatch_main_and_extended,
    init_channel_iter, init_voice_basics, mute_silent_voices, process_voices,
    set_channel_note, voice_mix, EffectCtx, ModuleBackend, SongPlaybackResources,
    MOD_EFFECT_TABLE, MOD_E_TABLE,
};

pub struct ModBackend {}
impl ModBackend {
    pub fn new() -> Self { Self {} }
}

impl ModuleBackend for ModBackend {
    fn process_tick(&mut self, r: &mut SongPlaybackResources) {
        let first_tick = *r.tick == 0;
        let instruments = &r.song_data.instruments;

        // 1. Process all channels
        for (i, channel) in r.channels.iter_mut().enumerate() {
            let patterns = &r.song_data.patterns[r.song_data.pattern_order[*r.song_position] as usize];
            let row = &patterns.rows[*r.row];
            let pattern = &row.channels[i];

            // MOD has no note-delay encoding so the returned value is just
            // `first_tick`; we don't need it locally — `init_channel_iter`
            // is here for the side effects (tremor reset + last_instrument
            // update).
            init_channel_iter(
                channel, pattern, instruments, r.song_data.song_type, *r.tick, first_tick,
            );

            // Note trigger logic. MOD doesn't encode Off/Cut/Fade, so the
            // only branches we need are Trigger and None - and None still
            // matters because an instrument-only row (no note) refreshes
            // sample volume/panning on the existing voice.
            match pattern.note_action(r.song_data.song_type) {
            NoteAction::Trigger(_) => {
                if pattern.effect == 0x03 || pattern.effect == 0x05 { // Tone Porta
                    if first_tick {
                        let inst_idx = channel.last_instrument;
                        if inst_idx != 0 && !instruments[inst_idx].samples.is_empty() {
                            let sample = &instruments[inst_idx].samples[0];
                            // MOD samples always have relative_note = 0 (set
                            // by module.rs::read_sample), so the (1..=120)
                            // clamp matches what the historical (0..=119)
                            // produced for any reachable input - and this
                            // way MOD shares the engine's note convention.
                            channel.porta_to_note.target_note.period = if sample.c5_speed != 0 {
                                // STM porta target — see trigger branch below.
                                crate::channel_state::channel_state::Note::note_to_period_s3m(pattern.note, 11, sample.c5_speed)
                            } else {
                                let real_note = (pattern.note as i16 + sample.relative_note as i16).clamp(1, 120) as u8;
                                channel.note.note_to_period(real_note, sample.finetune, r.frequency_tables)
                            };
                        }
                    }
                } else if first_tick {
                    channel.on = true;
                    let inst_idx = channel.last_instrument;
                    if inst_idx != 0 && !instruments[inst_idx].samples.is_empty() {
                        let sample_idx = 0;
                        let prev_voice_idx = channel.voice_idx.unwrap_or(i);
                        cut_or_nna_existing_voice(r.voices, instruments, r.song_data.song_type, i, prev_voice_idx);

                        let voice_idx = alloc_voice(r.voices);
                        init_voice_basics(&mut r.voices[voice_idx], i, inst_idx, sample_idx);
                        let voice = &mut r.voices[voice_idx];
                        if pattern.instrument != 0 {
                            voice.volume.retrig(instruments[inst_idx].samples[sample_idx].volume as i32);
                            // ProTracker hard-pans channels in an LRRL
                            // pattern and never moves them; the per-sample
                            // panning field is an FT2 extension that real
                            // MOD files don't carry.
                            voice.panning.panning = r.song_data.initial_channel_panning[i];
                        }
                        voice.trigger_note(instruments, pattern.instrument != 0, channel.vibrato_retrig, channel.tremolo_retrig);

                        let sample = &instruments[inst_idx].samples[sample_idx];
                        set_channel_note(channel, voice, sample.relative_note, sample.finetune, pattern.note, r.rate, r.frequency_tables);
                        // STM uses c5_speed (loader-populated); MOD leaves
                        // it at 0. +11 mirrors S3M (STM parser octave shift).
                        if sample.c5_speed != 0 {
                            let p = crate::channel_state::channel_state::Note::note_to_period_s3m(pattern.note, 11, sample.c5_speed);
                            channel.note.period = p;
                            channel.note.base_period = p;
                            channel.update_frequency_voice(voice, r.rate, false, r.frequency_tables);
                        }
                        voice.last_played_note = pattern.note;
                        channel.voice_idx = Some(voice_idx);
                    }
                }
            }
            // Instrument-only row (no note): refresh sample volume / panning
            // / envelope state on the existing voice. MOD-style "ghost note".
            NoteAction::None if pattern.instrument != 0 && first_tick => {
                let inst_idx = channel.last_instrument;
                if inst_idx != 0 && !instruments[inst_idx].samples.is_empty() {
                    let sample_idx = 0;
                    if let Some(voice_idx) = channel.voice_idx {
                        let voice = &mut r.voices[voice_idx];
                        if voice.on && voice.channel_idx == i {
                            voice.volume.retrig(instruments[inst_idx].samples[sample_idx].volume as i32);
                            voice.panning.panning = r.song_data.initial_channel_panning[i];
                            voice.volume_envelope_state.reset(0, &instruments[inst_idx].volume_envelope);
                            voice.panning_envelope_state.reset(0, &instruments[inst_idx].panning_envelope);
                            voice.pitch_envelope_state.reset(0, &instruments[inst_idx].pitch_envelope);
                            voice.instrument_global_volume = instruments[inst_idx].global_volume;
                            voice.sample_global_volume = instruments[inst_idx].samples[sample_idx].global_volume;
                            voice.filter_cutoff = instruments[inst_idx].initial_filter_cutoff;
                            voice.filter_resonance = instruments[inst_idx].initial_filter_resonance;
                        }
                    }
                }
            }
            // MOD doesn't have Off / Cut / Fade encodings, and bare Note::None
            // (no instrument) is also a no-op.
            _ => {}
            }

            apply_porta_retrig_if_needed(
                r.voices, channel, pattern, i, first_tick, instruments, r.song_data.song_type,
            );

            let mut voice_ref = bind_voice_for_channel(r.voices, channel, i);

            // Effect Column. Flow-control (B/D/F) is the shared helper;
            // everything else dispatches through MOD_EFFECT_TABLE -> EffectKind.
            if apply_flow_control_effect(
                pattern, r.song_data.song_type, first_tick,
                r.pattern_change, r.speed, r.bpm, r.rate,
            ) {
                if let Some(v) = voice_ref.as_deref_mut() {
                    channel.update_frequency_voice(v, r.rate, false, r.frequency_tables);
                }
                continue;
            }

            let mut ctx = EffectCtx {
                pattern_change: r.pattern_change,
                global_volume: r.global_volume,
                instruments,
                frequency_tables: r.frequency_tables,
                tick: *r.tick,
                row: *r.row,
                first_tick,
                first_row_tick: first_tick,
                note_delay_first_tick: first_tick,
                song_type: r.song_data.song_type,
                rate: r.rate,
                old_effects: r.old_effects,
                compatible_g: r.compatible_g,
                use_amiga: r.song_data.use_amiga,
                fast_volume_slides: r.song_data.fast_volume_slides,
            };
            dispatch_main_and_extended(
                pattern, channel, voice_ref.as_deref_mut(),
                &mut ctx, &MOD_EFFECT_TABLE, &MOD_E_TABLE,
            );

            if let Some(v) = voice_ref.as_deref_mut() {
                channel.update_frequency_voice(v, r.rate, false, r.frequency_tables);
                if channel.vibrato_active_this_row && !first_tick {
                    channel.advance_vibrato_pos(v);
                }
            }
        }

        // 2. Process all active voices (formula-table driven; the MOD
        // entry has `update_envelopes = false` and no global multipliers,
        // matching the previous formula `compute_base_volume()`. Going
        // through compute_base_volume() rather than a hand-rolled MOD
        // formula lets tremolo_shift land — see the historic comment that
        // used to live here for the regression that fixed.).
        process_voices(
            r.voices, r.channels, instruments, r.rate,
            r.global_volume.volume, voice_mix(r.song_data.song_type),
        );

        mute_silent_voices(r.voices, r.channels);
    }
}
