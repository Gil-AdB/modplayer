use crate::song::{GlobalVolume, BPM, PatternChange};
use crate::module_reader::{SongData, is_note_valid};
use crate::channel_state::{ChannelState, Voice};
use crate::tables::AudioTables;
use crate::instrument::Instrument;

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

        // 1. Process all channels
        for (i, channel) in r.channels.iter_mut().enumerate() {
            let patterns = &r.song_data.patterns[r.song_data.pattern_order[*r.song_position] as usize];
            let row = &patterns.rows[*r.row];
            let pattern = &row.channels[i];
            
            let is_note_delay = pattern.is_note_delay(r.song_data.song_type);
            let note_delay_first_tick = if is_note_delay { *r.tick == pattern.get_y() as u32 } else { first_tick };

            if pattern.instrument != 0 {
                channel.last_instrument = if (pattern.instrument as usize) < instruments.len() { pattern.instrument as usize } else { 0 };
            }

            if is_note_valid(pattern.note, r.song_data.song_type) {
                if pattern.is_porta_to_note(r.song_data.song_type) {
                    if first_tick {
                        let inst_idx = channel.last_instrument;
                        if inst_idx != 0 && (pattern.note as usize - 1) < instruments[inst_idx].sample_indexes.len() {
                            let it_mapping = instruments[inst_idx].sample_indexes[pattern.note as usize - 1];
                            let sample_idx = it_mapping.1 as usize;
                            if sample_idx > 0 && (sample_idx - 1) < instruments[inst_idx].samples.len() {
                                let sample = &instruments[inst_idx].samples[sample_idx - 1];
                                let real_note = (it_mapping.0 as i16 + sample.relative_note as i16).clamp(0, 119) as u8;
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

                                // Find a free voice
                                let mut voice_idx = 0;
                                let mut found = false;
                                for vi in 0..r.voices.len() {
                                    if !r.voices[vi].on { voice_idx = vi; found = true; break; }
                                }
                                if !found {
                                    let mut min_vol = 1_000_000.0f32;
                                    for vi in 0..r.voices.len() {
                                        if r.voices[vi].volume.output_volume < min_vol {
                                            min_vol = r.voices[vi].volume.output_volume;
                                            voice_idx = vi;
                                        }
                                    }
                                }

                                let voice = &mut r.voices[voice_idx];
                                voice.on = true;
                                voice.channel_idx = i;
                                voice.instrument = inst_idx;
                                voice.sample = final_sample_idx;
                                voice.sustained = true;
                                voice.sample_position = 4.0;
                                voice.loop_started = false;
                                voice.ping = true;
                                voice.volume.retrig(instrument.samples[final_sample_idx].volume as i32);
                                if instrument.samples[final_sample_idx].panning < 255 {
                                    voice.panning.panning = instrument.samples[final_sample_idx].panning;
                                } else {
                                    voice.panning.panning = r.song_data.initial_channel_panning[i];
                                }
                                
                                voice.trigger_note(instruments, pattern.instrument != 0);
                                
                                let sample = &instrument.samples[final_sample_idx];
                                let mapped_note = it_mapping.0 + 1;
                                let real_note = (mapped_note as i16 + sample.relative_note as i16).clamp(1, 120) as u8;
                                channel.note.set_note(real_note, sample.finetune, mapped_note, r.frequency_tables);
                                channel.update_frequency_voice(voice, r.rate, false, r.frequency_tables);
                                voice.last_played_note = pattern.note;
                                channel.last_played_note = pattern.note;
                                channel.voice_idx = Some(voice_idx);
                            }
                        }
                    }
                }
            } else if pattern.note == 97 { // Note Off
                if note_delay_first_tick {
                    if let Some(v_idx) = channel.voice_idx {
                        r.voices[v_idx].key_off(instruments, false);
                    }
                }
            } else if pattern.note == 121 { // Note Cut
                if note_delay_first_tick {
                    if let Some(v_idx) = channel.voice_idx {
                        r.voices[v_idx].on = false;
                        r.voices[v_idx].volume.output_volume = 0.0;
                    }
                }
            } else if pattern.note == 122 { // Note Fade
                if note_delay_first_tick {
                    if let Some(v_idx) = channel.voice_idx {
                        r.voices[v_idx].sustained = false;
                        let instrument_nna = &instruments[r.voices[v_idx].instrument];
                        r.voices[v_idx].volume.fadeout_speed = (instrument_nna.volume_fadeout as i32) << 6;
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

            // Volume Column
            match pattern.volume {
                0..=64 => { channel.set_volume(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.volume); }
                65..=74 => { channel.fine_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, (pattern.volume - 65) as i8); }
                75..=84 => { channel.fine_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, -((pattern.volume - 75) as i8)); }
                85..=94 => { channel.volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, (pattern.volume - 85) as i8); }
                95..=104 => { channel.volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, -((pattern.volume - 95) as i8)); }
                105..=114 => { channel.porta_up(r.song_data.song_type, first_tick, (pattern.volume - 105) << 2); }
                115..=124 => { channel.porta_down(r.song_data.song_type, first_tick, (pattern.volume - 115) << 2); }
                128..=192 => { if let Some(v) = voice_ref.as_deref_mut() { v.panning.set_panning(((pattern.volume - 128) << 2) as i32); } }
                193..=202 => { channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), note_delay_first_tick, pattern.volume - 193, r.compatible_g, r.rate, r.frequency_tables); }
                203..=212 => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, 0, pattern.volume - 203, r.old_effects, r.rate, r.frequency_tables); }
                _ => {}
            }

            // Effect Column
            match pattern.effect {
                0x01 => { if first_tick { *r.speed = pattern.effect_param as u32; } }
                0x02 => { r.pattern_change.set_jump(first_tick, pattern.effect_param); }
                0x03 => { r.pattern_change.set_break(r.song_data.song_type, first_tick, pattern.effect_param); }
                0x04 => { channel.it_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param); }
                0x05 => { channel.porta_down(r.song_data.song_type, first_tick, pattern.effect_param); }
                0x06 => { channel.porta_up(r.song_data.song_type, first_tick, pattern.effect_param); }
                0x07 => { channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), first_tick, pattern.effect_param, r.compatible_g, r.rate, r.frequency_tables); }
                0x08 => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, pattern.get_x(), pattern.get_y(), r.old_effects, r.rate, r.frequency_tables); }
                0x0A => { channel.arpeggio(*r.tick, pattern.get_x(), pattern.get_y()); }
                0x0B => { 
                    channel.vibrato(voice_ref.as_deref_mut(), first_tick, 0, 0, r.old_effects, r.rate, r.frequency_tables);
                    channel.it_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param);
                }
                0x0C => { 
                    channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), first_tick, 0, r.compatible_g, r.rate, r.frequency_tables);
                    channel.it_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param);
                }
                0x11 => { channel.it_retrig(voice_ref.as_deref_mut(), instruments, *r.tick, pattern.effect_param); }
                0x14 => { if first_tick { r.bpm.update(pattern.effect_param as u32, r.rate); } }
                0x16 => { r.global_volume.set_volume(note_delay_first_tick, pattern.effect_param); }
                0x17 => { r.global_volume.volume_slide(note_delay_first_tick, pattern.effect_param); }
                0x18 => { if first_tick { if let Some(v) = voice_ref.as_deref_mut() { v.panning.set_panning((pattern.effect_param as i32 * 4).min(255)); } } }
                0x13 => {
                    let x = pattern.get_x();
                    let y = pattern.get_y();
                    match x {
                        0x08 => { if first_tick { if let Some(v) = voice_ref.as_deref_mut() { v.panning.set_panning((y << 4) as i32); } } }
                        0x0C => { if *r.tick == y as u32 { channel.on = false; if let Some(v) = voice_ref.as_deref_mut() { v.on = false; } } }
                        0x0E => { if first_tick { *r.row_delay = y as usize; } }
                        _ => {}
                    }
                }
                0x1A => { // Z: Resonant Filter
                    if first_tick {
                        if let Some(v) = voice_ref.as_deref_mut() {
                            if pattern.effect_param < 0x80 {
                                v.filter_cutoff = pattern.effect_param;
                            } else if (0x80..=0x8F).contains(&pattern.effect_param) {
                                v.filter_resonance = (pattern.effect_param & 0x0F) << 3;
                            }
                        }
                    }
                }
                _ => {}
            }

            if let Some(v) = voice_ref.as_deref_mut() {
                channel.update_frequency_voice(v, r.rate, false, r.frequency_tables);
            }
        }

        // 2. Process all active voices (IT volume formula)
        let global_vol_f32 = r.global_volume.volume as f32 / 128.0;
        for (v_idx, voice) in r.voices.iter_mut().enumerate() {
            if !voice.on { continue; }
            let channel_vol_f32 = r.channels[voice.channel_idx].channel_volume as f32 / 64.0;
            let channel_force_off = r.channels[voice.channel_idx].force_off;
            
            voice.update_envelopes(instruments, r.rate);
            voice.update_fadeout();
            
            // IT formula: fadeout * envelope * channel_vol/64 * inst_global/128 * sample_global/64 * global_vol/128
            let base = voice.compute_base_volume();
            let inst_vol = voice.instrument_global_volume as f32 / 128.0;
            let sample_vol = voice.sample_global_volume as f32 / 64.0;
            let output_vol = base * channel_vol_f32 * inst_vol * sample_vol * global_vol_f32;
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
            if is_note_valid(pattern.note, r.song_data.song_type) {
                if pattern.is_porta_to_note(r.song_data.song_type) {
                    if first_tick {
                        let inst_idx = channel.last_instrument;
                        if inst_idx != 0 && (pattern.note as usize - 1) < instruments[inst_idx].sample_indexes.len() {
                            let it_mapping = instruments[inst_idx].sample_indexes[pattern.note as usize - 1];
                            let sample_idx = it_mapping.1 as usize;
                            if sample_idx > 0 && (sample_idx - 1) < instruments[inst_idx].samples.len() {
                                let sample = &instruments[inst_idx].samples[sample_idx - 1];
                                let real_note = (it_mapping.0 as i16 + sample.relative_note as i16).clamp(0, 119) as u8;
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
                                
                                // XM usually only has one voice per channel, but we use the voice pool for NNAs
                                let mut voice_idx = channel.voice_idx.unwrap_or(i);
                                if r.voices[voice_idx].on && r.voices[voice_idx].channel_idx == i {
                                    r.voices[voice_idx].on = false;
                                }

                                // Find a free voice
                                let mut found = false;
                                for vi in 0..r.voices.len() {
                                    if !r.voices[vi].on {
                                        voice_idx = vi;
                                        found = true;
                                        break;
                                    }
                                }
                                if !found {
                                    // Steal loudest voice if none free
                                    let mut min_vol = 1_000_000.0f32;
                                    for vi in 0..r.voices.len() {
                                        if r.voices[vi].volume.output_volume < min_vol {
                                            min_vol = r.voices[vi].volume.output_volume;
                                            voice_idx = vi;
                                        }
                                    }
                                }

                                let voice = &mut r.voices[voice_idx];
                                voice.on = true;
                                voice.channel_idx = i;
                                voice.instrument = inst_idx;
                                voice.sample = final_sample_idx;
                                voice.sustained = true;
                                voice.sample_position = 4.0;
                                voice.loop_started = false;
                                voice.ping = true;
                                voice.volume.retrig(instrument.samples[final_sample_idx].volume as i32);
                                voice.panning.panning = r.song_data.initial_channel_panning[i];
                                
                                // XM: a note without instrument keeps the current instrument/envelope phase.
                                // Envelopes reset only when a new instrument is explicitly provided.
                                voice.trigger_note(instruments, pattern.instrument != 0);
                                
                                let sample = &instrument.samples[final_sample_idx];
                                let mapped_note = it_mapping.0 + 1;
                                let real_note = (it_mapping.0 as i16 + sample.relative_note as i16).clamp(0, 119) as u8;
                                channel.note.set_note(real_note, sample.finetune, mapped_note, r.frequency_tables);
                                channel.update_frequency_voice(voice, r.rate, false, r.frequency_tables);
                                voice.last_played_note = pattern.note;
                                channel.voice_idx = Some(voice_idx);
                            }
                        }
                    }
                }
            } else if pattern.note == 97 { // Note Off
                if note_delay_first_tick {
                    if let Some(v_idx) = channel.voice_idx {
                        r.voices[v_idx].key_off(instruments, false);
                    }
                }
            } else if pattern.note == 121 { // Note Cut
                if note_delay_first_tick {
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

            // Volume Column
            match pattern.volume {
                0x10..=0x50 => { channel.set_volume(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.volume - 0x10); }
                0x60..=0x6f => { channel.volume_slide(voice_ref.as_deref_mut(), false, -(pattern.get_volume_param() as i8)); }
                0x70..=0x7f => { channel.volume_slide(voice_ref.as_deref_mut(), false, pattern.get_volume_param() as i8); }
                0x80..=0x8f => { channel.fine_volume_slide(voice_ref.as_deref_mut(), first_tick, -(pattern.get_volume_param() as i8)); }
                0x90..=0x9f => { channel.fine_volume_slide(voice_ref.as_deref_mut(), first_tick, pattern.get_volume_param() as i8); }
                0xa0..=0xaf => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, 0, pattern.get_volume_param(), r.old_effects, r.rate, r.frequency_tables); }
                0xb0..=0xbf => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, pattern.get_volume_param(), 0, r.old_effects, r.rate, r.frequency_tables); }
                0xd0..=0xdf => { channel.panning_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.get_volume_param() << 4); }
                0xe0..=0xef => { channel.panning_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.get_volume_param()); }
                0xf0..=0xff => { channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), note_delay_first_tick, pattern.get_volume_param(), r.compatible_g, r.rate, r.frequency_tables); }
                _ => {}
            }

            // Effect Column
            match pattern.effect {
                0x0 => { // Arpeggio
                    if pattern.effect_param != 0 {
                        channel.arpeggio(*r.tick, pattern.get_x(), pattern.get_y());
                    }
                }
                0x1 => { channel.porta_up(r.song_data.song_type, first_tick, pattern.effect_param); }
                0x2 => { channel.porta_down(r.song_data.song_type, first_tick, pattern.effect_param); }
                0x3 => { channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), first_tick, pattern.effect_param, r.compatible_g, r.rate, r.frequency_tables); }
                0x4 => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, pattern.get_x(), pattern.get_y(), r.old_effects, r.rate, r.frequency_tables); }
                0x5 => { 
                    channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), first_tick, 0, r.compatible_g, r.rate, r.frequency_tables); 
                    channel.volume_slide_main(voice_ref.as_deref_mut(), first_tick, pattern.effect_param); 
                }
                0x6 => { 
                    channel.vibrato(voice_ref.as_deref_mut(), first_tick, 0, 0, r.old_effects, r.rate, r.frequency_tables); 
                    channel.volume_slide_main(voice_ref.as_deref_mut(), first_tick, pattern.effect_param); 
                }
                0x7 => { channel.tremolo(voice_ref.as_deref_mut(), first_tick, pattern.get_x(), pattern.get_y()); }
                0x8 => { if first_tick { if let Some(v) = voice_ref.as_deref_mut() { v.panning.set_panning(pattern.effect_param as i32); } } }
                0x9 => { // Sample Offset
                    if first_tick {
                        let mut param = pattern.effect_param as u32;
                        if param == 0 { param = channel.last_sample_offset; }
                        channel.last_sample_offset = param;
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
                    let subcommand = pattern.get_x();
                    let param = pattern.get_y();
                    match subcommand {
                        0x1 => { channel.fine_porta_up(r.song_data.song_type, first_tick, param); }
                        0x2 => { channel.fine_porta_down(r.song_data.song_type, first_tick, param); }
                        0x9 => { channel.it_retrig(voice_ref.as_deref_mut(), instruments, *r.tick, param); }
                        0xA => { channel.fine_volume_slide(voice_ref.as_deref_mut(), first_tick, param as i8); }
                        0xB => { channel.fine_volume_slide(voice_ref.as_deref_mut(), first_tick, -(param as i8)); }
                        0xC => { if *r.tick == param as u32 { channel.on = false; if let Some(v) = voice_ref.as_deref_mut() { v.on = false; } } }
                        0xE => { if first_tick { *r.row_delay = param as usize; } }
                        _ => {}
                    }
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
                0x19 => { // Pxy: Panning Slide
                    channel.panning_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param);
                }
                0x1B => { // Rxy: Multi Retrig Note + Volume Slide
                    channel.retrig(voice_ref.as_deref_mut(), instruments, *r.tick, pattern.get_y(), pattern.get_x());
                }
                _ => {}
            }

            if let Some(v) = voice_ref.as_deref_mut() {
                channel.update_frequency_voice(v, r.rate, false, r.frequency_tables);
            }
        }

        // 2. Process all active voices (XM volume formula)
        let global_vol_f32 = r.global_volume.volume as f32 / 64.0;
        for (v_idx, voice) in r.voices.iter_mut().enumerate() {
            if !voice.on { continue; }
            let channel_vol_f32 = r.channels[voice.channel_idx].channel_volume as f32 / 64.0;
            let channel_force_off = r.channels[voice.channel_idx].force_off;
            
            voice.update_envelopes(instruments, r.rate);
            voice.update_fadeout();
            
            // XM formula: fadeout * envelope * channel_vol/64 * global_vol/64
            let base = voice.compute_base_volume();
            let output_vol = base * channel_vol_f32 * global_vol_f32;
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
                        let mut voice_idx = channel.voice_idx.unwrap_or(i);
                        if r.voices[voice_idx].on && r.voices[voice_idx].channel_idx == i {
                            r.voices[voice_idx].on = false;
                        }

                        // Find a free voice
                        let mut found = false;
                        for vi in 0..r.voices.len() {
                            if !r.voices[vi].on { voice_idx = vi; found = true; break; }
                        }
                        if !found {
                            let mut min_vol = 1_000_000.0f32;
                            for vi in 0..r.voices.len() {
                                if r.voices[vi].volume.output_volume < min_vol {
                                    min_vol = r.voices[vi].volume.output_volume;
                                    voice_idx = vi;
                                }
                            }
                        }

                        let voice = &mut r.voices[voice_idx];
                        voice.on = true;
                        voice.channel_idx = i;
                        voice.instrument = inst_idx;
                        voice.sample = sample_idx;
                        voice.sustained = true;
                        voice.sample_position = 4.0;
                        voice.loop_started = false;
                        voice.ping = true;
                        if pattern.instrument != 0 {
                            voice.volume.retrig(instruments[inst_idx].samples[sample_idx].volume as i32);
                            if instruments[inst_idx].samples[sample_idx].panning < 255 {
                                voice.panning.panning = instruments[inst_idx].samples[sample_idx].panning;
                            } else {
                                voice.panning.panning = r.song_data.initial_channel_panning[i];
                            }
                        }
                        voice.trigger_note(instruments, pattern.instrument != 0);
                        
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
                0x00 => { channel.arpeggio(*r.tick, pattern.get_x(), pattern.get_y()); }
                0x01 => { channel.porta_up(r.song_data.song_type, first_tick, pattern.effect_param); }
                0x02 => { channel.porta_down(r.song_data.song_type, first_tick, pattern.effect_param); }
                0x03 => { channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), first_tick, pattern.effect_param, r.compatible_g, r.rate, r.frequency_tables); }
                0x04 => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, pattern.get_x(), pattern.get_y(), r.old_effects, r.rate, r.frequency_tables); }
                0x05 => { 
                    channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), first_tick, 0, r.compatible_g, r.rate, r.frequency_tables);
                    channel.volume_slide_main(voice_ref.as_deref_mut(), first_tick, pattern.effect_param);
                }
                0x06 => { 
                    channel.vibrato(voice_ref.as_deref_mut(), first_tick, 0, 0, r.old_effects, r.rate, r.frequency_tables);
                    channel.volume_slide_main(voice_ref.as_deref_mut(), first_tick, pattern.effect_param);
                }
                0x07 => { channel.tremolo(voice_ref.as_deref_mut(), first_tick, pattern.get_x(), pattern.get_y()); }
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
                        0xE => { if first_tick { *r.row_delay = y as usize; } }
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
            let channel_vol_f32 = r.channels[voice.channel_idx].channel_volume as f32 / 64.0;
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
            if is_note_valid(pattern.note, r.song_data.song_type) {
                if pattern.is_porta_to_note(r.song_data.song_type) {
                    if first_tick {
                        let inst_idx = channel.last_instrument;
                        if inst_idx != 0 && (pattern.note as usize - 1) < instruments[inst_idx].sample_indexes.len() {
                            let it_mapping = instruments[inst_idx].sample_indexes[pattern.note as usize - 1];
                            let sample_idx = it_mapping.1 as usize;
                            if sample_idx > 0 && (sample_idx - 1) < instruments[inst_idx].samples.len() {
                                let sample = &instruments[inst_idx].samples[sample_idx - 1];
                                let real_note = (it_mapping.0 as i16 + sample.relative_note as i16).clamp(0, 119) as u8;
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
                                
                                let mut voice_idx = channel.voice_idx.unwrap_or(i);
                                if r.voices[voice_idx].on && r.voices[voice_idx].channel_idx == i {
                                    r.voices[voice_idx].on = false;
                                }

                                // Find a free voice
                                let mut found = false;
                                for vi in 0..r.voices.len() {
                                    if !r.voices[vi].on { voice_idx = vi; found = true; break; }
                                }
                                if !found {
                                    let mut min_vol = 1_000_000.0f32;
                                    for vi in 0..r.voices.len() {
                                        if r.voices[vi].volume.output_volume < min_vol {
                                            min_vol = r.voices[vi].volume.output_volume;
                                            voice_idx = vi;
                                        }
                                    }
                                }

                                let voice = &mut r.voices[voice_idx];
                                voice.on = true;
                                voice.channel_idx = i;
                                voice.instrument = inst_idx;
                                voice.sample = final_sample_idx;
                                voice.sustained = true;
                                voice.sample_position = 4.0;
                                voice.loop_started = false;
                                voice.ping = true;
                                voice.volume.retrig(instrument.samples[final_sample_idx].volume as i32);
                                if instrument.samples[final_sample_idx].panning < 255 {
                                    voice.panning.panning = instrument.samples[final_sample_idx].panning;
                                } else {
                                    voice.panning.panning = r.song_data.initial_channel_panning[i];
                                }
                                
                                voice.trigger_note(instruments, pattern.instrument != 0);
                                
                                let sample = &instrument.samples[final_sample_idx];
                                let mapped_note = it_mapping.0 + 1;
                                let real_note = (mapped_note as i16 + sample.relative_note as i16).clamp(1, 120) as u8;
                                channel.note.set_note(real_note, sample.finetune, mapped_note, r.frequency_tables);
                                channel.update_frequency_voice(voice, r.rate, false, r.frequency_tables);
                                voice.last_played_note = pattern.note;
                                channel.last_played_note = pattern.note;
                                channel.voice_idx = Some(voice_idx);
                            }
                        }
                    }
                }
            } else if pattern.note == 97 || pattern.note == 253 { // Note Off
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
                    0..=64 => { channel.set_volume(voice_ref.as_deref_mut(), true, pattern.volume); }
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
                8 => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, pattern.get_x(), pattern.get_y(), r.old_effects, r.rate, r.frequency_tables); }
                10 => { channel.arpeggio(*r.tick, pattern.get_x(), pattern.get_y()); }
                11 => { 
                    channel.vibrato(voice_ref.as_deref_mut(), first_tick, 0, 0, r.old_effects, r.rate, r.frequency_tables);
                    channel.it_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param);
                }
                12 => { 
                    channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), first_tick, 0, r.compatible_g, r.rate, r.frequency_tables);
                    channel.it_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param);
                }
                13 => { if first_tick { channel.channel_volume = pattern.effect_param.min(64); } }
                14 => { channel.channel_volume_slide(note_delay_first_tick, pattern.effect_param); }
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
                16 => { channel.panning_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param); }
                17 => { channel.it_retrig(voice_ref.as_deref_mut(), instruments, *r.tick, pattern.effect_param); }
                19 => {
                    let x = pattern.get_x();
                    let y = pattern.get_y();
                    match x {
                        0x8 => { if first_tick { if let Some(v) = voice_ref.as_deref_mut() { v.panning.set_panning((y as i32 * 17).min(255)); } } }
                        0xC => { if *r.tick == y as u32 { channel.on = false; if let Some(v) = voice_ref.as_deref_mut() { v.on = false; } } }
                        0xE => { if first_tick { *r.row_delay = y as usize; } }
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
        for (v_idx, voice) in r.voices.iter_mut().enumerate() {
            if !voice.on { continue; }
            let channel_vol_f32 = r.channels[voice.channel_idx].channel_volume as f32 / 64.0;
            let channel_force_off = r.channels[voice.channel_idx].force_off;
            
            voice.update_envelopes(instruments, r.rate);
            voice.update_fadeout();
            
            // S3M formula: fadeout * envelope * channel_vol/64 * global_vol/64
            // S3M has channel_volume and global_volume but no instrument/sample global volume
            let base = voice.compute_base_volume();
            let output_vol = base * channel_vol_f32 * global_vol_f32;
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
