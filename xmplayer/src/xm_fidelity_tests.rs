
#[cfg(test)]
mod tests {
    use crate::test_utils::{MockSongBuilder, SongTester};
    use crate::module_reader::SongType;
    use crate::pattern::Pattern;

    #[test]
    fn test_xm_arpeggio_reset() {
        // XM Arpeggio (effect 0) with param x*0x10 + y rotates the pitch
        // shift through 0, -x*64, -y*64 every 3 ticks. The *64 factor is
        // the engine's period-units-per-semitone (kept consistent with the
        // amiga period table). When the row ends and the next row has no
        // arpeggio (effect 0, param 0), the shift resets to 0 because
        // EffectKind::Arpeggio with effect_param == 0 takes the "clear
        // period_shift" branch.
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(2);
        // Row 0: Arpeggio 037 (x=3, y=7)
        builder.add_pattern_row(0, 0, 0, 0, 255, 0x0, 0x37);
        // Row 1: empty (no effect, no params)

        let mut tester = builder.get_tester();

        const X_SHIFT: i16 = -(3 * 64); // tick%3 == 1
        const Y_SHIFT: i16 = -(7 * 64); // tick%3 == 2

        tester.tick(); // process row 0 tick 0 -> tick 1
        assert_eq!(tester.song.channels[0].period_shift, 0);

        tester.tick(); // process tick 1 -> tick 2
        assert_eq!(tester.song.channels[0].period_shift, X_SHIFT);

        tester.tick(); // process tick 2 -> tick 3
        assert_eq!(tester.song.channels[0].period_shift, Y_SHIFT);

        tester.tick(); // process tick 3 (3 % 3 == 0) -> tick 4
        assert_eq!(tester.song.channels[0].period_shift, 0);

        // Drain to row 1 first tick.
        tester.step_to_row(1);
        tester.tick();
        assert_eq!(tester.song.channels[0].period_shift, 0,
                   "arpeggio shift should clear on a row with empty effect");
    }

    #[test]
    fn test_xm_multi_retrig() {
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(1);
        
        // Setup a note to retrig
        // Row 0: Note C-4, Inst 1, Multi-Retrig R02 (Retrig every 2 ticks, no vol change)
        builder.add_pattern_row(0, 0, 48, 1, 255, 0x1B, 0x02);
        
        let mut tester = builder.get_tester();
        
        // Tick 0: Trigger note. Retrig also runs (0 % 2 == 0).
        tester.tick();
        assert!(tester.song.voices[0].on);
        assert_eq!(tester.song.voices[0].sample_position, 4.0);
        
        // Manually advance position to simulate playback
        tester.song.voices[0].sample_position = 100.0;

        // Tick 1: No retrig (1 % 2 != 0)
        tester.tick();
        assert_eq!(tester.song.voices[0].sample_position, 100.0);

        // Tick 2: Retrig! (2 % 2 == 0)
        tester.tick();
        // Sample position should be reset to 4.0
        assert_eq!(tester.song.voices[0].sample_position, 4.0);
    }

    #[test]
    fn test_xm_key_off_logic() {
        // XM Kxy fires key-off when tick == y (XM spec: "Key off the note
        // at tick xx"). Confirmed against the apply_effect dispatch's
        // KeyOffAtTick arm: `if ctx.tick == pattern.effect_param as u32`.
        // FT2 semantics: key-off ENDS SUSTAIN (envelope advances past
        // sustain point, fadeout starts ramping `fadeout_vol` down) but
        // does NOT cut the voice immediately — repro mview.xm ch12.
        // Mock instrument has volume_fadeout=0 so the voice stays on
        // indefinitely; we observe key-off via `sustained == false`.
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(1);
        // K04 -> Key Off when tick == 4. Default speed is 6 so we have
        // ticks 0..5 within the row.
        builder.add_pattern_row(0, 0, 48, 1, 255, 0x14, 0x04);

        let mut tester = builder.get_tester();

        for _ in 0..4 {
            tester.tick();
            assert!(tester.song.voices[0].sustained, "voice should still be sustained before tick 4 fires Kxx");
        }
        // Now we've processed ticks 0..3 and we're about to process tick 4.
        tester.tick();
        assert!(!tester.song.voices[0].sustained, "K04 should end sustain at tick 4");
    }

