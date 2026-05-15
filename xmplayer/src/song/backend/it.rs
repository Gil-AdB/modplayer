use crate::instrument::Instrument;
use crate::channel_state::Voice;
use crate::pattern::NoteAction;
use crate::song::backend::{
    alloc_voice, apply_flow_control_effect, apply_porta_retrig_if_needed,
    bind_voice_for_channel, dispatch_main_and_extended, dispatch_vol_col,
    init_channel_iter, init_voice_basics, mute_silent_voices, process_voices,
    set_channel_note, validate_voice_pool, voice_mix, EffectCtx, ModuleBackend,
    SongPlaybackResources, IT_EFFECT_TABLE, IT_S_TABLE, IT_VOL_COL,
};

pub struct ItBackend {}

impl ItBackend {
    pub fn new() -> Self {
        Self {}
    }

    fn apply_it_action(voices: &mut [Voice], channel: &mut crate::channel_state::ChannelState, voice_idx: usize, action: u8, instrument: &Instrument) {
        if action == 0 {
            // Cut: snapshot the voice state into a background slot and
            // ramp THAT copy to 0 — the source slot is freed so the
            // new note's alloc_voice can claim it and start its own
            // ramp from 0. Then clear the host channel's voice_idx if
            // it points at this slot, otherwise a later alloc_voice
            // hand-off to a different channel orphans the pointer (the
            // spx-shuttledeparture DCT=3 invariant warnings).
            crate::song::backend::spawn_background_cut_inline(
                voices, voice_idx,
                crate::channel_state::VoiceCutReason::NoteCut,
            );
            if channel.voice_idx == Some(voice_idx) {
                channel.voice_idx = None;
            }
            return;
        }
        let voice = &mut voices[voice_idx];
        match action {
            0 => { unreachable!(); }
            1 => { // Continue
                // Do nothing
            }
            2 => { // Note Off (NNA=2, equivalent to OMT KeyOff)
                // Per OMT's `KeyOff` (Snd_fx.cpp:6141): release sustain (so
                // any vol envelope enters its release phase) AND set the
                // fadeout flag (`!sustained`). For instruments with no vol
                // envelope and fadeout==0 (orbiter inst 15), this means the
                // voice keeps playing at full level until the sample ends
                // naturally — the new NNA-spawned voice overlaps it. Our
                // previous implementation cut the voice immediately when
                // the envelope was off, which silenced the tail at every
                // retrigger and made orbiter ch7 ~10× quieter than OMT.
                voice.sustained = false;
                voice.volume.fadeout_speed = (instrument.volume_fadeout as i32) << 6;
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

        // Deferred stale-pointer cleanup — see xm.rs's process_tick for
        // the rationale. alloc_voice's fallback paths hand us slots
        // whose previous owner still has a stale channels[old].voice_idx
        // pointer. We stash the (old_owner, voice_idx) pairs here and
        // apply after the per-channel loop ends.
        let mut stale_clears: Vec<(usize, usize)> = Vec::new();

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
                                let mapped = (it_mapping.0 + 1) as u8;
                                channel.porta_to_note.target_note.period = if sample.c5_speed != 0 {
                                    // IT: mapped 1-indexed; offset -1. Don't apply
                                    // sample.relative_note — IT's loader bakes the
                                    // c5_speed offset into it.
                                    crate::channel_state::channel_state::Note::note_to_period_s3m(mapped, -1, sample.c5_speed)
                                } else {
                                    let real_note = (it_mapping.0 as i16 + sample.relative_note as i16).clamp(1, 120) as u8;
                                    channel.note.note_to_period(real_note, sample.finetune, r.frequency_tables)
                                };
                                // IT-linear path also needs an absolute Hz
                                // target so `PortaToNoteState::next_tick`
                                // can slide `linear_hz` instead of the
                                // unused period. Mirrors the trigger
                                // path's use of `it_linear_frequency`
                                // a few lines below — same mapped note +
                                // sample c5_speed.
                                if !r.song_data.use_amiga && sample.c5_speed != 0 {
                                    channel.porta_to_note.target_note.linear_hz =
                                        crate::channel_state::channel_state::Note::it_linear_frequency(mapped, sample.c5_speed);
                                } else {
                                    channel.porta_to_note.target_note.linear_hz = 0.0;
                                }
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
                                    let (on, ci, last_note, samp, inst_id) = {
                                        let v = &r.voices[vi];
                                        (v.on, v.channel_idx, v.last_played_note, v.sample, v.instrument)
                                    };
                                    if !on || ci != i { continue; }
                                    match instrument.dct {
                                        1 => { if last_note == pattern.note { Self::apply_it_action(r.voices, channel, vi, instrument.dca, instrument); dca_applied = true; } }
                                        2 => { if samp == final_sample_idx && inst_id == inst_idx { Self::apply_it_action(r.voices, channel, vi, instrument.dca, instrument); dca_applied = true; } }
                                        3 => { if inst_id == inst_idx { Self::apply_it_action(r.voices, channel, vi, instrument.dca, instrument); dca_applied = true; } }
                                        _ => {}
                                    }
                                }
                                if !dca_applied {
                                    if let Some(v_idx) = channel.voice_idx {
                                        if r.voices[v_idx].on {
                                            Self::apply_it_action(r.voices, channel, v_idx, instrument.nna, instrument);
                                        }
                                    }
                                }

                                // For IT envCarry, snapshot the previous voice's
                                // envelope state — but only when the new note is
                                // re-triggering the *same* instrument. Per IT
                                // spec the carry bit means "don't reset this
                                // envelope on retrigger of this instrument";
                                // a fresh instrument always reinitialises.
                                let carry_snapshot = channel.voice_idx
                                    .filter(|&vi| {
                                        r.voices.get(vi)
                                            .map(|v| v.on && v.instrument == inst_idx)
                                            .unwrap_or(false)
                                    })
                                    .map(|vi| {
                                        let pv = &r.voices[vi];
                                        (
                                            pv.volume_envelope_state,
                                            pv.panning_envelope_state,
                                            pv.pitch_envelope_state,
                                        )
                                    });

                                let voice_idx = alloc_voice(r.voices);
                                {
                                    let v = &r.voices[voice_idx];
                                    if v.on && v.channel_idx != i {
                                        stale_clears.push((v.channel_idx, voice_idx));
                                    }
                                }
                                init_voice_basics(&mut r.voices[voice_idx], i, inst_idx, final_sample_idx);
                                let voice = &mut r.voices[voice_idx];
                                voice.volume.retrig(instrument.samples[final_sample_idx].volume as i32);
                                if instrument.samples[final_sample_idx].panning < 255 {
                                    voice.panning.panning = instrument.samples[final_sample_idx].panning;
                                } else {
                                    voice.panning.panning = r.song_data.initial_channel_panning[i];
                                }

                                // Drop the snapshot into the new voice's envelope
                                // states for the carry-enabled envelopes only. (For
                                // non-carry envelopes trigger_note will overwrite
                                // these via reset just below.)
                                if let Some((vol_env, pan_env, pitch_env)) = carry_snapshot {
                                    if instrument.volume_envelope.carry  { voice.volume_envelope_state  = vol_env; }
                                    if instrument.panning_envelope.carry { voice.panning_envelope_state = pan_env; }
                                    if instrument.pitch_envelope.carry   { voice.pitch_envelope_state   = pitch_env; }
                                }

                                voice.trigger_note(instruments, pattern.instrument != 0, channel.vibrato_retrig, channel.tremolo_retrig);

                                let sample = &instrument.samples[final_sample_idx];
                                let mapped_note = it_mapping.0 + 1;
                                set_channel_note(channel, voice, sample.relative_note, sample.finetune, mapped_note, r.rate, r.frequency_tables);
                                // IT c5_speed: amiga goes through the period
                                // table; linear mode stashes Hz on linear_hz
                                // and `Note::frequency` returns it directly.
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
                        if r.voices[v_idx].channel_idx == i {
                            r.voices[v_idx].key_off(instruments, r.song_data.song_type);
                        }
                    }
                }
            }
            NoteAction::Cut => {
                if note_delay_first_tick {
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
            NoteAction::Fade => {
                if note_delay_first_tick {
                    if let Some(v_idx) = channel.voice_idx {
                        if r.voices[v_idx].channel_idx == i {
                            r.voices[v_idx].sustained = false;
                            let instrument_nna = &instruments[r.voices[v_idx].instrument];
                            r.voices[v_idx].volume.fadeout_speed = (instrument_nna.volume_fadeout as i32) << 6;
                        }
                    }
                }
            }
            NoteAction::None => {}
            }

            apply_porta_retrig_if_needed(
                r.voices, channel, pattern, i, first_tick, instruments, r.song_data.song_type,
            );

            let mut voice_ref = bind_voice_for_channel(r.voices, channel, i);

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
                fast_volume_slides: r.song_data.fast_volume_slides,
            };

            // Volume column: data-driven via IT_VOL_COL table (see backend.rs).
            dispatch_vol_col(IT_VOL_COL, pattern.volume, channel, voice_ref.as_deref_mut(), &ctx);

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
                &mut ctx, &IT_EFFECT_TABLE, &IT_S_TABLE,
            );

            if let Some(v) = voice_ref.as_deref_mut() {
                channel.update_frequency_voice(v, r.rate, false, r.frequency_tables);
                if channel.vibrato_active_this_row && !first_tick {
                    channel.advance_vibrato_pos(v);
                }
            }
        }

        // Apply deferred stale-pointer clears (see top of process_tick).
        for (ch_idx, vi) in stale_clears {
            if ch_idx < r.channels.len() && r.channels[ch_idx].voice_idx == Some(vi) {
                r.channels[ch_idx].voice_idx = None;
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
        validate_voice_pool(r.voices, r.channels);
    }
}
