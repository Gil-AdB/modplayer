use crate::module_reader::{SongType, is_note_valid};
use crate::song::backend::{alloc_voice, ModuleBackend, SongPlaybackResources};

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

            if pattern.instrument != 0 {
                channel.last_instrument = if (pattern.instrument as usize) < instruments.len() { pattern.instrument as usize } else { 0 };
            }

            // Note trigger logic
            if is_note_valid(pattern.note, r.song_data.song_type) {
                if pattern.effect == 0x03 || pattern.effect == 0x05 { // Tone Porta
                    if first_tick {
                        let inst_idx = channel.last_instrument;
                        if inst_idx != 0 && !instruments[inst_idx].samples.is_empty() {
                            let sample = &instruments[inst_idx].samples[0];
                            let real_note = (pattern.note as i16 + sample.relative_note as i16).clamp(0, 119) as u8;
                            channel.porta_to_note.target_note.period = channel.note.note_to_period(real_note, sample.finetune, r.frequency_tables);
                        }
                    }
                } else if first_tick {
                    channel.on = true;
                    let inst_idx = channel.last_instrument;
                    if inst_idx != 0 && !instruments[inst_idx].samples.is_empty() {
                        let sample_idx = 0;
                        // MOD always cuts the existing voice on the channel.
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
                        voice.sample = sample_idx;
                        voice.sustained = true;
                        voice.sample_position = 4.0;
                        voice.loop_started = false;
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
                        let real_note = (pattern.note as i16 + sample.relative_note as i16).clamp(0, 119) as u8;
                        channel.note.set_note(real_note, sample.finetune, pattern.note, r.frequency_tables);
                        channel.update_frequency_voice(voice, r.rate, false, r.frequency_tables);
                        voice.last_played_note = pattern.note;
                        channel.voice_idx = Some(voice_idx);
                    }
                }
            } else if pattern.instrument != 0 && first_tick {
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

            let mut voice_ref = channel.voice_idx.and_then(|idx| {
                if r.voices[idx].channel_idx == i {
                    Some(&mut r.voices[idx])
                } else {
                    None
                }
            });

            // Effect Column (MOD effects 0-F)
            match pattern.effect {
                0x00 => { channel.arpeggio(*r.tick, pattern.get_x(), pattern.get_y(), false); }
                0x01 => { channel.porta_up(r.song_data.song_type, first_tick, pattern.effect_param); }
                0x02 => { channel.porta_down(r.song_data.song_type, first_tick, pattern.effect_param); }
                0x03 => { channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), first_tick, pattern.effect_param, r.compatible_g, r.rate, r.frequency_tables); }
                0x04 => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, pattern.get_x(), pattern.get_y(), r.old_effects, r.rate, r.frequency_tables, r.song_data.song_type); }
                0x05 => { 
                    channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), first_tick, 0, r.compatible_g, r.rate, r.frequency_tables);
                    channel.volume_slide_main(voice_ref.as_deref_mut(), first_tick, pattern.effect_param);
                }
                0x06 => { 
                    channel.vibrato(voice_ref.as_deref_mut(), first_tick, 0, 0, r.old_effects, r.rate, r.frequency_tables, r.song_data.song_type);
                    channel.volume_slide_main(voice_ref.as_deref_mut(), first_tick, pattern.effect_param);
                }
                0x07 => { channel.tremolo(voice_ref.as_deref_mut(), first_tick, pattern.get_x(), pattern.get_y(), r.song_data.song_type); }
                0x08 => { if first_tick { if let Some(v) = voice_ref.as_deref_mut() { v.panning.set_panning(pattern.effect_param as i32); } } }
                0x09 => { 
                    if first_tick { 
                        let mut param = pattern.effect_param as u32;
                        if param == 0 { param = channel.last_sample_offset; }
                        channel.last_sample_offset = param;
                        if let Some(v) = voice_ref.as_deref_mut() { v.sample_position = (param as f32) * 256.0 + 4.0; }
                    }
                }
                0x0A => { channel.volume_slide_main(voice_ref.as_deref_mut(), first_tick, pattern.effect_param); }
                0x0B => { if first_tick { r.pattern_change.set_jump(true, pattern.effect_param); } }
                0x0C => { channel.set_volume(voice_ref.as_deref_mut(), first_tick, pattern.effect_param); }
                0x0D => { if first_tick { r.pattern_change.set_break(r.song_data.song_type, true, pattern.effect_param); } }
                0x0E => {
                    let x = pattern.get_x();
                    let y = pattern.get_y();
                    match x {
                        0x1 => { channel.fine_porta_up(r.song_data.song_type, first_tick, y); }
                        0x2 => { channel.fine_porta_down(r.song_data.song_type, first_tick, y); }
                        0x6 => { /* Pattern Loop */ }
                        0x9 => { if first_tick { channel.it_retrig(voice_ref.as_deref_mut(), instruments, *r.tick, y); } }
                        0xA => { if first_tick { channel.fine_volume_slide(voice_ref.as_deref_mut(), true, y as i8); } }
                        0xB => { if first_tick { channel.fine_volume_slide(voice_ref.as_deref_mut(), true, -(y as i8)); } }
                        0xC => { if *r.tick == y as u32 { channel.on = false; if let Some(v) = voice_ref.as_deref_mut() { v.on = false; } } }
                        0xE => { if first_tick && !r.pattern_change.delay_processed { r.pattern_change.pattern_delay = y as u8; r.pattern_change.delay_processed = true; } }
                        _ => {}
                    }
                }
                0x0F => {
                    if first_tick {
                        if pattern.effect_param >= 32 { r.bpm.update(pattern.effect_param as u32, r.rate); }
                        else { *r.speed = pattern.effect_param as u32; }
                    }
                }
                _ => {}
            }

            if let Some(v) = voice_ref.as_deref_mut() {
                channel.update_frequency_voice(v, r.rate, false, r.frequency_tables);
            }
        }

        // 2. Process all active voices (MOD volume formula - no envelope, no inst/sample global vol)
        let global_vol_f32 = 1.0; // MOD has no global volume
        for (v_idx, voice) in r.voices.iter_mut().enumerate() {
            if !voice.on { continue; }
            let _channel_vol_f32 = r.channels[voice.channel_idx].channel_volume as f32 / 64.0;
            let channel_force_off = r.channels[voice.channel_idx].force_off;
            
            voice.update_fadeout();
            
            // MOD formula: fadeout * channel_vol/64 (no envelope, no inst/sample global vol)
            let fadeout = voice.volume.fadeout_vol as f32 / 65536.0;
            let channel_vol = voice.volume.get_volume() as f32 / 64.0;
            let output_vol = fadeout * channel_vol * global_vol_f32;
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
