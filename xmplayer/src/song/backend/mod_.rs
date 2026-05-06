use crate::pattern::NoteAction;
use crate::song::backend::{
    alloc_voice, apply_effect, apply_extended, apply_flow_control_effect,
    cut_or_nna_existing_voice, init_voice_basics, mute_silent_voices,
    set_channel_note, EffectCtx, ModuleBackend, SongPlaybackResources,
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
            channel.tremor_silenced = false;

            let patterns = &r.song_data.patterns[r.song_data.pattern_order[*r.song_position] as usize];
            let row = &patterns.rows[*r.row];
            let pattern = &row.channels[i];

            if pattern.instrument != 0 {
                channel.last_instrument = if (pattern.instrument as usize) < instruments.len() { pattern.instrument as usize } else { 0 };
            }

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
                            let real_note = (pattern.note as i16 + sample.relative_note as i16).clamp(1, 120) as u8;
                            channel.porta_to_note.target_note.period = channel.note.note_to_period(real_note, sample.finetune, r.frequency_tables);
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
                            if instruments[inst_idx].samples[sample_idx].panning < 255 {
                                voice.panning.panning = instruments[inst_idx].samples[sample_idx].panning;
                            } else {
                                voice.panning.panning = r.song_data.initial_channel_panning[i];
                            }
                        }
                        voice.trigger_note(instruments, pattern.instrument != 0, channel.vibrato_retrig, channel.tremolo_retrig);

                        let sample = &instruments[inst_idx].samples[sample_idx];
                        set_channel_note(channel, voice, sample.relative_note, sample.finetune, pattern.note, r.rate, r.frequency_tables);
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
                            if instruments[inst_idx].samples[sample_idx].panning < 255 {
                                voice.panning.panning = instruments[inst_idx].samples[sample_idx].panning;
                            } else {
                                voice.panning.panning = r.song_data.initial_channel_panning[i];
                            }
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

            let mut voice_ref = channel.voice_idx.and_then(|idx| {
                if r.voices[idx].channel_idx == i {
                    Some(&mut r.voices[idx])
                } else {
                    None
                }
            });

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
            };
            let kind = if pattern.effect < 32 {
                MOD_EFFECT_TABLE[pattern.effect as usize]
            } else {
                crate::song::backend::EffectKind::None
            };
            let is_extended = apply_effect(kind, channel, voice_ref.as_deref_mut(), &mut ctx, pattern);
            if is_extended {
                let ext = MOD_E_TABLE[pattern.get_x() as usize];
                apply_extended(ext, channel, voice_ref.as_deref_mut(), &mut ctx, pattern.get_y());
            }

            if let Some(v) = voice_ref.as_deref_mut() {
                channel.update_frequency_voice(v, r.rate, false, r.frequency_tables);
            }
        }

        // 2. Process all active voices (MOD volume formula).
        //
        // MOD has no envelopes, no per-instrument or per-sample global volume,
        // and no song-level global volume. compute_base_volume() degenerates to
        //   (fadeout/65536) * (vol/64) + tremolo, clamped, * 1.0
        // because envelope_vol defaults to 16384 (full) and sample.global_volume
        // defaults to 64. Going through compute_base_volume() lets MOD pick up
        // tremolo_shift (effect 0x07), which the previous hand-rolled formula
        // ignored - the handler ran but the output never landed.
        for voice in r.voices.iter_mut() {
            if !voice.on { continue; }
            let channel = &r.channels[voice.channel_idx];
            let silenced = channel.force_off || channel.tremor_silenced;

            voice.update_fadeout();

            let output_vol = voice.compute_base_volume();
            voice.set_output_volume(output_vol);

            if silenced {
                voice.set_output_volume(0.0);
            }
        }

        mute_silent_voices(r.voices, r.channels);
    }
}
