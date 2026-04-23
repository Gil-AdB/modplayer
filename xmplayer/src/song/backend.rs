use crate::song::{GlobalVolume, BPM, PatternChange, Song};
use crate::module_reader::{SongData, SongType, is_note_valid};
use crate::channel_state::{ChannelState, Voice};
use crate::channel_state::channel_state::clamp;
use crate::tables::AudioTables;
use crate::instrument::Instrument;
use std::borrow::Borrow;

pub struct SongPlaybackResources<'a> {
    pub song_position:              &'a mut usize,
    pub row:                        &'a mut usize,
    pub tick:                       &'a mut u32,
    pub speed:                      &'a mut u32,
    pub global_volume:              &'a mut GlobalVolume,
    pub song_data:                  &'a SongData,
    pub channels:                   &'a mut [ChannelState],
    pub voices:                     &'a mut [Voice],
    pub pattern_change:             &'a mut PatternChange,
    pub row_delay:                  &'a mut usize,
    pub bpm:                        &'a mut BPM,
    pub frequency_tables:           &'a AudioTables,
    pub rate:                       f32,
    pub old_effects:                bool,
    pub compatible_g:               bool,
}

pub trait ModuleBackend: Send {
    fn process_tick(&mut self, resources: &mut SongPlaybackResources);
}

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

        // 1. Process channels (Note trigger and Effects)
        for i in 0..r.channels.len() {
            let channel = &mut r.channels[i];

            // Lazy cleanup: if the voice we were tracking was stolen by another channel, detach it now.
            if let Some(v_idx) = channel.voice_idx {
                if r.voices[v_idx].channel_idx != i {
                    channel.voice_idx = None;
                }
            }

            let patterns = &r.song_data.patterns[r.song_data.pattern_order[*r.song_position] as usize];
            let row = &patterns.rows[*r.row];
            let pattern = &row.channels[i];

            let note_delay_first_tick = if pattern.is_note_delay(r.song_data.song_type) { *r.tick == pattern.get_y() as u32 } else {first_tick};

            if pattern.is_porta_to_note(r.song_data.song_type) && first_tick && is_note_valid(pattern.note, r.song_data.song_type) {
                let note = pattern.note;
                let mut inst_idx = channel.last_instrument;
                if pattern.instrument != 0 {
                    inst_idx = if (pattern.instrument as usize) < instruments.len() { pattern.instrument as usize } else { 0 };
                }
                
                let mut final_sample_idx = 0;
                let mut mapped_note = note;
                
                if inst_idx != 0 && (note as usize - 1) < instruments[inst_idx].sample_indexes.len() {
                    let it_mapping = instruments[inst_idx].sample_indexes[note as usize - 1];
                    mapped_note = it_mapping.0 + 1;
                    let sample_idx = it_mapping.1 as usize;
                    if sample_idx > 0 {
                        final_sample_idx = sample_idx - 1;
                    }
                }
                
                if inst_idx != 0 && final_sample_idx < instruments[inst_idx].samples.len() {
                    let sample = &instruments[inst_idx].samples[final_sample_idx];
                    let real_note = clamp(mapped_note as i16 + sample.relative_note as i16, 0, 119) as u8;
                    channel.porta_to_note.target_note.period = channel.note.note_to_period(real_note, sample.finetune, r.frequency_tables);
                } else {
                    channel.porta_to_note.target_note.period = channel.note.note_to_period(pattern.note, 0, r.frequency_tables);
                }
            }

            if !pattern.is_porta_to_note(r.song_data.song_type) &&
                ((pattern.is_note_delay(r.song_data.song_type) && *r.tick == pattern.get_y() as u32) ||
                    (!pattern.is_note_delay(r.song_data.song_type) && first_tick)) {
                
                let note = pattern.note;
                let mut inst_idx = channel.last_instrument;
                if pattern.instrument != 0 {
                    inst_idx = if (pattern.instrument as usize) < instruments.len() { pattern.instrument as usize } else { 0 };
                    channel.last_instrument = inst_idx;
                }

                if is_note_valid(note, r.song_data.song_type) {
                    // IT Duplicate Check (DCT/DCA)
                    if inst_idx != 0 {
                        let new_inst = &instruments[inst_idx];
                        let mut dca_applied = false;

                        // Check all active voices for duplicates on this host channel
                        for vi in 0..r.voices.len() {
                            let v = &mut r.voices[vi];
                            if !v.on || v.channel_idx != i { continue; }

                            match new_inst.dct {
                                1 => { // Note match
                                    if v.last_played_note == note {
                                        Self::apply_it_action(r.voices, vi, new_inst.dca, new_inst);
                                        dca_applied = true;
                                    }
                                }
                                2 => { // Sample match
                                    // Find sample index for new note
                                    let sample_idx = if (note as usize - 1) < new_inst.sample_indexes.len() {
                                        new_inst.sample_indexes[note as usize - 1].1
                                    } else { 0 };
                                    
                                    if sample_idx > 0 && v.sample == (sample_idx - 1) as usize && v.instrument == inst_idx {
                                        Self::apply_it_action(r.voices, vi, new_inst.dca, new_inst);
                                        dca_applied = true;
                                    }
                                }
                                3 => { // Instrument match
                                    if v.instrument == inst_idx {
                                        Self::apply_it_action(r.voices, vi, new_inst.dca, new_inst);
                                        dca_applied = true;
                                    }
                                }
                                _ => {}
                            }
                        }

                        // If no DCT matched, apply NNA to current voice if it exists
                        if !dca_applied {
                            if let Some(v_idx) = channel.voice_idx {
                                if r.voices[v_idx].on {
                                    Self::apply_it_action(r.voices, v_idx, new_inst.nna, new_inst);
                                }
                            }
                        }
                    }

                    // Start a new voice
                    let note_idx = (note - 1) as usize;
                    let mut trigger_voice = false;
                    let mut final_sample_idx = 0;
                    let mut mapped_note = note;

                    if inst_idx != 0 && note_idx < instruments[inst_idx].sample_indexes.len() {
                        let it_mapping = instruments[inst_idx].sample_indexes[note_idx];
                        let sample_idx = it_mapping.1 as usize;
                        mapped_note = it_mapping.0 + 1; // IT notes are 0..119, Pattern notes are 1..120
                        if sample_idx > 0 {
                            final_sample_idx = sample_idx - 1;
                            if final_sample_idx < instruments[inst_idx].samples.len() {
                                trigger_voice = true;
                            }
                        }
                    }

                    if trigger_voice {
                        // Find free voice or steal quietest
                        let mut v_idx = 0;
                        let mut found = false;
                        for vi in 0..r.voices.len() {
                            if !r.voices[vi].on { v_idx = vi; found = true; break; }
                        }
                        if !found {
                            let mut min_vol = 1_000_000.0f32;
                            for vi in 0..r.voices.len() {
                                if r.voices[vi].volume.output_volume < min_vol {
                                    min_vol = r.voices[vi].volume.output_volume;
                                    v_idx = vi;
                                }
                            }
                        }
                        
                        let mut clone_voice = None;
                        if pattern.instrument == 0 {
                            if let Some(old_idx) = channel.voice_idx {
                                clone_voice = Some(r.voices[old_idx].clone());
                            }
                        }
                        
                        let voice = &mut r.voices[v_idx];
                        voice.on = true;
                        voice.channel_idx = i;
                        voice.instrument = inst_idx;
                        voice.sample = final_sample_idx;
                        voice.last_played_note = mapped_note;
                        
                        if let Some(old_voice) = clone_voice {
                            voice.volume = old_voice.volume;
                            voice.panning = old_voice.panning;
                            voice.volume_envelope_state = old_voice.volume_envelope_state;
                            voice.panning_envelope_state = old_voice.panning_envelope_state;
                            voice.pitch_envelope_state = old_voice.pitch_envelope_state;
                            voice.vibrato_state = old_voice.vibrato_state;
                            voice.tremolo_state = old_voice.tremolo_state;
                            voice.instrument_global_volume = instruments[inst_idx].global_volume;
                            voice.sample_global_volume = old_voice.sample_global_volume;
                            voice.sample_position = 4.0;
                            voice.loop_started = false;
                            voice.ping = true;
                        } else {
                            voice.trigger_note(instruments);
                            let sample = &instruments[inst_idx].samples[final_sample_idx];
                            voice.volume.retrig(sample.volume as i32);
                            voice.panning.panning = sample.panning;
                        }

                        channel.voice_idx = Some(v_idx);
                        channel.last_played_note = note;
                        channel.on = true;
                        
                        // We need to borrow voice again because trigger_note might have changed it
                        let voice = &mut r.voices[v_idx];
                        let sample = &instruments[inst_idx].samples[final_sample_idx];
                        voice.surround = sample.surround;
                        let real_note = (mapped_note as i16 + sample.relative_note as i16) as u8;
                        channel.note.set_note(real_note, sample.finetune, mapped_note, r.frequency_tables);
                        channel.update_frequency_voice(voice, r.rate, false, r.frequency_tables);
                    }
                }

                if note == 97 { // Note Off
                    if let Some(v_idx) = channel.voice_idx {
                        r.voices[v_idx].key_off(&instruments, pattern.is_note_delay(r.song_data.song_type));
                    }
                } else if note == 121 { // Note Cut
                    if let Some(v_idx) = channel.voice_idx {
                        r.voices[v_idx].on = false;
                        r.voices[v_idx].volume.output_volume = 0.0;
                    }
                } else if note == 122 { // Note Fade
                    if let Some(v_idx) = channel.voice_idx {
                        r.voices[v_idx].sustained = false;
                        let instrument_nna = &instruments[r.voices[v_idx].instrument];
                        r.voices[v_idx].volume.fadeout_speed = (instrument_nna.volume_fadeout as i32) << 6;
                    }
                }
            }

            // Handle effects (even if there is no active voice, global effects and volume slides apply to channel state)
            let mut voice_ref = channel.voice_idx.and_then(|idx| {
                if r.voices[idx].channel_idx == i {
                    Some(&mut r.voices[idx])
                } else {
                    None
                }
            });
                
            if !first_tick && (pattern.has_vibrato(r.song_data.song_type) || pattern.effect == 0x4 || pattern.effect == 0x6) {
                channel.vibrato(voice_ref.as_deref_mut(), first_tick, pattern.get_vibrato_speed(), pattern.get_vibrato_depth(), r.old_effects, r.frequency_tables);
            }

            match pattern.volume {
                0..=64 => { channel.set_volume(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.volume); }
                65..=74 => { channel.fine_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, (pattern.volume - 65) as i8); }
                75..=84 => { channel.fine_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, -((pattern.volume - 75) as i8)); }
                85..=94 => { channel.volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, (pattern.volume - 85) as i8); }
                95..=104 => { channel.volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, -((pattern.volume - 95) as i8)); }
                105..=114 => { channel.porta_up(r.song_data.song_type, first_tick, (pattern.volume - 105) << 2); }
                115..=124 => { channel.porta_down(r.song_data.song_type, first_tick, (pattern.volume - 115) << 2); }
                128..=192 => { if let Some(v) = voice_ref.as_deref_mut() { v.panning.set_panning(((pattern.volume - 128) << 2) as i32); } } // Panning
                193..=202 => { channel.porta_up(r.song_data.song_type, first_tick, pattern.volume - 192); } // Portamento Up
                203..=212 => { channel.porta_down(r.song_data.song_type, first_tick, pattern.volume - 202); } // Portamento Down
                _ => {}
            }

            match pattern.effect {
                0x01 => { if first_tick { *r.speed = pattern.effect_param as u32; } } // A: Set Speed
                0x02 => { r.pattern_change.set_jump(first_tick, pattern.effect_param); } // B: Pattern Jump
                0x03 => { r.pattern_change.set_break(r.song_data.song_type, first_tick, pattern.effect_param); } // C: Pattern Break
                0x04 => { channel.it_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param); } // D: Volume Slide
                0x05 => { // E: Porta Down
                    let param = if !r.compatible_g && pattern.effect_param == 0 { channel.last_it_slide_speed } else { pattern.effect_param };
                    if !r.compatible_g && pattern.effect_param != 0 { channel.last_it_slide_speed = pattern.effect_param; }
                    channel.porta_down(r.song_data.song_type, first_tick, param); 
                }
                0x06 => { // F: Porta Up
                    let param = if !r.compatible_g && pattern.effect_param == 0 { channel.last_it_slide_speed } else { pattern.effect_param };
                    if !r.compatible_g && pattern.effect_param != 0 { channel.last_it_slide_speed = pattern.effect_param; }
                    channel.porta_up(r.song_data.song_type, first_tick, param); 
                }
                0x07 => { // G: Porta Note
                    let param = if !r.compatible_g && pattern.effect_param == 0 { channel.last_it_slide_speed } else { pattern.effect_param };
                    if !r.compatible_g && pattern.effect_param != 0 { channel.last_it_slide_speed = pattern.effect_param; }
                    channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), first_tick, param, r.compatible_g, r.frequency_tables); 
                }
                0x08 => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, pattern.get_x(), pattern.get_y(), r.old_effects, r.frequency_tables); } // H: Vibrato
                0x0A => { channel.arpeggio(*r.tick, pattern.get_x(), pattern.get_y()); } // J: Arpeggio
                0x0B => { // K: Vibrato + Volume Slide
                    channel.vibrato(voice_ref.as_deref_mut(), first_tick, 0, 0, r.old_effects, r.frequency_tables);
                    channel.it_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param);
                }
                0x0C => { // L: Porta Note + Volume Slide
                    channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), first_tick, 0, r.compatible_g, r.frequency_tables);
                    channel.it_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param);
                }
                0x0F => { /* O: Offset - to be implemented */ }
                0x11 => { channel.it_retrig(voice_ref.as_deref_mut(), &r.song_data.instruments, *r.tick, pattern.effect_param); } // Q: Multi-Retrig
                0x14 => { if first_tick { r.bpm.update(pattern.effect_param as u32, r.rate); } } // T: Set Tempo
                0x16 => { r.global_volume.set_volume(note_delay_first_tick, pattern.effect_param); } // V: Set Global Vol
                0x17 => { r.global_volume.volume_slide(note_delay_first_tick, pattern.effect_param); } // W: Global Volume Slide
                0x18 => { if first_tick { if let Some(v) = voice_ref.as_deref_mut() { v.panning.set_panning((pattern.effect_param as i32 * 4).min(255)); } } } // X: Set Panning
                0x13 => { // S: Special
                    let x = pattern.get_x();
                    match x {
                        0x08 => { if first_tick { if let Some(v) = voice_ref.as_deref_mut() { v.panning.set_panning((pattern.get_y() << 4) as i32); } } } // S8x: Set Panning
                        0x0C => { if *r.tick == pattern.get_y() as u32 { channel.on = false; if let Some(v) = voice_ref.as_deref_mut() { v.on = false; } } } // SCx: Note Cut
                        0x0D => { /* SDx: Note Delay - already handled by note_delay_first_tick logic */ }
                        0x0E => { if first_tick { *r.row_delay = pattern.get_y() as usize; } } // SEx: Pattern Row Delay
                        _ => {}
                    }
                }
                0x0D => { if first_tick { channel.channel_volume = pattern.effect_param.min(64); } } // M: Channel Volume
                0x0E => { channel.channel_volume_slide(note_delay_first_tick, pattern.effect_param); } // N: Channel Volume Slide
                0x10 => { channel.panning_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param); } // P: Panning Slide
                0x1A => { // Z: Resonant Filter
                    if let Some(v) = voice_ref.as_deref_mut() {
                        if pattern.effect_param < 0x80 {
                            v.filter_cutoff = pattern.effect_param;
                        } else if (0x80..=0x8F).contains(&pattern.effect_param) {
                            v.filter_resonance = (pattern.effect_param & 0x0F) << 3; // Map 0-15 to 0-120ish
                        }
                    }
                }
                _ => {}
            }
                
            if let Some(v) = voice_ref.as_deref_mut() {
                channel.update_frequency_voice(v, r.rate, false, r.frequency_tables);
            }
        }

        // 2. Process all active voices (Envelopes and Final Volume)
        let divisor = 128.0;
        let global_vol_f32 = r.global_volume.volume as f32 / divisor;
        for (v_idx, voice) in r.voices.iter_mut().enumerate() {
            if !voice.on { continue; }
            let channel_vol_f32 = r.channels[voice.channel_idx].channel_volume as f32 / 64.0;
            voice.update_envelopes(instruments, r.rate);
            voice.update_output_volume(global_vol_f32, channel_vol_f32, divisor);
            
            let is_host_voice = r.channels[voice.channel_idx].voice_idx == Some(v_idx);
            
            if !voice.sustained && (voice.volume.fadeout_vol == 0 || voice.volume.output_volume < 0.00001) {
                voice.on = false;
            } else if !is_host_voice && voice.volume.output_volume < 0.00001 {
                voice.on = false;
            }
        }
    }
}

