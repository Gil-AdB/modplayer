use crate::pattern::NoteAction;
use crate::song::backend::{
    alloc_voice, apply_flow_control_effect, apply_porta_retrig_if_needed,
    bind_voice_for_channel, cut_or_nna_existing_voice, dispatch_main_and_extended,
    dispatch_vol_col, init_channel_iter, init_voice_basics, mute_silent_voices,
    process_voices, set_channel_note, validate_voice_pool, voice_mix, EffectCtx,
    ModuleBackend, RowTiming, SongPlaybackResources, S3M_EFFECT_TABLE, S3M_S_TABLE,
    S3M_VOL_COL,
};

pub struct S3MBackend {}
impl S3MBackend {
    pub fn new() -> Self { Self {} }
}

impl ModuleBackend for S3MBackend {
    fn process_tick(&mut self, r: &mut SongPlaybackResources) {
        let first_tick = *r.tick == 0;
        let instruments = &r.song_data.instruments;
        // 1. Process channels
        for (i, channel) in r.channels.iter_mut().enumerate() {
            let patterns = &r.song_data.patterns[r.song_data.pattern_order[*r.song_position] as usize];
            let row = &patterns.rows[*r.row];
            let pattern = &row.channels[i];

            let note_delay_first_tick = init_channel_iter(
                channel, pattern, instruments, r.song_data.song_type, *r.tick, first_tick,
            );
            let timing = RowTiming::for_row(pattern, r.song_data.song_type);

            // Note trigger logic
            // (S3M parser converts file-byte 254 -> engine 97 (Note Off);
            // there is no engine-side 253/254, so the dead checks in the
            // old code have been dropped along with this rewrite.)
            match pattern.note_action(r.song_data.song_type) {
            NoteAction::Off => {
                if *r.tick == timing.trigger_tick {
                    if let Some(v_idx) = channel.voice_idx {
                        if r.voices[v_idx].channel_idx == i {
                            r.voices[v_idx].key_off(instruments, r.song_data.song_type);
                        }
                    }
                }
            }
            NoteAction::Cut => {
                if *r.tick == timing.trigger_tick {
                    if let Some(v_idx) = channel.voice_idx {
                        if r.voices[v_idx].channel_idx == i {
                            crate::song::backend::spawn_background_cut_inline(
                                r.voices, v_idx,
                                crate::channel_state::VoiceCutReason::NoteCut,
                            );
                            channel.voice_idx = None;
                        }
                    }
                }
            }
            NoteAction::Trigger(_) => {
                if pattern.is_porta_to_note(r.song_data.song_type) {
                    if first_tick {
                        let inst_idx = channel.last_instrument;
                        if inst_idx != 0 && (pattern.note as usize - 1) < instruments[inst_idx].sample_indexes.len() {
                            let it_mapping = instruments[inst_idx].sample_indexes[pattern.note as usize - 1];
                            let sample_idx = it_mapping.1 as usize;
                            if sample_idx > 0 && (sample_idx - 1) < instruments[inst_idx].samples.len() {
                                let sample = &instruments[inst_idx].samples[sample_idx - 1];
                                // Formula path bypasses LUT quantization when
                                // c5_speed is recorded (S3M loader sets it).
                                // Use raw pattern.note (NOT pattern.note +
                                // relative_note) — the formula already folds
                                // the c5_speed offset that relative_note was a
                                // LUT-rounded representation of.
                                channel.porta_to_note.target_note.period = if sample.c5_speed != 0 {
                                    crate::channel_state::channel_state::Note::note_to_period_s3m(pattern.note, 11, sample.c5_speed)
                                } else {
                                    let real_note = (pattern.note as i16 + sample.relative_note as i16).clamp(1, 120) as u8;
                                    channel.note.note_to_period(real_note, sample.finetune, r.frequency_tables)
                                };
                            }
                        }
                    }
                } else if *r.tick == timing.trigger_tick {
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
                                
                                let prev_voice_idx = channel.voice_idx.unwrap_or(i);
                                cut_or_nna_existing_voice(r.voices, channel, instruments, r.song_data.song_type, i, prev_voice_idx);

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
                                // S3M c5_speed override: replace the LUT-derived
                                // period with the closed-form formula so we don't
                                // lose precision through (c5_speed → finetune+
                                // relative_note → 1/16-semitone LUT). Also seed
                                // channel.note.c5_speed so subsequent effects
                                // (S2 finetune, arpeggio) can read/mutate it.
                                channel.note.c5_speed = sample.c5_speed;
                                if sample.c5_speed != 0 {
                                    let p = crate::channel_state::channel_state::Note::note_to_period_s3m(pattern.note, 11, sample.c5_speed);
                                    channel.note.period = p;
                                    channel.note.base_period = p;
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
            // S3M "instrument with no note" reloads the sample's default
            // volume on the live voice (parallels mod_.rs).
            NoteAction::None if pattern.instrument != 0 && first_tick => {
                let inst_idx = channel.last_instrument;
                if inst_idx != 0 {
                    let instrument = &instruments[inst_idx];
                    let lookup_note = channel.last_played_note;
                    if lookup_note >= 1 && (lookup_note as usize - 1) < instrument.sample_indexes.len() {
                        let it_mapping = instrument.sample_indexes[lookup_note as usize - 1];
                        let sample_idx = it_mapping.1 as usize;
                        if sample_idx > 0 && (sample_idx - 1) < instrument.samples.len() {
                            let sample = &instrument.samples[sample_idx - 1];
                            if sample.length > 0 {
                                if let Some(v_idx) = channel.voice_idx {
                                    if r.voices[v_idx].on {
                                        r.voices[v_idx].volume.retrig(sample.volume as i32);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // S3M does not produce a 'Fade' encoding from the parser, and
            // None drops through.
            NoteAction::Fade | NoteAction::None => {}
            }

            apply_porta_retrig_if_needed(
                r.voices, channel, pattern, i, first_tick, instruments, r.song_data.song_type,
            );

            let mut voice_ref = bind_voice_for_channel(r.voices, channel, i);

            // For S3M, RowTiming::for_row sets vol_col_tick = trigger_tick
            // (delay_schedule(S3M).vol_col_at_trigger == true). The shared
            // dispatch_vol_col gates SetVolume on `note_delay_first_tick`,
            // which is true on exactly the same tick — so we override
            // ctx.note_delay_first_tick here to match the S3M-specific
            // schedule even when it disagrees with init_channel_iter's
            // default.
            let s3m_vol_col_fires = *r.tick == timing.vol_col_tick;
            let mut ctx = EffectCtx {
                pattern_change: r.pattern_change,
                global_volume: r.global_volume,
                instruments,
                frequency_tables: r.frequency_tables,
                tick: *r.tick,
                row: *r.row,
                first_tick,
                first_row_tick: first_tick,
                note_delay_first_tick: s3m_vol_col_fires,
                song_type: r.song_data.song_type,
                rate: r.rate,
                old_effects: r.old_effects,
                compatible_g: r.compatible_g,
                use_amiga: r.song_data.use_amiga,
                fast_volume_slides: r.song_data.fast_volume_slides,
            };

            // Volume column: data-driven via S3M_VOL_COL (just SetVolume in 0..=64).
            dispatch_vol_col(S3M_VOL_COL, pattern.volume, channel, voice_ref.as_deref_mut(), &ctx);
            // Restore for the main effect path, which expects note_delay_first_tick
            // as init_channel_iter computed it (the two values coincide for S3M
            // today, but keep them separate so the abstraction stays honest).
            ctx.note_delay_first_tick = note_delay_first_tick;

            // Effect Column. Flow control (A/B/C/T) goes through the
            // shared helper to stay in sync with the duration-calc path.
            if apply_flow_control_effect(
                pattern, r.song_data.song_type, first_tick,
                ctx.pattern_change, r.speed, r.bpm, r.rate,
            ) {
                if let Some(v) = voice_ref.as_deref_mut() {
                    channel.update_frequency_voice(v, r.rate, false, r.frequency_tables);
                }
                continue;
            }
            dispatch_main_and_extended(
                pattern, channel, voice_ref.as_deref_mut(),
                &mut ctx, &S3M_EFFECT_TABLE, &S3M_S_TABLE,
            );

            if let Some(v) = voice_ref.as_deref_mut() {
                channel.update_frequency_voice(v, r.rate, false, r.frequency_tables);
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
        validate_voice_pool(r.voices, r.channels);
    }
}