    #[test]
    fn test_xm_key_off_no_envelope_silences_voice() {
        // FT2: key-off with no volume envelope zeros the voice volume
        // (`realVol = 0; outVol = 0`). Voice stays alive (sample
        // position advances) but is silent — equivalent to a cut for
        // audible output.
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(1);
        builder.add_pattern_row(0, 0, 48, 1, 255, 0x14, 0x03); // K03

        let mut tester = builder.get_tester();

        for _ in 0..3 {
            tester.tick();
            assert!(tester.song.voices[0].sustained);
        }
        tester.tick();
        assert!(!tester.song.voices[0].sustained, "K03 should end sustain at tick 3");
        assert_eq!(tester.song.voices[0].volume.volume, 0,
            "key-off with no vol-env should zero voice volume");
    }

    #[test]
    fn test_xm_note_without_instrument_keeps_envelope_position() {
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(2);

        // Enable a simple envelope so we can observe phase continuity.
        builder.instruments[1].volume_envelope.on = true;
        builder.instruments[1].volume_envelope.points[0].frame = 0;
        builder.instruments[1].volume_envelope.points[0].value = 64;
        builder.instruments[1].volume_envelope.points[1].frame = 40;
        builder.instruments[1].volume_envelope.points[1].value = 32;
        builder.instruments[1].volume_envelope.size = 2;

        // Row 0: note + instrument (starts envelope)
        builder.add_pattern_row(0, 0, 48, 1, 255, 0x0, 0x00);
        // Row 1: note without instrument (XM should NOT reset envelope)
        builder.add_pattern_row(0, 1, 50, 0, 255, 0x0, 0x00);

        let mut tester = builder.get_tester();

        // Advance through row 0 so envelope position grows.
        tester.step_to_row(1);
        let env_before = tester.song.voices[0].volume_envelope_state.frame;
        assert!(env_before > 0);

        // Process row 1 tick 0 note event; envelope should continue, not jump back to 0.
        tester.tick();
        let env_after = tester.song.voices[0].volume_envelope_state.frame;
        assert!(env_after >= env_before, "Envelope reset unexpectedly: before={}, after={}", env_before, env_after);
    }

    #[test]
    fn test_xm_set_envelope_pos_pan_gating() {
        // FT2 logic bug: panning-envelope position only updates when the
        // *volume* envelope's sustain flag is set. Volume-envelope
        // position is gated on the volume envelope being enabled.
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(2);

        // Volume envelope enabled, sustain OFF.
        builder.instruments[1].volume_envelope.on = true;
        builder.instruments[1].volume_envelope.sustain = false;
        builder.instruments[1].volume_envelope.points[0].frame = 0;
        builder.instruments[1].volume_envelope.points[0].value = 64;
        builder.instruments[1].volume_envelope.points[1].frame = 40;
        builder.instruments[1].volume_envelope.points[1].value = 32;
        builder.instruments[1].volume_envelope.size = 2;

        // Panning envelope present and enabled (independent of vol-env sustain).
        builder.instruments[1].panning_envelope.on = true;
        builder.instruments[1].panning_envelope.points[0].frame = 0;
        builder.instruments[1].panning_envelope.points[0].value = 32;
        builder.instruments[1].panning_envelope.points[1].frame = 40;
        builder.instruments[1].panning_envelope.points[1].value = 16;
        builder.instruments[1].panning_envelope.size = 2;

        // Row 0: trigger. Row 1: Lxx = L20 (set env pos to 0x20 = 32).
        builder.add_pattern_row(0, 0, 48, 1, 255, 0x00, 0x00);
        builder.add_pattern_row(0, 1, 0, 0, 255, 0x15, 0x20);

        let mut tester = builder.get_tester();
        tester.step_to_row(1);
        tester.tick(); // row 1 tick 0 — Lxx fires

        let v = &tester.song.voices[0];
        // vol-env.on=true → vol-env position jumped to 32 (+1 for tick advance).
        assert!(v.volume_envelope_state.frame >= 32 && v.volume_envelope_state.frame <= 33,
            "vol-env frame should be ~32, got {}", v.volume_envelope_state.frame);
        // vol-env.sustain=false → pan-env position NOT moved (stayed at start).
        assert!(v.panning_envelope_state.frame < 32,
            "pan-env should not have moved: frame={}", v.panning_envelope_state.frame);
    }

