use crate::pattern::NoteAction;
use crate::song::backend::{
    alloc_voice, apply_flow_control_effect, apply_porta_retrig_if_needed,
    bind_voice_for_channel, cut_or_nna_existing_voice, dispatch_main_and_extended,
    init_channel_iter, init_voice_basics, mute_silent_voices, process_voices,
    set_channel_note, voice_mix, EffectCtx, ModuleBackend, SongPlaybackResources,
    XM_EFFECT_TABLE, XM_E_TABLE,
};

pub struct XmBackend {}

impl XmBackend {
    pub fn new() -> Self {
        Self {}
    }
}

impl ModuleBackend for XmBackend {
    fn process_tick(&mut self, r: &mut SongPlaybackResources) {
        let first_tick = *r.tick == 0;
        let first_row_tick = r.first_row_tick && first_tick;

        // Context for the OUR_DUMP_CH-gated `[OUR]` debug dump.
        crate::channel_state::DUMP_CTX_ORD.store(*r.song_position as i32, std::sync::atomic::Ordering::Relaxed);
        crate::channel_state::DUMP_CTX_ROW.store(*r.row as i32, std::sync::atomic::Ordering::Relaxed);
        crate::channel_state::DUMP_CTX_TICK.store(*r.tick as i32, std::sync::atomic::Ordering::Relaxed);

        let instruments = &r.song_data.instruments;

        // 1. Process channels
        for (i, channel) in r.channels.iter_mut().enumerate() {
            let patterns = &r.song_data.patterns[r.song_data.pattern_order[*r.song_position] as usize];
            let row = &patterns.rows[*r.row];
            let pattern = &row.channels[i];

            let note_delay_first_tick = init_channel_iter(
                channel, pattern, instruments, r.song_data.song_type, *r.tick, first_tick,
            );

            // FT2: on a delayed row with no note byte, the trigger falls
            // back to the channel's last note. Without it, an
            // instrument-only delayed row never retriggers.
            let action = pattern.note_action(r.song_data.song_type);
            let trigger_note_value: u8 = match action {
                NoteAction::Trigger(n) => n,
                NoteAction::None
                    if pattern.is_note_delay(r.song_data.song_type)
                        && note_delay_first_tick
                        && channel.last_played_note != 0 =>
                {
                    channel.last_played_note
                }
                _ => 0,
            };

            if trigger_note_value != 0 {
                if pattern.is_porta_to_note(r.song_data.song_type) {
                    if first_tick {
                        let inst_idx = channel.last_instrument;
                        if inst_idx != 0 && (trigger_note_value as usize - 1) < instruments[inst_idx].sample_indexes.len() {
                            let it_mapping = instruments[inst_idx].sample_indexes[trigger_note_value as usize - 1];
                            let sample_idx = it_mapping.1 as usize;
                            if sample_idx > 0 && (sample_idx - 1) < instruments[inst_idx].samples.len() {
                                let sample = &instruments[inst_idx].samples[sample_idx - 1];
                                let real_note = (trigger_note_value as i16 + sample.relative_note as i16).clamp(1, 120) as u8;
                                channel.porta_to_note.target_note.period = channel.note.note_to_period(real_note, sample.finetune, r.frequency_tables);
                            }
                        }
                    }
                } else if note_delay_first_tick {
                    channel.on = true;
                    let inst_idx = channel.last_instrument;
                    if inst_idx != 0 {
                        let instrument = &instruments[inst_idx];
                        let note_idx = (trigger_note_value - 1) as usize;
                        if note_idx < instrument.sample_indexes.len() {
                            let it_mapping = instrument.sample_indexes[note_idx];
                            let sample_idx = it_mapping.1 as usize;
                            if sample_idx > 0 && (sample_idx - 1) < instrument.samples.len() {
                                let final_sample_idx = sample_idx - 1;

                                let prev_voice_idx = channel.voice_idx.unwrap_or(i);
                                // FT2: a note WITHOUT an instrument column keeps
                                // the current voice volume. Only an explicit
                                // instrument reloads sample default vol+pan.
                                let prev_vol = r.voices[prev_voice_idx].volume.volume;
                                cut_or_nna_existing_voice(r.voices, instruments, r.song_data.song_type, i, prev_voice_idx);

                                let voice_idx = alloc_voice(r.voices);
                                init_voice_basics(&mut r.voices[voice_idx], i, inst_idx, final_sample_idx);
                                let voice = &mut r.voices[voice_idx];
                                if pattern.instrument != 0 {
                                    voice.volume.retrig(instrument.samples[final_sample_idx].volume as i32);
                                } else {
                                    voice.volume.retrig(prev_vol as i32);
                                }
                                voice.panning.panning = r.song_data.initial_channel_panning[i];

                                // XM: a note without instrument keeps the current instrument/envelope phase.
                                voice.trigger_note(instruments, pattern.instrument != 0, channel.vibrato_retrig, channel.tremolo_retrig);

                                let sample = &instrument.samples[final_sample_idx];
                                set_channel_note(channel, voice, sample.relative_note, sample.finetune, trigger_note_value, r.rate, r.frequency_tables);
                                voice.last_played_note = trigger_note_value;
                                channel.voice_idx = Some(voice_idx);
                            }
                        }
                    }
                }
            }
            match action {
                NoteAction::Off if note_delay_first_tick => {
                    if let Some(v_idx) = channel.voice_idx {
                        r.voices[v_idx].key_off(instruments, r.song_data.song_type);
                    }
                }
                NoteAction::Cut if note_delay_first_tick => {
                    if let Some(v_idx) = channel.voice_idx {
                        r.voices[v_idx].on = false;
                        r.voices[v_idx].cut_reason = Some(crate::channel_state::VoiceCutReason::NoteCut);
                        r.voices[v_idx].volume.output_volume = 0.0;
                    }
                }
                _ => {}
            }

            apply_porta_retrig_if_needed(
                r.voices, channel, pattern, i, first_tick, instruments, r.song_data.song_type,
            );

            let mut voice_ref = bind_voice_for_channel(r.voices, channel, i);

            // Volume Column
            match pattern.volume {
                0x10..=0x50 => { channel.set_volume(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.volume - 0x10); }
                0x60..=0x6f => { channel.volume_slide(voice_ref.as_deref_mut(), first_tick, -(pattern.get_volume_param() as i8)); }
                0x70..=0x7f => { channel.volume_slide(voice_ref.as_deref_mut(), first_tick, pattern.get_volume_param() as i8); }
                0x80..=0x8f => { channel.fine_volume_slide(voice_ref.as_deref_mut(), first_tick, -(pattern.get_volume_param() as i8)); }
                0x90..=0x9f => { channel.fine_volume_slide(voice_ref.as_deref_mut(), first_tick, pattern.get_volume_param() as i8); }
                0xa0..=0xaf => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, 0, pattern.get_volume_param(), r.old_effects, r.rate, r.frequency_tables, r.song_data.song_type); }
                0xb0..=0xbf => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, pattern.get_volume_param(), 0, r.old_effects, r.rate, r.frequency_tables, r.song_data.song_type); }
                0xc0..=0xcf => { if let Some(v) = voice_ref.as_deref_mut() { v.panning.set_panning(((pattern.get_volume_param() as i32) * 17).min(255)); } }
                0xd0..=0xdf => { channel.panning_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.get_volume_param() << 4, r.song_data.song_type); }
                0xe0..=0xef => { channel.panning_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.get_volume_param(), r.song_data.song_type); }
                0xf0..=0xfe => { channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), note_delay_first_tick, pattern.get_volume_param(), r.compatible_g, r.rate, r.frequency_tables); }
                _ => {}
            }

            // Effect Column. Flow-control effects (B/D/F) go through the
            // shared helper so the duration-calc fast path uses the same
            // implementation; we drop them from this match entirely.
            if apply_flow_control_effect(
                pattern, r.song_data.song_type, first_tick,
                r.pattern_change, r.speed, r.bpm, r.rate,
            ) {
                channel.period_shift = 0;
                if let Some(v) = voice_ref.as_deref_mut() {
                    channel.update_frequency_voice(v, r.rate, false, r.frequency_tables);
                }
                continue;
            }

            // Main effect dispatch via XM_EFFECT_TABLE -> EffectKind. The
            // shared dispatch_main_and_extended also handles the Exy
            // follow-up through XM_E_TABLE when EffectKind is Extended.
            let mut ctx = EffectCtx {
                pattern_change: r.pattern_change,
                global_volume: r.global_volume,
                instruments,
                frequency_tables: r.frequency_tables,
                tick: *r.tick,
                row: *r.row,
                first_tick,
                first_row_tick,
                note_delay_first_tick,
                song_type: r.song_data.song_type,
                rate: r.rate,
                old_effects: r.old_effects,
                compatible_g: r.compatible_g,
                use_amiga: r.song_data.use_amiga,
                fast_volume_slides: r.song_data.fast_volume_slides,
            };
            dispatch_main_and_extended(
                pattern, channel, voice_ref.as_deref_mut(),
                &mut ctx, &XM_EFFECT_TABLE, &XM_E_TABLE,
            );

            if let Some(v) = voice_ref.as_deref_mut() {
                channel.update_frequency_voice(v, r.rate, false, r.frequency_tables);
                // Post-increment vibrato wave AFTER the freq update (FT2).
                if channel.vibrato_active_this_row && !first_tick {
                    channel.advance_vibrato_pos(v);
                }
            }
        }

        // 2. Process all active voices.
        process_voices(
            r.voices, r.channels, instruments, r.rate,
            r.global_volume.volume, voice_mix(r.song_data.song_type),
        );

        mute_silent_voices(r.voices, r.channels);
    }
}
