use crate::pattern::NoteAction;
use crate::song::backend::{
    alloc_voice, apply_extended, cut_or_nna_existing_voice, init_voice_basics,
    mute_silent_voices, ModuleBackend, SongPlaybackResources, MOD_E_TABLE,
};

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

            // Note trigger logic. MOD doesn't encode Off/Cut/Fade, so the
            // only branches we need are Trigger and None - and None still
            // matters because an instrument-only row (no note) refreshes
            // sample volume/panning on the existing voice.
            match pattern.note_action(r.song_data.song_type) {
            NoteAction::Trigger(_) => {
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
                        let prev_voice_idx = channel.voice_idx.unwrap_or(i);
                        cut_or_nna_existing_voice(r.voices, instruments, r.song_data.song_type, i, prev_voice_idx);

                        let voice_idx = alloc_voice(r.voices);
                        init_voice_basics(&mut r.voices[voice_idx], i, inst_idx, sample_idx);
                        let voice = &mut r.voices[voice_idx];
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
            }
            // Instrument-only row (no note): refresh sample volume / panning
            // / envelope state on the existing voice. MOD-style "ghost note".
            NoteAction::None if pattern.instrument != 0 && first_tick => {
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
            // MOD doesn't have Off / Cut / Fade encodings, and bare Note::None
            // (no instrument) is also a no-op.
            _ => {}
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
                        let param = channel.recall_or_set(crate::channel_state::EffectMemorySlot::SampleOffset, pattern.effect_param);
                        if let Some(v) = voice_ref.as_deref_mut() { v.sample_position = (param as f32) * 256.0 + 4.0; }
                    }
                }
                0x0A => { channel.volume_slide_main(voice_ref.as_deref_mut(), first_tick, pattern.effect_param); }
                0x0B => { if first_tick { r.pattern_change.set_jump(true, pattern.effect_param); } }
                0x0C => { channel.set_volume(voice_ref.as_deref_mut(), first_tick, pattern.effect_param); }
                0x0D => { if first_tick { r.pattern_change.set_break(r.song_data.song_type, true, pattern.effect_param); } }
                0x0E => {
                    let kind = MOD_E_TABLE[pattern.get_x() as usize];
                    apply_extended(
                        kind, channel, voice_ref.as_deref_mut(),
                        r.pattern_change, instruments,
                        *r.tick, *r.row, first_tick, first_tick,
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
                _ => {}
            }

            if let Some(v) = voice_ref.as_deref_mut() {
                channel.update_frequency_voice(v, r.rate, false, r.frequency_tables);
            }
        }

        // 2. Process all active voices (MOD volume formula).
        //
        // MOD has no envelopes, no per-instrument or per-sample global volume,
        // and no song-level global volume. compute_base_volume() degenerates to
        //   (fadeout/65536) * (vol/64) + tremolo, clamped, * 1.0
        // because envelope_vol defaults to 16384 (full) and sample.global_volume
        // defaults to 64. Going through compute_base_volume() lets MOD pick up
        // tremolo_shift (effect 0x07), which the previous hand-rolled formula
        // ignored - the handler ran but the output never landed.
        for voice in r.voices.iter_mut() {
            if !voice.on { continue; }
            let channel_force_off = r.channels[voice.channel_idx].force_off;

            voice.update_fadeout();

            let output_vol = voice.compute_base_volume();
            voice.set_output_volume(output_vol);

            if channel_force_off {
                voice.set_output_volume(0.0);
            }
        }

        mute_silent_voices(r.voices, r.channels);
    }
}
