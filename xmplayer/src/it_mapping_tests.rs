#[cfg(test)]
mod tests {
    use crate::test_utils::MockSongBuilder;
    use crate::module_reader::SongType;
    use crate::pattern::Pattern;

    #[test]
    fn test_it_keyboard_mapping() {
        let mut builder = MockSongBuilder::new(SongType::IT, 2);
        builder.add_empty_pattern(64);
        
        // Instrument 1: Map C-5 (61 relative to 1) to C-6 (73 relative to 1)
        builder.add_instrument("MappingTest", vec![0.5; 1000]);
        // IT C-5 speed 8363 means Note 61 should play at 8363Hz.
        // In our engine, 8363Hz is Note 49. So relative_note must be -12.
        builder.instruments[1].samples[0].relative_note = -12;
        // Pattern note 61 (C-5) maps to note 60 in instrument mapping (0-119).
        // Let's map instrument mapping index 60 to note 72 (C-6).
        builder.instruments[1].sample_indexes[60] = (72, 1);
        
        // Row 0: Play C-5
        builder.set_pattern_row(0, 0, 0, Pattern { note: 61, instrument: 1, volume: 64, ..Pattern::new() });
        
        let mut tester = builder.get_tester();
        
        // Tick 0: New note
        tester.tick();
        
        let chan = &tester.song.channels[0];
        // The pattern note should be 61, but the "real" note derived from instrument should be 72 + sample.relative_note (0) = 72.
        assert_eq!(chan.last_played_note, 61);
        assert_eq!(tester.song.voices[chan.voice_idx.unwrap()].last_played_note, 73); // mapped_note + 1
        
        // Frequency check: C-6 is double C-5
        let voice = &tester.song.voices[chan.voice_idx.unwrap()];
        let c5_freq = 8363.0; // standard C-5
        let expected_freq = c5_freq * 2.0; 
        
        assert!((voice.frequency - expected_freq).abs() < 1.0, "Frequency should be roughly {}, got {}", expected_freq, voice.frequency);
    }

    #[test]
    fn test_it_sample_panning() {
        let mut builder = MockSongBuilder::new(SongType::IT, 2);
        builder.add_empty_pattern(64);
        
        // Sample with panning 16 (L) -> 16/64 = 0.25 -> 255 * 0.25 = 63.
        builder.add_instrument("PanTest", vec![0.5; 1000]);
        builder.instruments[1].samples[0].relative_note = -12;
        builder.instruments[1].samples[0].panning = 63; // scaled from 16
        
        // Row 0: Play Note
        builder.set_pattern_row(0, 0, 0, Pattern { note: 61, instrument: 1, volume: 64, ..Pattern::new() });
        
        let mut tester = builder.get_tester();
        tester.tick();
        
        let chan = &tester.song.channels[0];
        let voice = &tester.song.voices[chan.voice_idx.unwrap()];
        
        assert_eq!(voice.panning.panning, 63);
    }
}
