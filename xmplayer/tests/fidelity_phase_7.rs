#[cfg(test)]
mod tests {
    use xmplayer::test_utils::{MockSongBuilder};
    use xmplayer::module_reader::{SongType};
    use xmplayer::pattern::Pattern;
    use xmplayer::envelope::{Envelope, EnvelopePoint};

    #[test]
    fn test_volume_envelope_basic() {
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(64);
        
        let mut env = Envelope::new();
        env.on = true;
        env.size = 3;
        env.points[0] = EnvelopePoint { frame: 0, value: 0 };
        env.points[1] = EnvelopePoint { frame: 10, value: 64 };
        env.points[2] = EnvelopePoint { frame: 20, value: 32 };
        
        builder.instruments[1].volume_envelope = env;
        
        let mut tester = builder.get_tester();
        tester.song.song_data.patterns[0].rows[0].channels[0] = Pattern {
            note: 48, instrument: 1, volume: 255, effect: 0, effect_param: 0,
        };
        
        tester.tick();
        assert_eq!(tester.song.voices[0].volume.envelope_vol, 0);
        
        for _ in 0..5 { tester.tick(); }
        assert!(tester.song.voices[0].volume.envelope_vol > 0 && tester.song.voices[0].volume.envelope_vol < 64 * 256);
        
        for _ in 0..5 { tester.tick(); }
        assert_eq!(tester.song.voices[0].volume.envelope_vol, 64 * 256);
    }

    #[test]
    fn test_envelope_sustain() {
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(64);
        
        let mut env = Envelope::new();
        env.on = true;
        env.size = 2;
        env.sustain = true;
        env.sustain_point = 1;
        env.points[0] = EnvelopePoint { frame: 0, value: 0 };
        env.points[1] = EnvelopePoint { frame: 5, value: 64 };
        
        builder.instruments[1].volume_envelope = env;
        
        let mut tester = builder.get_tester();
        tester.song.song_data.patterns[0].rows[0].channels[0] = Pattern {
            note: 48, instrument: 1, volume: 255, effect: 0, effect_param: 0,
        };
        
        for _ in 0..10 { tester.tick(); }
        assert_eq!(tester.song.voices[0].volume.envelope_vol, 64 * 256);
        assert!(tester.song.voices[0].sustained);
    }

    #[test]
    fn test_envelope_loop() {
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(64);
        
        let mut env = Envelope::new();
        env.on = true;
        env.size = 3;
        env.has_loop = true;
        env.loop_start_point = 1;
        env.loop_end_point = 2;
        env.points[0] = EnvelopePoint { frame: 0, value: 0 };
        env.points[1] = EnvelopePoint { frame: 10, value: 64 };
        env.points[2] = EnvelopePoint { frame: 20, value: 32 };
        
        builder.instruments[1].volume_envelope = env;
        
        let mut tester = builder.get_tester();
        tester.song.song_data.patterns[0].rows[0].channels[0] = Pattern {
            note: 48, instrument: 1, volume: 255, effect: 0, effect_param: 0,
        };
        
        for _ in 0..11 { tester.tick(); }
        assert_eq!(tester.song.voices[0].volume.envelope_vol, 64 * 256);
        
        for _ in 0..10 { tester.tick(); }
        assert_eq!(tester.song.voices[0].volume.envelope_vol, 32 * 256);
        
        tester.tick();
        assert_eq!(tester.song.voices[0].volume_envelope_state.frame, 11); 
    }

    #[test]
    fn test_sustain_release_fadeout() {
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(64);
        
        builder.instruments[1].volume_fadeout = 1024; // Slow fadeout
        
        let mut env = Envelope::new();
        env.on = true;
        env.size = 2;
        env.sustain = true;
        env.sustain_point = 1;
        env.points[0] = EnvelopePoint { frame: 0, value: 64 };
        env.points[1] = EnvelopePoint { frame: 10, value: 64 };
        
        builder.instruments[1].volume_envelope = env;
        
        let mut tester = builder.get_tester();
        tester.song.song_data.patterns[0].rows[0].channels[0] = Pattern {
            note: 48, instrument: 1, volume: 255, effect: 0, effect_param: 0,
        };
        
        for _ in 0..15 { tester.tick(); }
        assert_eq!(tester.song.voices[0].volume.fadeout_vol, 65536);
        
        // Key-OFF (simulated)
        tester.song.voices[0].sustained = false;
        
        tester.tick();
        assert!(tester.song.voices[0].volume.fadeout_vol < 65536, "Fadeout should start after sustain release");
    }

    #[test]
    fn test_sticky_panning_envelope() {
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(64);
        
        let mut env = Envelope::new();
        env.on = true;
        env.size = 2;
        env.sustain = true;
        env.sustain_point = 1;
        env.points[0] = EnvelopePoint { frame: 0, value: 0 };
        env.points[1] = EnvelopePoint { frame: 10, value: 64 }; // Max right
        
        builder.instruments[1].panning_envelope = env;
        
        let mut tester = builder.get_tester();
        tester.song.song_data.patterns[0].rows[0].channels[0] = Pattern {
            note: 48, instrument: 1, volume: 255, effect: 0, effect_param: 0,
        };
        
        for _ in 0..15 { tester.tick(); }
        assert!(tester.song.voices[0].panning_envelope_state.sustained);
        
        // Key-OFF
        tester.song.voices[0].sustained = false;
        
        // Sticky sustain for panning means it stays at sustain point even after key-off
        // unless it has a loop.
        tester.tick();
        assert!(tester.song.voices[0].panning_envelope_state.sustained, "Panning envelope should be sticky");
    }

    #[test]
    fn test_auto_vibrato() {
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(64);
        
        builder.instruments[1].vibrato_envelope.vibrato_depth = 4;
        builder.instruments[1].vibrato_envelope.vibrato_rate = 16;
        builder.instruments[1].vibrato_envelope.vibrato_sweep = 0; 
        builder.instruments[1].vibrato_envelope.vibrato_type = 0; 
        
        let mut tester = builder.get_tester();
        tester.song.song_data.patterns[0].rows[0].channels[0] = Pattern {
            note: 48, instrument: 1, volume: 255, effect: 0, effect_param: 0,
        };
        
        tester.tick();
        let f0 = tester.song.voices[0].frequency + tester.song.voices[0].frequency_shift;
        
        tester.tick();
        let f1 = tester.song.voices[0].frequency + tester.song.voices[0].frequency_shift;
        assert_ne!(f0, f1, "Auto vibrato should shift frequency");
    }

    #[test]
    fn test_auto_vibrato_sweep() {
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(64);
        
        builder.instruments[1].vibrato_envelope.vibrato_depth = 16;
        builder.instruments[1].vibrato_envelope.vibrato_rate = 16;
        builder.instruments[1].vibrato_envelope.vibrato_sweep = 64; 
        builder.instruments[1].vibrato_envelope.vibrato_type = 0; 
        
        let mut tester = builder.get_tester();
        tester.song.song_data.patterns[0].rows[0].channels[0] = Pattern {
            note: 48, instrument: 1, volume: 255, effect: 0, effect_param: 0,
        };
        
        tester.tick();
        assert_eq!(tester.song.voices[0].frequency_shift, 0.0, "Vibrato should start at 0 amplitude during sweep");
        
        for _ in 0..10 { tester.tick(); }
        // If sweep is 64, it should take 256/64 = 4 ticks to reach full depth?
        // Wait, how does sweep work? 
        // In FT2, depth = current_sweep * max_depth / 256
        assert_ne!(tester.song.voices[0].frequency_shift, 0.0, "Vibrato should kick in after sweep progress");
    }
}