    #[test]
    fn test_xm_panning_set_full_range() {
        // XM 8xy ("set panning") takes the full byte 0..=255 directly.
        // Apply_effect's SetPanningXm arm passes effect_param straight to
        // panning.set_panning. 0xFF should land at 255 (hard right).
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(1);
        builder.add_pattern_row(0, 0, 48, 1, 255, 0x08, 0xFF);

        let mut tester = builder.get_tester();
        tester.tick();
        assert_eq!(tester.song.voices[0].panning.panning, 255);
    }

    #[test]
    fn test_xm_arpeggio_no_memory() {
        // XM arpeggio (effect 0) reads x/y directly from this row's
        // effect_param. Per the dispatch's `has_memory = false` branch
        // for XM/MOD, a row with effect 0 / param 0 takes the
        // "clear period_shift" path and does NOT recall the previous
        // row's arpeggio nibbles.
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(2);
        builder.add_pattern_row(0, 0, 48, 1, 255, 0x0, 0x37);  // arpeggio 037
        builder.add_pattern_row(0, 1, 0,  0, 255, 0x0, 0x00);  // empty

        let mut tester = builder.get_tester();
        tester.step_to_row(1);
        // Multiple ticks of an empty arpeggio row shouldn't recall 037.
        for _ in 0..5 {
            tester.tick();
            assert_eq!(tester.song.channels[0].period_shift, 0,
                       "XM arpeggio has no memory; empty row stays at 0");
        }
    }

    #[test]
    fn test_xm_multi_retrig_with_volume_change() {
        // XM Rxy: retrig every y ticks; x specifies the volume modifier.
        // Volume modifier 1 = -1, 2 = -2, 3 = -4, 9 = +1, etc. (per
        // ChannelState::retrig). R12 -> retrig every 2 ticks, x=1 (-1
        // per retrig).
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(1);
        // Note + volume 50 (out of 64) so subtract-1 leaves room.
        builder.set_pattern_row(0, 0, 0, Pattern {
            note: 48, instrument: 1, volume: 0x10 + 50, // XM vol col: 0x10..=0x50 = set vol 0..64
            effect: 0x1B, effect_param: 0x12,
        });

        let mut tester = builder.get_tester();
        tester.tick(); // tick 0: trigger note, retrig fires (0%2==0 but tick==0 so no retrig)
        let v0 = tester.song.voices[0].volume.volume;
        tester.tick(); // tick 1: no retrig
        tester.tick(); // tick 2: retrig fires, vol -= 1
        let v2 = tester.song.voices[0].volume.volume;
        assert!(v2 < v0, "Rxy with x=1 should subtract 1 from volume on retrig (v0={}, v2={})", v0, v2);
        assert_eq!(tester.song.voices[0].sample_position, 4.0,
                   "retrig should reset sample position");
    }

    #[test]
    fn test_xm_set_global_volume() {
        // XM Gxx (effect 0x10) sets global volume on first tick.
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(1);
        builder.add_pattern_row(0, 0, 48, 1, 255, 0x10, 0x20); // G20 -> global vol 32
        let mut tester = builder.get_tester();
        tester.tick();
        assert_eq!(tester.song.global_volume.volume, 0x20);
    }

    #[test]
    fn test_xm_global_volume_slide() {
        // XM Hxy (effect 0x11) slides global volume per tick. H02 -> down by 2.
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(1);
        builder.add_pattern_row(0, 0, 48, 1, 255, 0x11, 0x02);
        let mut tester = builder.get_tester();
        let start = tester.song.global_volume.volume;
        tester.tick(); // first_tick - no slide
        tester.tick(); // tick 1: slide -2
        tester.tick(); // tick 2: slide -2
        let after = tester.song.global_volume.volume;
        assert!(after < start, "global volume should slide down: {} -> {}", start, after);
    }

