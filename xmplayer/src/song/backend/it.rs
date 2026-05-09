use crate::instrument::Instrument;
use crate::channel_state::Voice;
use crate::pattern::NoteAction;
use crate::song::backend::{
    alloc_voice, apply_flow_control_effect, apply_porta_retrig_if_needed,
    bind_voice_for_channel, dispatch_main_and_extended, init_channel_iter,
    init_voice_basics, mute_silent_voices, process_voices, set_channel_note,
    voice_mix, EffectCtx, ModuleBackend, SongPlaybackResources,
    IT_EFFECT_TABLE, IT_S_TABLE,
};

pub struct ItBackend {}

impl ItBackend {
    pub fn new() -> Self {
        Self {}
    }

    fn apply_it_action(voices: &mut [Voice], voice_idx: usize, action: u8, instrument: &Instrument) {
        let voice = &mut voices[voice_idx];
        match action {
            0 => { // Cut
                voice.on = false;
                voice.cut_reason = Some(crate::channel_state::VoiceCutReason::NoteCut);
                voice.volume.output_volume = 0.0;
            }
            1 => { // Continue
                // Do nothing
            }
            2 => { // Note Off
                if instrument.volume_envelope.on {
                    voice.sustained = false;
                } else {
                    // IT: If no volume envelope is active, Note Off = Cut
                    voice.on = false;
                    voice.volume.output_volume = 0.0;
                }
            }
            3 => { // Note Fade
                voice.sustained = false;
                voice.volume.fadeout_speed = (instrument.volume_fadeout as i32) << 6;
            }
            _ => {}
        }
    }
}