pub struct XmBackend {}

impl XmBackend {
    pub fn new() -> Self {
        Self {}
    }
}

impl ModuleBackend for XmBackend {
    fn process_tick(&mut self, r: &mut SongPlaybackResources) {
        let first_tick = *r.tick == 0;
        let instruments = &r.song_data.instruments;

        // 1. Process channels (Note trigger and Effects)
        for i in 0..r.channels.len() {
            let channel = &mut r.channels[i];

            if let Some(v_idx) = channel.voice_idx {
                if r.voices[v_idx].channel_idx != i {
                    channel.voice_idx = None;
                }
            }

            let patterns = &r.song_data.patterns[r.song_data.pattern_order[*r.song_position] as usize];
            let row = &patterns.rows[*r.row];
            let pattern = &row.channels[i];

            let note_delay_first_tick = first_tick;

            if pattern.is_porta_to_note(r.song_data.song_type) && first_tick && is_note_valid(pattern.note, r.song_data.song_type) {
                let note = pattern.note;
                let mut inst_idx = channel.last_instrument;
                if pattern.instrument != 0 {
                    inst_idx = if (pattern.instrument as usize) < instruments.len() { pattern.instrument as usize } else { 0 };
                }
                
                let mut final_sample_idx = 0;
                
                if inst_idx != 0 && (note as usize - 1) < instruments[inst_idx].sample_indexes.len() {
                    let it_mapping = instruments[inst_idx].sample_indexes[note as usize - 1];
                    final_sample_idx = it_mapping.1 as usize;
                }
                
                if inst_idx != 0 && final_sample_idx < instruments[inst_idx].samples.len() {
                    let sample = &instruments[inst_idx].samples[final_sample_idx];
                    let real_note = clamp(note as i16 + sample.relative_note as i16, 0, 119) as u8;
                    channel.porta_to_note.target_note.period = channel.note.note_to_period(real_note, sample.finetune, r.frequency_tables);
                } else {
                    channel.porta_to_note.target_note.period = channel.note.note_to_period(pattern.note, 0, r.frequency_tables);
                }
            }

            if !pattern.is_porta_to_note(r.song_data.song_type) && first_tick {
                
                let note = pattern.note;
                let mut inst_idx = channel.last_instrument;
                if pattern.instrument != 0 {
                    inst_idx = if (pattern.instrument as usize) < instruments.len() { pattern.instrument as usize } else { 0 };
                    channel.last_instrument = inst_idx;
                }

                if is_note_valid(note, r.song_data.song_type) {
                    if let Some(old_v_idx) = channel.voice_idx {
                        let instrument_nna = &instruments[r.voices[old_v_idx].instrument];
                        match instrument_nna.nna {
                            0 => { r.voices[old_v_idx].on = false; }
                            1 => { r.voices[old_v_idx].key_off(instruments, false); }
                            2 => {
                                r.voices[old_v_idx].sustained = false;
                                r.voices[old_v_idx].volume.fadeout_speed = (instrument_nna.volume_fadeout as i32) << 6;
                            }
                            _ => { r.voices[old_v_idx].key_off(instruments, false); }
                        }
                    }

                    // Start a new voice
                    let mut trigger_voice = false;
                    let mut final_sample_idx = 0;
                    let mapped_note = note;

                    if inst_idx != 0 && (note as usize - 1) < instruments[inst_idx].sample_indexes.len() {
                        let it_mapping = instruments[inst_idx].sample_indexes[note as usize - 1];
                        final_sample_idx = it_mapping.1 as usize;
                        if final_sample_idx < instruments[inst_idx].samples.len() {
                            trigger_voice = true;
                        }
                    }

                    if trigger_voice {
                        let mut v_idx = 0;
                        let mut found = false;
                        for vi in 0..r.voices.len() {
                            if !r.voices[vi].on { v_idx = vi; found = true; break; }
                        }
                        if !found {
                            let mut min_vol = 1_000_000.0f32;
                            for vi in 0..r.voices.len() {
                                if r.voices[vi].volume.output_volume < min_vol {
                                    min_vol = r.voices[vi].volume.output_volume;
                                    v_idx = vi;
                                }
                            }
                        }
                        
                        let mut clone_voice = None;
                        if pattern.instrument == 0 {
                            if let Some(old_idx) = channel.voice_idx {
                                clone_voice = Some(r.voices[old_idx].clone());
                            }
                        }
                        
                        let voice = &mut r.voices[v_idx];
                        voice.on = true;
                        voice.channel_idx = i;
                        voice.instrument = inst_idx;
                        voice.sample = final_sample_idx;
                        voice.last_played_note = mapped_note;
                        
                        if let Some(old_voice) = clone_voice {
                            voice.volume = old_voice.volume;
                            voice.panning = old_voice.panning;
                            voice.volume_envelope_state = old_voice.volume_envelope_state;
                            voice.panning_envelope_state = old_voice.panning_envelope_state;
                            voice.pitch_envelope_state = old_voice.pitch_envelope_state;
                            voice.vibrato_state = old_voice.vibrato_state;
                            voice.tremolo_state = old_voice.tremolo_state;
                            voice.instrument_global_volume = instruments[inst_idx].global_volume;
                            voice.sample_global_volume = old_voice.sample_global_volume;
                            voice.sample_position = 4.0;
                            voice.loop_started = false;
                            voice.ping = true;
                        } else {
                            voice.trigger_note(instruments);
                            let sample = &instruments[inst_idx].samples[final_sample_idx];
                            voice.volume.retrig(sample.volume as i32);
                            voice.panning.panning = sample.panning;
                        }

                        channel.voice_idx = Some(v_idx);
                        channel.last_played_note = note;
                        channel.on = true;
                        
                        let voice = &mut r.voices[v_idx];
                        let sample = &instruments[inst_idx].samples[final_sample_idx];
                        voice.surround = sample.surround;
                        let real_note = (mapped_note as i16 + sample.relative_note as i16) as u8;
                        channel.note.set_note(real_note, sample.finetune, mapped_note, r.frequency_tables);
                        channel.update_frequency_voice(voice, r.rate, false, r.frequency_tables);
                    }
                }

                if note == 97 { // Note Off
                    if let Some(v_idx) = channel.voice_idx {
                        r.voices[v_idx].key_off(&instruments, false);
                    }
                } else if note == 121 { // Note Cut
                    if let Some(v_idx) = channel.voice_idx {
                        r.voices[v_idx].on = false;
                        r.voices[v_idx].volume.output_volume = 0.0;
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
                
            if !first_tick && (pattern.effect == 0x4 || pattern.effect == 0x6) {
                channel.vibrato(voice_ref.as_deref_mut(), first_tick, pattern.get_vibrato_speed(), pattern.get_vibrato_depth(), r.old_effects, r.frequency_tables);
            }

            match pattern.volume {
                0x10..=0x50 => { channel.set_volume(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.volume - 0x10); }
                0x60..=0x6f => { channel.volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, -(pattern.get_volume_param() as i8)); }
                0x70..=0x7f => { channel.volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.get_volume_param() as i8); }
                0x80..=0x8f => { channel.fine_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, -(pattern.get_volume_param() as i8)); }
                0x90..=0x9f => { channel.fine_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.get_volume_param() as i8); }
                0xa0..=0xaf => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, 0, pattern.get_volume_param(), r.old_effects, r.frequency_tables); }
                0xb0..=0xbf => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, pattern.get_volume_param(), 0, r.old_effects, r.frequency_tables); }
                0xd0..=0xdf => { channel.panning_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.get_volume_param() << 4); }
                0xe0..=0xef => { channel.panning_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.get_volume_param()); }
                0xf0..=0xff => { channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), note_delay_first_tick, pattern.get_volume_param(), r.compatible_g, r.frequency_tables); }
                _ => {}
            }

            match pattern.effect {
                0x1 => { channel.porta_up(r.song_data.song_type, first_tick, pattern.effect_param); }
                0x2 => { channel.porta_down(r.song_data.song_type, first_tick, pattern.effect_param); }
                0x3 => { channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), first_tick, pattern.effect_param, r.compatible_g, r.frequency_tables); }
                0x4 => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, pattern.get_x(), pattern.get_y(), r.old_effects, r.frequency_tables); }
                0x5 => { channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), first_tick, 0, r.compatible_g, r.frequency_tables); channel.volume_slide_main(voice_ref.as_deref_mut(), first_tick, pattern.effect_param); }
                0x6 => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, 0, 0, r.old_effects, r.frequency_tables); channel.volume_slide_main(voice_ref.as_deref_mut(), first_tick, pattern.effect_param); }
                0x7 => { channel.tremolo(voice_ref.as_deref_mut(), first_tick, pattern.get_x(), pattern.get_y()); }
                0xA => { channel.volume_slide_main(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param); }
                0xB => { r.pattern_change.set_jump(first_tick, pattern.effect_param); } // B: Pattern Jump
                0xD => { r.pattern_change.set_break(r.song_data.song_type, first_tick, pattern.effect_param); } // D: Pattern Break
                0x8 => { if let Some(v) = voice_ref.as_deref_mut() { v.panning.set_panning(pattern.effect_param as i32); } }
                0x0F => { // Set Speed / BPM (Fxx)
                    if first_tick {
                        if pattern.effect_param < 32 {
                            *r.speed = pattern.effect_param as u32;
                        } else {
                            r.bpm.update(pattern.effect_param as u32, r.rate);
                        }
                    }
                }
                0x10 => { r.global_volume.set_volume(note_delay_first_tick, pattern.effect_param); }
                0x11 => { r.global_volume.volume_slide(first_tick, pattern.effect_param); }
                0x14 => { if first_tick { r.bpm.update(pattern.effect_param as u32, r.rate); } } // T: Set Tempo
                0x16 => { r.global_volume.set_volume(note_delay_first_tick, pattern.effect_param); } // V: Set Global Vol
                0x17 => { r.global_volume.volume_slide(note_delay_first_tick, pattern.effect_param); } // W: Global Volume Slide
                0x18 => { if first_tick { if let Some(v) = voice_ref.as_deref_mut() { v.panning.set_panning((pattern.effect_param as i32 * 4).min(255)); } } } // X: Set Panning
                0x1D => { channel.tremor(*r.tick, pattern.effect_param); } // I: Tremor
                0x1E => { channel.it_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param); } // D: Volume Slide (S3M/IT style)
                0x1F => { // K: Vibrato + Volume Slide (S3M/IT style)
                    channel.vibrato(voice_ref.as_deref_mut(), first_tick, 0, 0, r.old_effects, r.frequency_tables);
                    channel.it_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param);
                }
                0x20 => { // L: Porta Note + Volume Slide (S3M/IT style)
                    channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), first_tick, 0, r.compatible_g, r.frequency_tables);
                    channel.it_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param);
                }
                0x21 => { channel.it_retrig(voice_ref.as_deref_mut(), &r.song_data.instruments, *r.tick, pattern.effect_param); } // Q: Multi Retrig (S3M/IT style)
                0xE => {
                    let subcommand = pattern.get_x();
                    let param = pattern.get_y();
                    match subcommand {
                        0x1 => { channel.fine_porta_up(r.song_data.song_type, first_tick, param); }
                        0x2 => { channel.fine_porta_down(r.song_data.song_type, first_tick, param); }
                        0xA => { channel.fine_volume_slide(voice_ref.as_deref_mut(), first_tick, param as i8); }
                        0xB => { channel.fine_volume_slide(voice_ref.as_deref_mut(), first_tick, -(param as i8)); }
                        0xC => { if *r.tick == param as u32 { if let Some(v) = voice_ref.as_deref_mut() { v.on = false; } } }
                        _ => {}
                    }
                }
                _ => {}
            }
                
            if let Some(v) = voice_ref.as_deref_mut() {
                channel.update_frequency_voice(v, r.rate, false, r.frequency_tables);
            }
        }

        // 2. Process all active voices (Envelopes and Final Volume)
        let divisor = 64.0;
        let global_vol_f32 = r.global_volume.volume as f32 / divisor;
        for (v_idx, voice) in r.voices.iter_mut().enumerate() {
            if !voice.on { continue; }
            let channel_vol_f32 = r.channels[voice.channel_idx].channel_volume as f32 / 64.0;
            voice.update_envelopes(instruments, r.rate);
            voice.update_output_volume(global_vol_f32, channel_vol_f32, divisor);
            
            let is_host_voice = r.channels[voice.channel_idx].voice_idx == Some(v_idx);
            
            if !voice.sustained && (voice.volume.fadeout_vol == 0 || voice.volume.output_volume < 0.00001) {
                voice.on = false;
            } else if !is_host_voice && voice.volume.output_volume < 0.00001 {
                voice.on = false;
            }
        }
    }
}

pub struct S3MModBackend {}

impl S3MModBackend {
    pub fn new() -> Self {
        Self {}
    }
}

impl ModuleBackend for S3MModBackend {
    fn process_tick(&mut self, r: &mut SongPlaybackResources) {
        // For now, S3M and MOD share logic with XM as per previous monolithic implementation
        // but can be specialized later.
        let mut xm = XmBackend::new();
        xm.process_tick(r);
    }
}