    #[test]
    fn test_xm_panning_slide_pxy() {
        // XM Pxy (effect 0x19): right > 0 slides right; left > 0 slides left.
        // Defaults to channel panning 128. P02 = slide right by 2 per non-first tick.
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(1);
        builder.add_pattern_row(0, 0, 48, 1, 255, 0x19, 0x20); // P20 -> right=2
        let mut tester = builder.get_tester();
        tester.tick(); // tick 0
        let p0 = tester.song.voices[0].panning.panning;
        tester.tick(); // tick 1
        tester.tick(); // tick 2
        let p2 = tester.song.voices[0].panning.panning;
        assert!(p2 > p0, "P20 should slide panning right: {} -> {}", p0, p2);
    }

    /// Build a 2-row module: row 0 triggers the note, row 1 carries the
    /// combo effect. Returns a tester positioned at row 1 tick 0 so the
    /// caller can step ticks and observe vol-slide behavior.
    fn make_combo(song_type: SongType, combo_eff: u8, combo_param: u8) -> SongTester {
        let mut builder = MockSongBuilder::new(song_type, 1);
        builder.add_empty_pattern(2);
        // Row 0: just trigger the note. Volume comes from sample.volume
        // (default 64 for the mock instrument).
        builder.set_pattern_row(0, 0, 0, Pattern {
            note: 49, instrument: 1, volume: 255, effect: 0, effect_param: 0,
        });
        // Row 1: the combo effect.
        builder.set_pattern_row(0, 1, 0, Pattern {
            note: 0, instrument: 0, volume: 255,
            effect: combo_eff, effect_param: combo_param,
        });
        let mut tester = builder.get_tester();
        tester.step_to_row(1);
        tester
    }

    #[test]
    fn test_combo_porta_plus_volslide_runs_in_all_formats() {
        // PortaPlusVolSlide: XM 5, MOD 5, S3M 12 (L), IT 0xC. Verify
        // each format dispatches the combo, runs the vol-slide on
        // subsequent ticks, and lowers the voice volume.
        for st in [SongType::XM, SongType::MOD, SongType::S3M, SongType::IT] {
            let combo_eff: u8 = match st {
                SongType::XM | SongType::MOD => 0x05,
                SongType::S3M                => 12,
                SongType::IT                 => 0x0C,
                _                            => continue,
            };
            let mut tester = make_combo(st, combo_eff, 0x02);
            let v_before = tester.song.voices[0].volume.volume;
            // tick 0: first-tick gate — slide skipped.
            tester.tick();
            // ticks 1..=3: slide fires three times (-2 each in XM/MOD;
            // -2 each in IT/S3M because Dxy with x=0 / y=2 means down-by-y).
            for _ in 0..3 { tester.tick(); }
            let v_after = tester.song.voices[0].volume.volume;
            assert!(v_after < v_before,
                    "{:?} combo should slide volume down (was {}, now {})",
                    st, v_before, v_after);
        }
    }

    #[test]
    fn test_xm_e3_glissando() {
        // E3y enables glissando (porta-to-note snaps to semitones) when y!=0.
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(2);
        builder.add_pattern_row(0, 0, 48, 1, 255, 0x0E, 0x31); // E31 -> glissando on
        builder.add_pattern_row(0, 1, 0, 0,  255, 0x0E, 0x30); // E30 -> glissando off
        let mut tester = builder.get_tester();

        tester.tick();
        assert!(tester.song.channels[0].glissando, "E31 should enable glissando");

        tester.step_to_row(1);
        tester.tick();
        assert!(!tester.song.channels[0].glissando, "E30 should disable glissando");
    }

    #[test]
    fn test_xm_e4_vibrato_waveform() {
        // E4y selects vibrato waveform (low 2 bits) and retrig flag (bit 2).
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(1);
        // E42 -> waveform 2 (square), retrig=true (bit 4 == 0).
        builder.add_pattern_row(0, 0, 48, 1, 255, 0x0E, 0x42);
        let mut tester = builder.get_tester();
        tester.tick();
        assert_eq!(tester.song.channels[0].vibrato_waveform, 2);
        assert!(tester.song.channels[0].vibrato_retrig);
    }

    #[test]
    fn test_xm_e5_set_finetune() {
        // E5y sets finetune to (y << 4) - 128, mapping nibble 0..15 to
        // -128..112 in i8.
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(1);
        builder.add_pattern_row(0, 0, 48, 1, 255, 0x0E, 0x58);  // E58 -> 8<<4 - 128 = 0
        let mut tester = builder.get_tester();
        tester.tick();
        assert_eq!(tester.song.channels[0].note.finetune, 0);
    }

