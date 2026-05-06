use crate::module_reader::SongType;
use crate::pattern::NoteAction;
use crate::song::backend::{
    alloc_voice, apply_extended, set_channel_note, ModuleBackend,
    SongPlaybackResources, XM_E_TABLE,
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
                                
                                // XM cuts the existing voice on the channel before starting a new one.
                                // The match-on-song_type block is dead under XmBackend (always XM/MOD),
                                // and is preserved here for the helper-extraction follow-up.
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
                                voice.panning.panning = r.song_data.initial_channel_panning[i];

                                // XM: a note without instrument keeps the current instrument/envelope phase.
                                // Envelopes reset only when a new instrument is explicitly provided.
                                voice.trigger_note(instruments, pattern.instrument != 0, channel.vibrato_retrig, channel.tremolo_retrig);

                                // XM spec: RealNote = PatternNote + RelativeTone.
                                let sample = &instrument.samples[final_sample_idx];
                                set_channel_note(channel, voice, sample.relative_note, sample.finetune, pattern.note, r.rate, r.frequency_tables);
                                voice.last_played_note = pattern.note;
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
                        r.voices[v_idx].volume.output_volume = 0.0;
                    }
                }
            }
            // XM doesn't support note 122 (fade) or note > 96 - both fall through.
            NoteAction::Fade | NoteAction::None => {}
            }

            let mut voice_ref = channel.voice_idx.and_then(|idx| {
                if r.voices[idx].channel_idx == i {
                    Some(&mut r.voices[idx])
                } else {
                    None
                }
            });

            // Volume Column
            match pattern.volume {
                0x10..=0x50 => { channel.set_volume(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.volume - 0x10); }
                0x60..=0x6f => { channel.volume_slide(voice_ref.as_deref_mut(), false, -(pattern.get_volume_param() as i8)); }
                0x70..=0x7f => { channel.volume_slide(voice_ref.as_deref_mut(), false, pattern.get_volume_param() as i8); }
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

            // Effect Column
            match pattern.effect {
                0x0 => { // Arpeggio
                    if pattern.effect_param != 0 {
                        channel.arpeggio(*r.tick, pattern.get_x(), pattern.get_y(), false);
                    } else {
                        channel.period_shift = 0;
                    }
                }
                0x1 => { channel.period_shift = 0; channel.porta_up(r.song_data.song_type, first_tick, pattern.effect_param); }
                0x2 => { channel.period_shift = 0; channel.porta_down(r.song_data.song_type, first_tick, pattern.effect_param); }
                0x3 => { channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), first_tick, pattern.effect_param, r.compatible_g, r.rate, r.frequency_tables); }
                0x4 => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, pattern.get_x(), pattern.get_y(), r.old_effects, r.rate, r.frequency_tables, r.song_data.song_type); }
                0x5 => { 
                    channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), first_tick, 0, r.compatible_g, r.rate, r.frequency_tables); 
                    channel.volume_slide_main(voice_ref.as_deref_mut(), first_tick, pattern.effect_param); 
                }
                0x6 => { 
                    channel.vibrato(voice_ref.as_deref_mut(), first_tick, 0, 0, r.old_effects, r.rate, r.frequency_tables, r.song_data.song_type); 
                    channel.volume_slide_main(voice_ref.as_deref_mut(), first_tick, pattern.effect_param); 
                }
                0x7 => { channel.tremolo(voice_ref.as_deref_mut(), first_tick, pattern.get_x(), pattern.get_y(), r.song_data.song_type); }
                0x8 => { if first_tick { if let Some(v) = voice_ref.as_deref_mut() { v.panning.set_panning(pattern.effect_param as i32); } } }
                0x9 => { // Sample Offset
                    if first_tick {
                        let param = channel.recall_or_set(crate::channel_state::EffectMemorySlot::SampleOffset, pattern.effect_param);
                        if let Some(v) = voice_ref.as_deref_mut() {
                            v.sample_position = (param as f32) * 256.0 + 4.0;
                        }
                    }
                }
                0xA => { channel.volume_slide_main(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param); }
                0xB => { r.pattern_change.set_jump(first_tick, pattern.effect_param); }
                0xC => { if first_tick { channel.set_volume(voice_ref.as_deref_mut(), true, pattern.effect_param); } }
                0xD => { r.pattern_change.set_break(r.song_data.song_type, first_tick, pattern.effect_param); }
                0xE => {
                    let kind = XM_E_TABLE[pattern.get_x() as usize];
                    apply_extended(
                        kind, channel, voice_ref.as_deref_mut(),
                        r.pattern_change, instruments,
                        *r.tick, *r.row, first_tick, first_row_tick,
                        r.song_data.song_type, r.rate, r.frequency_tables,
                        pattern.get_y(),
                    );
                }
                0x0F => {
                    if first_tick {
                        if pattern.effect_param >= 32 { r.bpm.update(pattern.effect_param as u32, r.rate); }
                        else { *r.speed = pattern.effect_param as u32; }
                    }
                }
                0x14 => { // Kxx: Key Off at tick xx
                    if *r.tick == pattern.effect_param as u32 {
                        if let Some(v) = voice_ref.as_deref_mut() {
                            v.key_off(instruments, false);
                        }
                    }
                }
                0x15 => { // Lxx: Set Envelope Position
                    if first_tick {
                        if let Some(v) = voice_ref.as_deref_mut() {
                            let inst = &instruments[v.instrument];
                            v.volume_envelope_state.set_position(&inst.volume_envelope, pattern.effect_param);
                            v.panning_envelope_state.set_position(&inst.panning_envelope, pattern.effect_param);
                            v.pitch_envelope_state.set_position(&inst.pitch_envelope, pattern.effect_param);
                        }
                    }
                }
                0x10 => { // Gxx: Set Global Volume
                    r.global_volume.set_volume(first_tick, pattern.effect_param);
                }
                0x11 => { // Hxy: Global Volume Slide
                    r.global_volume.volume_slide(first_tick, pattern.effect_param);
                }
                0x19 => { // Pxy: Panning Slide
                    channel.panning_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param, r.song_data.song_type);
                }
                0x1B => { // Rxy: Multi Retrig Note + Volume Slide
                    channel.retrig(voice_ref.as_deref_mut(), instruments, *r.tick, pattern.get_y(), pattern.get_x());
                }
                _ => {}
            }

            if let Some(v) = voice_ref.as_deref_mut() {
                // If effect is not Arpeggio, reset period_shift
                if pattern.effect != 0 {
                    channel.period_shift = 0;
                }
                channel.update_frequency_voice(v, r.rate, false, r.frequency_tables);
            }
        }

        // 2. Process all active voices (XM volume formula)
        let global_vol_f32 = r.global_volume.volume as f32 / 64.0;
        for (v_idx, voice) in r.voices.iter_mut().enumerate() {
            if !voice.on { continue; }
            let channel_force_off = r.channels[voice.channel_idx].force_off;
        
        voice.update_envelopes(instruments, r.rate);
        voice.update_fadeout();
            
            // XM formula: fadeout * envelope * channel_vol/64 * global_vol/64
            let base = voice.compute_base_volume();
            let output_vol = base * global_vol_f32;
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
