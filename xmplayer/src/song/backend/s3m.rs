use crate::pattern::NoteAction;
use crate::song::backend::{
    alloc_voice, apply_flow_control_effect, apply_porta_retrig_if_needed,
    bind_voice_for_channel, cut_or_nna_existing_voice, dispatch_main_and_extended,
    init_channel_iter, init_voice_basics, mute_silent_voices, set_channel_note,
    EffectCtx, ModuleBackend, RowTiming, SongPlaybackResources,
    S3M_EFFECT_TABLE, S3M_S_TABLE,
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
                        r.voices[v_idx].key_off(instruments, false);
                    }
                }
            }
            NoteAction::Cut => {
                if *r.tick == timing.trigger_tick {
                    if let Some(v_idx) = channel.voice_idx {
                        r.voices[v_idx].on = false;
                        r.voices[v_idx].volume.output_volume = 0.0;
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
                                cut_or_nna_existing_voice(r.voices, instruments, r.song_data.song_type, i, prev_voice_idx);

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
            // volume on the live voice. Without this, a perpetual D0A volume
            // slide on rows with only an instrument byte drains volume to 0
            // and never recovers (audible at 2ND_PM ~4:24-4:27 — the bass
            // ch1 slid to silence). OpenMPT does this in Snd_fx.cpp:2873-2964
            // (`retrigEnv = note == NOTE_NONE && instr != 0` →
            //  `chn.nVolume = oldSample->nVolume`, gated on HasSampleData
            //  for S3M). MOD has the same quirk and already handles it in
            // mod_.rs's NoteAction::None branch — this is the S3M parallel.
            NoteAction::None if pattern.instrument != 0 && first_tick => {
                let inst_idx = channel.last_instrument;
                if inst_idx != 0 {
                    let instrument = &instruments[inst_idx];
                    // Look up the sample for the most recently played note
                    // (S3M instruments have a 1:1 keyboard map, so any
                    // valid note works as a sample-index source). Use the
                    // channel's last_played_note as the reference; if it
                    // isn't set yet we have nothing to reload.
                    let lookup_note = channel.last_played_note;
                    if lookup_note >= 1 && (lookup_note as usize - 1) < instrument.sample_indexes.len() {
                        let it_mapping = instrument.sample_indexes[lookup_note as usize - 1];
                        let sample_idx = it_mapping.1 as usize;
                        if sample_idx > 0 && (sample_idx - 1) < instrument.samples.len() {
                            let sample = &instrument.samples[sample_idx - 1];
                            // OpenMPT's HasSampleData() guard.
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

            // Volume Column (S3M volume range: 0-63, 255 = no volume present).
            // The tick at which it fires is decided by the per-format
            // DelaySchedule (see backend.rs::delay_schedule). For S3M the
            // current rule is "fire at trigger tick" — change one const
            // there to flip the per-format quirk without touching this
            // call site or the other backends.
            if *r.tick == timing.vol_col_tick {
                match pattern.volume {
                    0..=64 => {
                        channel.set_volume(voice_ref.as_deref_mut(), true, pattern.volume);
                    }
                    _ => {}
                }
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
                &mut ctx, &S3M_EFFECT_TABLE, &S3M_S_TABLE,
            );

            if let Some(v) = voice_ref.as_deref_mut() {
                channel.update_frequency_voice(v, r.rate, false, r.frequency_tables);
            }
        }

        // 2. Process all active voices (S3M volume formula).
        let global_vol_f32 = r.global_volume.volume as f32 / 64.0;
        for voice in r.voices.iter_mut() {
            if !voice.on { continue; }
            let channel = &r.channels[voice.channel_idx];
            let silenced = channel.force_off || channel.tremor_silenced;

            voice.update_envelopes(instruments, r.rate);
            voice.update_fadeout();

            // S3M formula: compute_base_volume() * channel_vol/64 * global_vol/64
            // (master_volume is applied centrally in output.rs to match OpenMPT's
            // single-application model; the previous duplicate per-voice
            // multiply made our render ~2× louder than OpenMPT's reference.)
            let channel_vol = channel.channel_volume as f32 / 64.0;
            let output_vol = voice.compute_base_volume() * channel_vol * global_vol_f32;
            voice.set_output_volume(output_vol);

            if silenced {
                voice.set_output_volume(0.0);
            }
        }

        mute_silent_voices(r.voices, r.channels);
    }
}