    #[test]
    fn test_xm_ecx_note_cut_at_tick() {
        // ECx silences the voice when tick == x.
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(1);
        builder.add_pattern_row(0, 0, 48, 1, 255, 0x0E, 0xC2); // EC2 -> cut at tick 2
        let mut tester = builder.get_tester();
        tester.tick(); // tick 0
        assert!(tester.song.voices[0].on);
        tester.tick(); // tick 1
        assert!(tester.song.voices[0].on);
        tester.tick(); // tick 2 -> cut
        assert!(!tester.song.voices[0].on);
    }

    #[test]
    fn test_combo_vibrato_plus_volslide_runs_in_all_formats() {
        // VibratoPlusVolSlide: XM 6, MOD 6, S3M 11 (K), IT 0xB.
        for st in [SongType::XM, SongType::MOD, SongType::S3M, SongType::IT] {
            let combo_eff: u8 = match st {
                SongType::XM | SongType::MOD => 0x06,
                SongType::S3M                => 11,
                SongType::IT                 => 0x0B,
                _                            => continue,
            };
            let mut tester = make_combo(st, combo_eff, 0x02);
            let v_before = tester.song.voices[0].volume.volume;
            tester.tick();
            for _ in 0..3 { tester.tick(); }
            let v_after = tester.song.voices[0].volume.volume;
            assert!(v_after < v_before,
                    "{:?} combo should slide volume down (was {}, now {})",
                    st, v_before, v_after);
        }
    }

    #[test]
    fn test_xm_note_off_only_keys_off_own_voice() {
        // channel.voice_idx can go stale when the voice pool's allocator
        // hands the same slot to a different channel after the original
        // channel's voice has been silenced. A subsequent NoteAction::Off
        // (note=97) row must check the voice's current channel_idx before
        // calling key_off, or it will mute someone else's voice.
        let mut builder = MockSongBuilder::new(SongType::XM, 2);
        builder.add_empty_pattern(4);
        // Ch0: trigger a long note.
        builder.set_pattern_row(0, 0, 0, Pattern {
            note: 49, instrument: 1, volume: 255, effect: 0, effect_param: 0,
        });
        // Ch1: trigger then key-off later.
        builder.set_pattern_row(0, 0, 1, Pattern {
            note: 49, instrument: 1, volume: 255, effect: 0, effect_param: 0,
        });
        builder.set_pattern_row(0, 1, 1, Pattern {
            note: 97, instrument: 0, volume: 255, effect: 0, effect_param: 0,
        });

        let mut tester = builder.get_tester();
        tester.tick(); // row 0: both channels trigger
        let ch0_voice_idx = tester.song.channels[0].voice_idx.expect("ch0 voice");
        let ch1_voice_idx = tester.song.channels[1].voice_idx.expect("ch1 voice");
        assert_ne!(ch0_voice_idx, ch1_voice_idx);
        // Manually corrupt ch1.voice_idx to point at ch0's voice slot (simulates
        // the stale-slot scenario that triggered the original bug).
        tester.song.channels[1].voice_idx = Some(ch0_voice_idx);

        tester.step_to_row(1);
        tester.tick(); // row 1: ch1 NoteAction::Off

        // ch0's voice must NOT have been keyed off by ch1's note=97.
        let ch0_voice = &tester.song.voices[ch0_voice_idx];
        assert!(ch0_voice.sustained,
            "ch0 voice must remain sustained when ch1's key-off targets the wrong slot");
    }

