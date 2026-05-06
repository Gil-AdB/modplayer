
#[cfg(test)]
mod tests {
    use crate::test_utils::MockSongBuilder;
    use crate::module_reader::SongType;

    #[test]
    fn test_xm_arpeggio_reset() {
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(2);
        
        // Row 0: Arpeggio 037
        builder.add_pattern_row(0, 0, 0, 0, 255, 0x0, 0x37);
        
        // Row 1: No effect
        // (already empty)

        let mut tester = builder.get_tester();
        
        // Process Row 0, Tick 0 (should reset shift)
        tester.tick();
        assert_eq!(tester.song.channels[0].period_shift, 0);

        // Process Row 0, Tick 1 (Arpeggio active)
        tester.tick();
        assert_eq!(tester.song.channels[0].period_shift, 3);

        // Process Row 0, Tick 2 (Arpeggio active)
        tester.tick();
        assert_eq!(tester.song.channels[0].period_shift, 7);

        // Process Row 0, Tick 3 (Arpeggio active, back to 0)
        tester.tick();
        assert_eq!(tester.song.channels[0].period_shift, 0);

        // Tick through remaining ticks of Row 0 (speed 6)
        tester.tick(); // tick 4 -> 5
        tester.tick(); // tick 5 -> row 1, tick 0

        // Process Row 1, Tick 0 (Should reset)
        tester.tick(); // row 1, tick 0 -> 1
        assert_eq!(tester.song.row, 1);
        assert_eq!(tester.song.tick, 1);
        assert_eq!(tester.song.channels[0].period_shift, 0);
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
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(1);
        
        // Speed is 6 (default).
        // K04 means Key Off when speed - tick == 4.
        // 6 - tick == 4 => tick == 2.
        builder.add_pattern_row(0, 0, 48, 1, 255, 0x14, 0x04);
        
        let mut tester = builder.get_tester();
        
        // Tick 0
        tester.tick();
        assert!(tester.song.voices[0].on);
        
        // Tick 1
        tester.tick();
        assert!(tester.song.voices[0].on);
        
        // Tick 2: Key Off!
        tester.tick();
        assert!(!tester.song.voices[0].on);
    }

    #[test]
    fn test_xm_key_off_no_envelope() {
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(1);
        
        // Use an instrument WITHOUT a volume envelope
        // Our MockSongBuilder currently creates instruments without envelopes by default
        builder.add_pattern_row(0, 0, 48, 1, 255, 0x14, 0x03); // K03 at speed 6 -> tick 3
        
        let mut tester = builder.get_tester();
        
        // Tick 0, 1, 2: Playing
        tester.tick();
        tester.tick();
        tester.tick();
        assert!(tester.song.voices[0].on);
        
        // Tick 3: Key Off! Should stop immediately
        tester.tick();
        assert!(!tester.song.voices[0].on);
        assert_eq!(tester.song.voices[0].volume.fadeout_vol, 0);
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
}
