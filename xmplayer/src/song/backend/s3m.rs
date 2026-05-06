use crate::module_reader::{SongType, is_note_valid};
use crate::song::backend::{alloc_voice, set_channel_note, ModuleBackend, SongPlaybackResources};

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

            let is_note_delay = pattern.is_note_delay(r.song_data.song_type);
            let note_delay_first_tick = if is_note_delay { *r.tick == pattern.get_y() as u32 } else { first_tick };

            if pattern.instrument != 0 {
                channel.last_instrument = if (pattern.instrument as usize) < instruments.len() { pattern.instrument as usize } else { 0 };
            }

            // Note trigger logic
            // Note trigger logic
            if pattern.note == 97 || pattern.note == 253 { // Note Off
                if note_delay_first_tick {
                    if let Some(v_idx) = channel.voice_idx {
                        r.voices[v_idx].key_off(instruments, false);
                    }
                }
            } else if pattern.note == 121 || pattern.note == 254 { // Note Cut
                if note_delay_first_tick {
                    if let Some(v_idx) = channel.voice_idx {
                        r.voices[v_idx].on = false;
                        r.voices[v_idx].volume.output_volume = 0.0;
                    }
                }
            } else if is_note_valid(pattern.note, r.song_data.song_type) {
                if pattern.is_porta_to_note(r.song_data.song_type) {
                    if first_tick {
                        let inst_idx = channel.last_instrument;
                        if inst_idx != 0 && (pattern.note as usize - 1) < instruments[inst_idx].sample_indexes.len() {
                            let it_mapping = instruments[inst_idx].sample_indexes[pattern.note as usize - 1];
                            let sample_idx = it_mapping.1 as usize;
                            if sample_idx > 0 && (sample_idx - 1) < instruments[inst_idx].samples.len() {
                                let sample = &instruments[inst_idx].samples[sample_idx - 1];
                                let real_note = (pattern.note as i16 + sample.relative_note as i16).clamp(1, 120) as u8;
                                channel.porta_to_note.target_note.period = channel.note.note_to_period(real_note, sample.finetune, r.frequency_tables);
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
                                
                                let prev_voice_idx = channel.voice_idx.unwrap_or(i);
                                if r.voices[prev_voice_idx].on && r.voices[prev_voice_idx].channel_idx == i {
                                    match r.song_data.song_type {
                                        SongType::XM | SongType::MOD => { r.voices[prev_voice_idx].on = false; }
                                        _ => {
                                            let old_inst = &instruments[r.voices[prev_voice_idx].instrument];
                                            match old_inst.nna {
                                                0 => { r.voices[prev_voice_idx].on = false; } // Cut
                                                1 => { /* Continue */ }
                                                2 => { r.voices[prev_voice_idx].key_off(instruments, false); } // Note Off
                                                3 => { r.voices[prev_voice_idx].sustained = false; } // Fade
                                                _ => { r.voices[prev_voice_idx].on = false; }
                                            }
                                        }
                                    }
                                }

                                let voice_idx = alloc_voice(r.voices);
                                let voice = &mut r.voices[voice_idx];
                                voice.on = true;
                                voice.channel_idx = i;
                                voice.instrument = inst_idx;
                                voice.sample = final_sample_idx;
                                voice.sustained = true;
                                voice.sample_position = 4.0;
                                voice.loop_started = false;
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
                                voice.last_played_note = pattern.note;
                                channel.last_played_note = pattern.note;
                                channel.voice_idx = Some(voice_idx);
                            }
                        }
                    }
                }
            }

            let mut voice_ref = channel.voice_idx.and_then(|idx| {
                if r.voices[idx].channel_idx == i {
                    Some(&mut r.voices[idx])
                } else {
                    None
                }
            });

            // Volume Column (S3M volume range: 0-63, 255 = no volume present)
            if first_tick {
                match pattern.volume {
                    0..=64 => { 
                        channel.set_volume(voice_ref.as_deref_mut(), true, pattern.volume); 
                    }
                    _ => {}
                }
            }

            // Effect Column
            match pattern.effect {
                1 => { // A: Set Speed
                    if first_tick && pattern.effect_param != 0 {
                        *r.speed = pattern.effect_param as u32;
                    }
                }
                2 => { // B: Pattern Jump
                    if first_tick {
                        r.pattern_change.set_jump(true, pattern.effect_param);
                    }
                }
                3 => { // C: Pattern Break
                    if first_tick {
                        r.pattern_change.set_break(r.song_data.song_type, true, pattern.effect_param);
                    }
                }
                4 => { channel.it_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param); }
                5 => { channel.porta_down(r.song_data.song_type, first_tick, pattern.effect_param); }
                6 => { channel.porta_up(r.song_data.song_type, first_tick, pattern.effect_param); }
                7 => { channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), first_tick, pattern.effect_param, r.compatible_g, r.rate, r.frequency_tables); }
                8 => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, pattern.get_x(), pattern.get_y(), r.old_effects, r.rate, r.frequency_tables, r.song_data.song_type); }
                10 => { channel.arpeggio(*r.tick, pattern.get_x(), pattern.get_y(), true); }
                11 => { 
                    channel.vibrato(voice_ref.as_deref_mut(), first_tick, 0, 0, r.old_effects, r.rate, r.frequency_tables, r.song_data.song_type);
                    channel.it_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param);
                }
                12 => { 
                    channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), first_tick, 0, r.compatible_g, r.rate, r.frequency_tables);
                    channel.it_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param);
                }
                13 => { if first_tick { channel.channel_volume = pattern.effect_param.min(64); } }
                14 => { channel.channel_volume_slide(first_tick, pattern.effect_param); }
                15 => { // Sample Offset
                    if first_tick {
                        let mut param = pattern.effect_param as u32;
                        if param == 0 { param = channel.last_sample_offset; }
                        channel.last_sample_offset = param;
                        if let Some(v) = voice_ref.as_deref_mut() {
                            v.sample_position = (param as f32) * 256.0 + 4.0;
                        }
                    }
                }
                16 => { channel.panning_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param, r.song_data.song_type); }
                17 => { channel.it_retrig(voice_ref.as_deref_mut(), instruments, *r.tick, pattern.effect_param); }
                19 => {
                    let x = pattern.get_x();
                    let y = pattern.get_y();
                    match x {
                        0x8 => { if first_tick { if let Some(v) = voice_ref.as_deref_mut() { v.panning.set_panning((y as i32 * 17).min(255)); } } }
                        0xB => { // SBx: Pattern Loop
                            if first_tick {
                                if y == 0 {
                                    channel.loop_row = *r.row as u8;
                                } else {
                                    if channel.loop_count == 0 {
                                        channel.loop_count = y;
                                        r.pattern_change.set_loop(channel.loop_row);
                                    } else {
                                        channel.loop_count -= 1;
                                        if channel.loop_count > 0 {
                                            r.pattern_change.set_loop(channel.loop_row);
                                        }
                                    }
                                }
                            }
                        }
                        0xC => { if *r.tick == y as u32 { channel.on = false; if let Some(v) = voice_ref.as_deref_mut() { v.on = false; } } }
                        0xE => { if first_tick && !r.pattern_change.delay_processed { r.pattern_change.pattern_delay = y as u8; r.pattern_change.delay_processed = true; } }

                        _ => {}
                    }
                }
                20 => { if first_tick { r.bpm.update(pattern.effect_param as u32, r.rate); } }
                22 => { r.global_volume.set_volume(note_delay_first_tick, pattern.effect_param); }
                23 => { r.global_volume.volume_slide(note_delay_first_tick, pattern.effect_param); }
                24 => { if first_tick { if let Some(v) = voice_ref.as_deref_mut() { v.panning.set_panning(pattern.effect_param as i32); } } }
                _ => {}
            }

            if let Some(v) = voice_ref.as_deref_mut() {
                channel.update_frequency_voice(v, r.rate, false, r.frequency_tables);
            }
        }

        // 2. Process all active voices (S3M volume formula)
        let global_vol_f32 = r.global_volume.volume as f32 / 64.0;
        let master_vol_f32 = (r.song_data.master_volume & 127) as f32 / 64.0;
        for (v_idx, voice) in r.voices.iter_mut().enumerate() {
            if !voice.on { continue; }
            let channel = &r.channels[voice.channel_idx];
            let channel_force_off = channel.force_off;
            
            voice.update_envelopes(instruments, r.rate);
            voice.update_fadeout();
            
            // S3M formula: compute_base_volume() * channel_vol/64 * global_vol/64 * master_vol/64
            // compute_base_volume() includes fadeout, envelope, and sample volume
            let channel_vol = channel.channel_volume as f32 / 64.0;
            let output_vol = voice.compute_base_volume() * channel_vol * global_vol_f32 * master_vol_f32;
            voice.set_output_volume(output_vol);
            
            if channel_force_off {
                voice.set_output_volume(0.0);
            }
            
            let is_host_voice = r.channels[voice.channel_idx].voice_idx == Some(v_idx);

            if !voice.sustained && (voice.volume.fadeout_vol == 0 || voice.volume.output_volume < 0.00001) {
                voice.on = false;
            } else if !is_host_voice && voice.volume.output_volume < 0.00001 {
                voice.on = false;
            }
        }
    }
}