    #[test]
    fn test_xm_porta_to_note_uses_playing_instrument_finetune() {
        // FT2 preparePortamento: target period uses the channel-state
        // relative_note / finetune captured at the last trigger — NOT the
        // new instrument byte's sample. Refactor previously read from
        // channel.last_instrument.
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.instruments[1].samples[0].finetune = 0;
        // Add a second instrument with a different finetune.
        let mut inst2 = builder.instruments[1].clone();
        inst2.samples[0].finetune = 64;
        builder.instruments.push(inst2);
        builder.add_empty_pattern(4);
        // Row 0: trigger A-4 with instrument 1 (finetune 0).
        builder.set_pattern_row(0, 0, 0, Pattern {
            note: 49, instrument: 1, volume: 255, effect: 0x00, effect_param: 0x00,
        });
        // Row 1: portamento to A#4 with instrument 2 (finetune 64).
        builder.set_pattern_row(0, 1, 0, Pattern {
            note: 50, instrument: 2, volume: 255, effect: 0x03, effect_param: 0xFF,
        });
        let mut tester = builder.get_tester();
        tester.step_to_row(1);
        tester.tick();
        // Target period should reflect inst-1's finetune (0), not inst-2's (64).
        // The exact value comes from note_to_period — verify it differs from
        // what inst-2 would produce.
        let target_with_inst1_finetune = tester.song.channels[0].porta_to_note.target_note.period;
        // What inst-2's finetune would produce:
        let target_with_inst2_finetune = tester.song.channels[0].note.note_to_period(50, 64, tester.song.frequency_tables);
        assert_ne!(target_with_inst1_finetune, target_with_inst2_finetune,
            "porta target must use the playing voice's finetune, not the new instrument's");
    }

    #[test]
    fn test_xm_tremor_count_reset_on_trigger() {
        // FT2: a fresh note trigger zeroes the channel's tremor counter so
        // a subsequent Txy starts in a known state, not mid-cycle.
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(3);
        // Row 0: trigger + tremor 0x42 (on 4 ticks, off 2 ticks). 5 ticks
        // are processed by step_row, leaving tremor_count > 0.
        builder.set_pattern_row(0, 0, 0, Pattern {
            note: 49, instrument: 1, volume: 255, effect: 0x1D, effect_param: 0x42,
        });
        // Row 1: fresh note trigger; tremor_count must reset.
        builder.set_pattern_row(0, 1, 0, Pattern {
            note: 50, instrument: 1, volume: 255, effect: 0x00, effect_param: 0x00,
        });
        let mut tester = builder.get_tester();
        tester.step_to_row(1);
        // The trigger fires at row 1 tick 0; tremor_count should be cleared.
        tester.tick();
        assert_eq!(tester.song.channels[0].tremor_count, 0,
            "tremor_count must reset on note trigger, got {}",
            tester.song.channels[0].tremor_count);
    }

    #[test]
    fn test_xm_vol_col_vibrato_arms_speed_vs_depth() {
        // FT2 vol col: 0xA0-AF sets vibrato speed (tick-zero handler);
        // 0xB0-BF sets vibrato depth and applies vibrato (tick-nonzero).
        // Refactor previously had these swapped — A wrote depth, B wrote speed.
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(4);
        // Row 0: trigger note; vol col 0xA4 = set vibrato speed = 4.
        builder.set_pattern_row(0, 0, 0, Pattern {
            note: 49, instrument: 1, volume: 0xA4, effect: 0, effect_param: 0,
        });
        let mut tester = builder.get_tester();
        tester.tick();
        // Speed is stored ×4 internally for sub-tick resolution.
        assert_eq!(tester.song.voices[0].vibrato_state.speed, 16,
            "vol col 0xA4 should set vibrato speed to 4 (stored as 16)");

        // Row 1: vol col 0xB6 = vibrato with depth=6.
        let mut builder2 = MockSongBuilder::new(SongType::XM, 1);
        builder2.add_empty_pattern(4);
        builder2.set_pattern_row(0, 0, 0, Pattern {
            note: 49, instrument: 1, volume: 0xA4, effect: 0, effect_param: 0,
        });
        builder2.set_pattern_row(0, 1, 0, Pattern {
            note: 0, instrument: 0, volume: 0xB6, effect: 0, effect_param: 0,
        });
        let mut tester2 = builder2.get_tester();
        tester2.step_to_row(1);
        tester2.tick();
        assert_eq!(tester2.song.voices[0].vibrato_state.depth, 6,
            "vol col 0xB6 should set vibrato depth to 6");
    }