impl ModuleBackend for ItBackend {
    fn process_tick(&mut self, r: &mut SongPlaybackResources) {
        let first_tick = *r.tick == 0;
        let instruments = &r.song_data.instruments;

        // 1. Process all channels
        for (i, channel) in r.channels.iter_mut().enumerate() {
            let patterns = &r.song_data.patterns[r.song_data.pattern_order[*r.song_position] as usize];
            let row = &patterns.rows[*r.row];
            let pattern = &row.channels[i];

            let note_delay_first_tick = init_channel_iter(
                channel, pattern, instruments, r.song_data.song_type, *r.tick, first_tick,
            );

            match pattern.note_action(r.song_data.song_type) {
            NoteAction::Trigger(_) => {
                if pattern.is_porta_to_note(r.song_data.song_type) {
                    if first_tick {
                        let inst_idx = channel.last_instrument;
                        if inst_idx != 0 && (pattern.note as usize - 1) < instruments[inst_idx].sample_indexes.len() {
                            let it_mapping = instruments[inst_idx].sample_indexes[pattern.note as usize - 1];
                            let sample_idx = it_mapping.1 as usize;
                            if sample_idx > 0 && (sample_idx - 1) < instruments[inst_idx].samples.len() {
                                let sample = &instruments[inst_idx].samples[sample_idx - 1];
                                channel.porta_to_note.target_note.period = if sample.c5_speed != 0 {
                                    // IT: mapped_note = it_mapping.0 + 1 (1-indexed engine
                                    // = OpenMPT NOTE_MIN-relative + 1). Formula wants
                                    // `note - NOTE_MIN`, hence offset -1. Don't apply
                                    // sample.relative_note — IT's loader bakes the
                                    // c5_speed offset into it via -12 + LUT-rounding,
                                    // and the formula already accounts for c5_speed.
                                    let mapped = (it_mapping.0 + 1) as u8;
                                    crate::channel_state::channel_state::Note::note_to_period_s3m(mapped, -1, sample.c5_speed)
                                } else {
                                    let real_note = (it_mapping.0 as i16 + sample.relative_note as i16).clamp(1, 120) as u8;
                                    channel.note.note_to_period(real_note, sample.finetune, r.frequency_tables)
                                };
                            }
                        }
                    }
                } else if note_delay_first_tick {
                    channel.on = true;
                    let inst_idx = channel.last_instrument;
                    if inst_idx != 0 {
                        let instrument = &instruments[inst_idx];
                        let note_idx = (pattern.note - 1) as usize;
                        if note_idx < instrument.sample_indexes.len() {
                            let it_mapping = instrument.sample_indexes[note_idx];
                            let sample_idx = it_mapping.1 as usize;
                            if sample_idx > 0 && (sample_idx - 1) < instrument.samples.len() {
                                let final_sample_idx = sample_idx - 1;
                                
                                // IT DCT/DCA handling
                                let mut dca_applied = false;
                                for vi in 0..r.voices.len() {
                                    let v = &mut r.voices[vi];
                                    if !v.on || v.channel_idx != i { continue; }
                                    match instrument.dct {
                                        1 => { if v.last_played_note == pattern.note { Self::apply_it_action(r.voices, vi, instrument.dca, instrument); dca_applied = true; } }
                                        2 => { if v.sample == final_sample_idx && v.instrument == inst_idx { Self::apply_it_action(r.voices, vi, instrument.dca, instrument); dca_applied = true; } }
                                        3 => { if v.instrument == inst_idx { Self::apply_it_action(r.voices, vi, instrument.dca, instrument); dca_applied = true; } }
                                        _ => {}
                                    }
                                }
                                if !dca_applied {
                                    if let Some(v_idx) = channel.voice_idx {
                                        if r.voices[v_idx].on {
                                            Self::apply_it_action(r.voices, v_idx, instrument.nna, instrument);
                                        }
                                    }
                                }

                                let voice_idx = alloc_voice(r.voices);
                                init_voice_basics(&mut r.voices[voice_idx], i, inst_idx, final_sample_idx);
                                let voice = &mut r.voices[voice_idx];
                                voice.volume.retrig(instrument.samples[final_sample_idx].volume as i32);
                                if instrument.samples[final_sample_idx].panning < 255 {
                                    voice.panning.panning = instrument.samples[final_sample_idx].panning;
                                } else {
                                    voice.panning.panning = r.song_data.initial_channel_panning[i];
                                }

                                voice.trigger_note(instruments, pattern.instrument != 0, channel.vibrato_retrig, channel.tremolo_retrig);

                                let sample = &instrument.samples[final_sample_idx];
                                let mapped_note = it_mapping.0 + 1;
                                set_channel_note(channel, voice, sample.relative_note, sample.finetune, mapped_note, r.rate, r.frequency_tables);
                                // IT c5_speed override. Two paths:
                                //   - amiga mode: formula returns amiga-scale period and
                                //     d_period2hz_tab maps it correctly (S3M-shape).
                                //   - linear mode (most IT files): OpenMPT treats period
                                //     == freq directly (Snd_fx.cpp:6566). Our
                                //     d_period2hz_tab can't represent that mapping, so
                                //     we compute the freq via OpenMPT's IT-linear
                                //     formula (`Note::it_linear_frequency`) and stash
                                //     it on `note.linear_hz`; `Note::frequency` returns
                                //     it directly without going through the period
                                //     table. Pre-fix: dU=8.323 = 399 kHz output, voices
                                //     ultrasonic & inaudible (sweep ratio 0.04).
                                channel.note.linear_hz = 0.0;
                                if sample.c5_speed != 0 {
                                    if r.song_data.use_amiga {
                                        let p = crate::channel_state::channel_state::Note::note_to_period_s3m(mapped_note as u8, -1, sample.c5_speed);
                                        channel.note.period = p;
                                        channel.note.base_period = p;
                                    } else {
                                        let hz = crate::channel_state::channel_state::Note::it_linear_frequency(mapped_note as u8, sample.c5_speed);
                                        channel.note.linear_hz = hz;
                                    }
                                    channel.update_frequency_voice(voice, r.rate, false, r.frequency_tables);
                                }
                                voice.last_played_note = pattern.note;
                                channel.last_played_note = pattern.note;
                                channel.voice_idx = Some(voice_idx);
                            }
                        }
                    }
                }
            }
            NoteAction::Off => {
                if note_delay_first_tick {
                    if let Some(v_idx) = channel.voice_idx {
                        r.voices[v_idx].key_off(instruments, false);
                    }
                }
            }
            NoteAction::Cut => {
                if note_delay_first_tick {
                    if let Some(v_idx) = channel.voice_idx {
                        r.voices[v_idx].on = false;
                        r.voices[v_idx].cut_reason = Some(crate::channel_state::VoiceCutReason::NoteCut);
                        r.voices[v_idx].volume.output_volume = 0.0;
                    }
                }
            }
            NoteAction::Fade => {
                if note_delay_first_tick {
                    if let Some(v_idx) = channel.voice_idx {
                        r.voices[v_idx].sustained = false;
                        let instrument_nna = &instruments[r.voices[v_idx].instrument];
                        r.voices[v_idx].volume.fadeout_speed = (instrument_nna.volume_fadeout as i32) << 6;
                    }
                }
            }
            NoteAction::None => {}
            }

            apply_porta_retrig_if_needed(
                r.voices, channel, pattern, i, first_tick, instruments, r.song_data.song_type,
            );

            let mut voice_ref = bind_voice_for_channel(r.voices, channel, i);

            // Volume Column
            match pattern.volume {
                0..=64 => { channel.set_volume(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.volume); }
                65..=74 => { channel.it_vol_col_fine_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, (pattern.volume - 65) as i8); }
                75..=84 => { channel.it_vol_col_fine_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, -((pattern.volume - 75) as i8)); }
                85..=94 => { channel.it_vol_col_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, (pattern.volume - 85) as i8); }
                95..=104 => { channel.it_vol_col_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, -((pattern.volume - 95) as i8)); }
                105..=114 => { channel.porta_up(r.song_data.song_type, first_tick, (pattern.volume - 105) << 2); }
                115..=124 => { channel.porta_down(r.song_data.song_type, first_tick, (pattern.volume - 115) << 2); }
                128..=192 => { if let Some(v) = voice_ref.as_deref_mut() { v.panning.set_panning(((pattern.volume - 128) << 2) as i32); } }
                193..=202 => { channel.it_vol_col_porta_to_note(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.volume - 193, r.compatible_g, r.rate, r.frequency_tables); }
                203..=212 => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, 0, pattern.volume - 203, r.old_effects, r.rate, r.frequency_tables, r.song_data.song_type); }
                _ => {}
            }

            // Effect Column. Flow control (A/B/C/T) goes through the
            // shared helper to stay in sync with the duration-calc path.
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
                note_delay_first_tick,
                song_type: r.song_data.song_type,
                rate: r.rate,
                old_effects: r.old_effects,
                compatible_g: r.compatible_g,
                use_amiga: r.song_data.use_amiga,
            };
            dispatch_main_and_extended(
                pattern, channel, voice_ref.as_deref_mut(),
                &mut ctx, &IT_EFFECT_TABLE, &IT_S_TABLE,
            );

            if let Some(v) = voice_ref.as_deref_mut() {
                channel.update_frequency_voice(v, r.rate, false, r.frequency_tables);
            }
        }

        // 2. Process all active voices (formula-table driven; the IT
        // entry sets `instrument_global` + `apply_global_vol` with
        // div=128, matching `compute_base_volume() * inst_vol/128 *
        // global_vol/128`. sample_global is already inside
        // compute_base_volume() — don't reapply it here, that was a
        // historic double-multiply regression.)
        process_voices(
            r.voices, r.channels, instruments, r.rate,
            r.global_volume.volume, voice_mix(r.song_data.song_type),
        );

        mute_silent_voices(r.voices, r.channels);
    }
}
