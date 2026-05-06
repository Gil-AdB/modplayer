
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
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(1);
        // K04 -> Key Off when tick == 4. Default speed is 6 so we have
        // ticks 0..5 within the row.
        builder.add_pattern_row(0, 0, 48, 1, 255, 0x14, 0x04);

        let mut tester = builder.get_tester();

        for _ in 0..4 {
            tester.tick();
            assert!(tester.song.voices[0].on, "voice should still be on before tick 4 fires Kxx");
        }
        // Now we've processed ticks 0..3 and we're about to process tick 4.
        tester.tick();
        assert!(!tester.song.voices[0].on, "K04 should fire at tick 4");
    }

    #[test]
    fn test_xm_key_off_no_envelope() {
        // For an instrument without a volume envelope, Voice::key_off sets
        // voice.on = false immediately (no fade-out animation needed).
        // Note: fadeout_vol is left at its current value because voice.on=false
        // already silences playback - no further mute is required.
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(1);
        builder.add_pattern_row(0, 0, 48, 1, 255, 0x14, 0x03); // K03

        let mut tester = builder.get_tester();

        // Ticks 0..2 process before K03 fires.
        for _ in 0..3 {
            tester.tick();
            assert!(tester.song.voices[0].on);
        }
        // Tick 3 triggers Key Off (no envelope -> immediate cut).
        tester.tick();
        assert!(!tester.song.voices[0].on, "K03 should cut the voice at tick 3");
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
}