    #[test]
    fn test_xm_vol_col_pan_slide_direction() {
        // FT2: vol col 0xD = pan-slide LEFT (pan decreases), 0xE = pan-slide RIGHT.
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(4);
        // Row 0: trigger with pan=128 (center).
        builder.set_pattern_row(0, 0, 0, Pattern {
            note: 49, instrument: 1, volume: 0xC8, effect: 0, effect_param: 0,
        });
        // Row 1: vol col 0xD2 = pan slide LEFT by 2 per tick.
        builder.set_pattern_row(0, 1, 0, Pattern {
            note: 0, instrument: 0, volume: 0xD2, effect: 0, effect_param: 0,
        });
        let mut tester = builder.get_tester();
        tester.step_to_row(1);
        let pan_start = tester.song.voices[0].panning.panning;
        // Slide fires on non-zero ticks; after a few ticks, pan should be lower.
        for _ in 0..4 { tester.tick(); }
        let pan_after_left = tester.song.voices[0].panning.panning;
        assert!(pan_after_left < pan_start,
            "vol col 0xD2 should slide pan LEFT (decrease), pan_start={} pan_after={}",
            pan_start, pan_after_left);
    }

    #[test]
    fn test_xm_instrument_only_row_refreshes_envelopes_and_volume() {
        // FT2: an XM row with an instrument byte but no note triggers
        // resetVolumes + triggerInstrument — sample default volume is
        // re-applied and envelope state rewinds to frame 0.
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.instruments[1].volume_envelope.on = true;
        builder.instruments[1].volume_envelope.points[0].frame = 0;
        builder.instruments[1].volume_envelope.points[0].value = 64;
        builder.instruments[1].volume_envelope.points[1].frame = 40;
        builder.instruments[1].volume_envelope.points[1].value = 32;
        builder.instruments[1].volume_envelope.size = 2;
        builder.add_empty_pattern(3);
        // Row 0: trigger; sample default vol is 64.
        builder.set_pattern_row(0, 0, 0, Pattern {
            note: 49, instrument: 1, volume: 255, effect: 0, effect_param: 0,
        });
        // Row 1: vol col forces vol down to 4 (so we can observe refresh).
        builder.set_pattern_row(0, 1, 0, Pattern {
            note: 0, instrument: 0, volume: 0x10 + 4, effect: 0, effect_param: 0,
        });
        // Row 2: instrument-only refresh — should reset vol back to 64.
        builder.set_pattern_row(0, 2, 0, Pattern {
            note: 0, instrument: 1, volume: 255, effect: 0, effect_param: 0,
        });
        let mut tester = builder.get_tester();
        tester.step_to_row(1);
        tester.tick();
        assert_eq!(tester.song.voices[0].volume.volume, 4,
            "row 1 should set vol to 4");
        tester.step_to_row(2);
        tester.tick();
        assert_eq!(tester.song.voices[0].volume.volume, 64,
            "instrument-only row should reset sample default vol");
        assert!(tester.song.voices[0].volume_envelope_state.frame < 3,
            "instrument-only row should rewind env, got frame {}",
            tester.song.voices[0].volume_envelope_state.frame);
    }

    #[test]
    fn test_xm_vol_col_vol_slide_skips_tick_zero() {
        // FT2 vol col vol slides (0x60-0x7F) are tick-non-zero handlers —
        // they don't fire on the first tick of the row. mview.xm ord=16
        // ch7 hit this: a sustained 1.42x loudness offset accumulated
        // because we were sliding on tick 0 as well.
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(4);
        // Row 0: trigger note at volume 32.
        builder.set_pattern_row(0, 0, 0, Pattern {
            note: 49, instrument: 1, volume: 0x10 + 32, effect: 0, effect_param: 0,
        });
        // Row 1: vol slide up by 2 in vol column (0x72). With speed 6
        // and tick-zero skipped, vol grows by 2 × 5 = +10 across the row.
        builder.set_pattern_row(0, 1, 0, Pattern {
            note: 0, instrument: 0, volume: 0x72, effect: 0, effect_param: 0,
        });

        let mut tester = builder.get_tester();
        tester.step_to_row(1);
        // Tick 0 of row 1: slide must NOT have fired.
        tester.tick();
        let v_tick0 = tester.song.voices[0].volume.volume;
        assert_eq!(v_tick0, 32,
            "vol col vol slide must not fire on tick 0 of the row, got {}", v_tick0);
        // Subsequent ticks slide by 2 each.
        tester.tick();
        assert_eq!(tester.song.voices[0].volume.volume, 34);
        tester.tick();
        assert_eq!(tester.song.voices[0].volume.volume, 36);
    }

