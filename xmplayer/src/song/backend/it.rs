use crate::instrument::Instrument;
use crate::channel_state::Voice;
use crate::pattern::NoteAction;
use crate::song::backend::{
    alloc_voice, apply_extended, apply_flow_control_effect, init_voice_basics,
    mute_silent_voices, set_channel_note, ModuleBackend, SongPlaybackResources,
    IT_S_TABLE,
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
            channel.tremor_silenced = false;

            let patterns = &r.song_data.patterns[r.song_data.pattern_order[*r.song_position] as usize];
            let row = &patterns.rows[*r.row];
            let pattern = &row.channels[i];
            
            let is_note_delay = pattern.is_note_delay(r.song_data.song_type);
            let note_delay_first_tick = if is_note_delay { *r.tick == pattern.get_y() as u32 } else { first_tick };

            if pattern.instrument != 0 {
                channel.last_instrument = if (pattern.instrument as usize) < instruments.len() { pattern.instrument as usize } else { 0 };
            }

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
                                let real_note = (it_mapping.0 as i16 + sample.relative_note as i16).clamp(1, 120) as u8;
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

            match pattern.effect {
                // 0x01 (A) SetSpeed, 0x02 (B) PatternJump,
                // 0x03 (C) PatternBreak, 0x14 (T) SetBpm -- all handled
                // by apply_flow_control_effect.
                0x04 => { channel.it_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param); }
                0x05 => { channel.porta_down(r.song_data.song_type, first_tick, pattern.effect_param); }
                0x06 => { channel.porta_up(r.song_data.song_type, first_tick, pattern.effect_param); }
                0x07 => { channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), first_tick, pattern.effect_param, r.compatible_g, r.rate, r.frequency_tables); }
                0x08 => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, pattern.get_x(), pattern.get_y(), r.old_effects, r.rate, r.frequency_tables, r.song_data.song_type); }
                0x0A => { channel.arpeggio(*r.tick, pattern.get_x(), pattern.get_y(), true); }
                0x0B => { 
                    channel.vibrato(voice_ref.as_deref_mut(), first_tick, 0, 0, r.old_effects, r.rate, r.frequency_tables, r.song_data.song_type);
                    channel.it_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param);
                }

                0x0C => { 
                    channel.porta_to_note(r.song_data.song_type, voice_ref.as_deref_mut(), first_tick, 0, r.compatible_g, r.rate, r.frequency_tables);
                    channel.it_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param);
                }
                0x11 => { channel.it_retrig(voice_ref.as_deref_mut(), instruments, *r.tick, pattern.effect_param); }
                // 0x14 (T) SetBpm - handled by apply_flow_control_effect.
                0x16 => { r.global_volume.set_volume(note_delay_first_tick, pattern.effect_param); }
                0x17 => { r.global_volume.volume_slide(note_delay_first_tick, pattern.effect_param); }
                0x18 => { if first_tick { if let Some(v) = voice_ref.as_deref_mut() { v.panning.set_panning((pattern.effect_param as i32 * 4).min(255)); } } }
                0x13 => {
                    let kind = IT_S_TABLE[pattern.get_x() as usize];
                    apply_extended(
                        kind, channel, voice_ref.as_deref_mut(),
                        r.pattern_change, instruments,
                        *r.tick, *r.row, first_tick, first_tick,
                        r.song_data.song_type, r.rate, r.frequency_tables,
                        pattern.get_y(),
                    );
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

        // 2. Process all active voices (IT volume formula).
        let global_vol_f32 = r.global_volume.volume as f32 / 128.0;
        for voice in r.voices.iter_mut() {
            if !voice.on { continue; }
            let channel = &r.channels[voice.channel_idx];
            let silenced = channel.force_off || channel.tremor_silenced;

            voice.update_envelopes(instruments, r.rate);
            voice.update_fadeout();

            // IT formula: fadeout * envelope * note_vol/64 + tremolo, clamped,
            //   * sample_global/64 * inst_global/128 * global_vol/128.
            // sample_global is already inside compute_base_volume(); don't
            // multiply by it again here (fixes a regression where samples with
            // non-default global volume came out attenuated by an extra
            // sample_global/64 factor).
            let inst_vol = voice.instrument_global_volume as f32 / 128.0;
            let output_vol = voice.compute_base_volume() * inst_vol * global_vol_f32;
            voice.set_output_volume(output_vol);

            if silenced {
                voice.set_output_volume(0.0);
            }
        }

        mute_silent_voices(r.voices, r.channels);
    }
}