    #[test]
    fn test_xm_note_delay_instrument_only_retriggers_last_note() {
        // FT2: an EDx delayed row with note=0 retriggers using the
        // channel's last played note.
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(4);
        // Row 0: C-4 trigger (last_played_note = 48).
        builder.set_pattern_row(0, 0, 0, Pattern {
            note: 48, instrument: 1, volume: 255, effect: 0x00, effect_param: 0x00,
        });
        // Row 1: no note, instrument only, delayed by 2 ticks.
        builder.set_pattern_row(0, 1, 0, Pattern {
            note: 0, instrument: 1, volume: 255, effect: 0x0E, effect_param: 0xD2,
        });
        let mut tester = builder.get_tester();
        tester.step_to_row(1);
        let pos_before = tester.song.voices[0].sample_position;
        // Walk ticks until the delay (2) fires.
        for _ in 0..3 { tester.tick(); }
        let pos_after = tester.song.voices[0].sample_position;
        assert_eq!(pos_after, 4.0,
            "delayed instrument-only row should retrigger (sample_position reset to 4.0). before={} after={}",
            pos_before, pos_after);
        assert_eq!(tester.song.voices[0].last_played_note, 48);
    }

    #[test]
    fn test_xm_porta_to_note_no_glissando_no_snap() {
        // Without glissando, porta_to_note must NOT snap to semitones —
        // the live period should land at an intermediate value.
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(8);
        // Row 0: C-4 trigger.
        builder.set_pattern_row(0, 0, 0, Pattern {
            note: 49, instrument: 1, volume: 255, effect: 0x00, effect_param: 0x00,
        });
        // Row 1: portamento to C#4 at slow speed; expect non-semitone period.
        builder.set_pattern_row(0, 1, 0, Pattern {
            note: 50, instrument: 0, volume: 255, effect: 0x03, effect_param: 0x01,
        });
        let mut tester = builder.get_tester();
        tester.step_to_row(1);
        tester.tick(); // tick 0: porta target loaded, no slide yet
        tester.tick(); // tick 1: one slide step
        let period = tester.song.channels[0].note.period as i32;
        // Without glissando, period should not be an exact multiple of 64.
        assert_ne!(period % 64, 0,
            "expected non-semitone period during porta without glissando, got {}", period);
    }

    #[test]
    fn test_xm_period_shift_cleared_on_note_trigger() {
        // After an arpeggio row leaves a non-zero period_shift, the next
        // row's new note must start from a clean shift (FT2 resets
        // outPeriod = realPeriod on trigger).
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(8);
        builder.set_pattern_row(0, 0, 0, Pattern {
            note: 49, instrument: 1, volume: 255, effect: 0x00, effect_param: 0x37,
        });
        builder.set_pattern_row(0, 1, 0, Pattern {
            note: 51, instrument: 1, volume: 255, effect: 0x00, effect_param: 0x00,
        });
        let mut tester = builder.get_tester();
        tester.step_to_row(1);
        tester.tick();
        assert_eq!(tester.song.channels[0].period_shift, 0);
        let expected_hz = 8363.0 * 2.0f32.powf(2.0 / 12.0);
        tester.assert_pitch_near(0, expected_hz, 5.0);
    }

    #[test]
    fn test_xm_period_shift_persists_across_non_arpeggio_effect() {
        // FT2 only resets outPeriod on arpeggio tick%3==0 or note trigger.
        // A non-arpeggio effect must NOT clear the leftover shift.
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(8);
        builder.set_pattern_row(0, 0, 0, Pattern {
            note: 49, instrument: 1, volume: 255, effect: 0x00, effect_param: 0x07,
        });
        builder.set_pattern_row(0, 1, 0, Pattern {
            note: 0, instrument: 0, volume: 255, effect: 0x08, effect_param: 0x80,
        });
        let mut tester = builder.get_tester();
        tester.step_to_row(1);
        let shift_before = tester.song.channels[0].period_shift;
        assert_ne!(shift_before, 0,
            "expected arpeggio to leave a non-zero period_shift");
        tester.tick();
        assert_eq!(tester.song.channels[0].period_shift, shift_before,
            "non-arpeggio effect must not clear period_shift");
    }
}
