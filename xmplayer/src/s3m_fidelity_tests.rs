#[cfg(test)]
mod tests {
    use crate::song::Song;
    use crate::test_utils::{MockSongBuilder, SongTester};
    use crate::module_reader::SongType;

    #[test]
    fn test_s3m_note_trigger() {
        let mut builder = MockSongBuilder::new(SongType::S3M, 1);
        builder.master_volume = 128 | 64; // Stereo, Master Vol 64 (1.0)
        builder.global_volume = 64;       // Global Vol 64 (1.0)
        
        // S3M Note C-4 is 49 in our engine. Volume 64.
        builder.add_pattern_row(0, 0, 49, 1, 64, 0, 0); // C-4, Inst 1, Vol 64
        
        let mut tester = SongTester::new(builder.build());
        
        // Tick 0: Note Trigger
        tester.tick();
        tester.assert_voice_on(0, true);
        // C-4 at 8363Hz (Standard) should have dU = 8363/48000 = 0.174229
        tester.assert_voice_du_near(0, 0.1742, 0.001);
        tester.assert_voice_volume_near(0, 1.0, 0.01);
    }

    #[test]
    fn test_s3m_volume_slide_d() {
        let mut builder = MockSongBuilder::new(SongType::S3M, 1);
        builder.master_volume = 128 | 64;
        builder.global_volume = 64;
        
        builder.add_pattern_row(0, 0, 49, 1, 64, 0, 0);   // Row 0: C-4, Vol 64
        builder.add_pattern_row(0, 1, 0, 0, 255, 4, 0x02); // Row 1: Slide Down 2 (D02)
        builder.add_pattern_row(0, 2, 0, 0, 255, 4, 0x20); // Row 2: Slide Up 2 (D20)
        
        let mut tester = SongTester::new(builder.build());
        
        tester.step_to_row(1); // At Row 1, Tick 0. Checked result of Row 0.
        tester.assert_voice_volume_near(0, 1.0, 0.01);
        
        tester.tick(); // Processes Row 1 Tick 0. Moves to Tick 1.
        tester.assert_voice_volume_near(0, 1.0, 0.01); // No slide on first tick
        
        tester.tick(); // Processes Row 1 Tick 1. Moves to Tick 2.
        tester.assert_voice_volume_near(0, 62.0/64.0, 0.01);
        
        tester.step_to_row(2); // At Row 2, Tick 0. Checked result of Row 1.
        // Row 1 slide ticks: T1, T2, T3, T4, T5 (5 ticks). 64 - 2*5 = 54.
        tester.assert_voice_volume_near(0, 54.0/64.0, 0.01);
        
        tester.tick(); // Processes Row 2 Tick 0.
        tester.tick(); // Processes Row 2 Tick 1.
        tester.assert_voice_volume_near(0, 56.0/64.0, 0.01);
    }

    #[test]
    fn test_s3m_effect_memory_d() {
        let mut builder = MockSongBuilder::new(SongType::S3M, 1);
        builder.master_volume = 128 | 64;
        builder.global_volume = 64;
        builder.add_pattern_row(0, 0, 49, 1, 64, 4, 0x02); // D02: VolSlide Down 2
        builder.add_pattern_row(0, 1, 0, 0, 255, 4, 0x00); // D00: Memory (should be D02)
        
        let mut tester = SongTester::new(builder.build());
        
        tester.step_to_row(1); // End of Row 0
        tester.assert_voice_volume_near(0, 54.0/64.0, 0.01);
        
        tester.step_to_row(2); // End of Row 1
        tester.assert_voice_volume_near(0, 44.0/64.0, 0.01);
    }

    #[test]
    fn test_s3m_cross_effect_memory_l() {
        let mut builder = MockSongBuilder::new(SongType::S3M, 1);
        builder.master_volume = 128 | 64;
        builder.global_volume = 64;
        builder.add_pattern_row(0, 0, 49, 1, 64, 4, 0x20); // D20: VolSlide Up 2
        builder.add_pattern_row(0, 1, 61, 0, 255, 7, 10);  // G0A: Porta to C-5, Speed 10
        builder.add_pattern_row(0, 2, 0, 0, 255, 12, 0x00); // L00: Porta + VolSlide (Memory)
        
        let mut tester = SongTester::new(builder.build());
        
        tester.step_to_row(1); // Row 0
        tester.assert_voice_volume_near(0, 1.0, 0.01); // Clamped at 64
        
        tester.step_to_row(2); // Row 1 (G0A - no vol slide)
        tester.assert_voice_volume_near(0, 1.0, 0.01);
        
        // Let's test slide down to be sure it uses D memory
        builder = MockSongBuilder::new(SongType::S3M, 1);
        builder.master_volume = 128 | 64;
        builder.global_volume = 64;
        builder.add_pattern_row(0, 0, 49, 1, 64, 4, 0x02); // D02: VolSlide Down 2
        builder.add_pattern_row(0, 1, 0, 0, 255, 12, 0x00); // L00: Porta + VolSlide Memory
        tester = SongTester::new(builder.build());
        
        tester.step_to_row(1); // Row 0
        tester.assert_voice_volume_near(0, 54.0/64.0, 0.01);
        tester.step_to_row(2); // Row 1
        tester.assert_voice_volume_near(0, 44.0/64.0, 0.01);
    }

    #[test]
    fn test_s3m_vibrato_memory_h_u() {
        let mut builder = MockSongBuilder::new(SongType::S3M, 1);
        builder.master_volume = 128 | 64;
        builder.global_volume = 64;
        builder.add_pattern_row(0, 0, 49, 1, 64, 8, 0x42); // H42: Vibrato
        builder.add_pattern_row(0, 1, 0, 0, 255, 21, 0x00); // U00: Fine Vibrato Memory (uses H42)
        
        let mut tester = SongTester::new(builder.build());
        
        tester.tick(); // Row 0 Tick 0
        tester.tick(); // Row 0 Tick 1
        let freq1 = tester.get_voice_du(0);
        
        tester.step_to_row(1);
        tester.tick(); // Row 1 Tick 0
        tester.tick(); // Row 1 Tick 1
        let freq2 = tester.get_voice_du(0);
        
        assert!(freq1 != 0.1742);
        assert!(freq2 != 0.1742);
    }
    #[test]
    fn test_s3m_effect_memory_i() {
        let mut builder = MockSongBuilder::new(SongType::S3M, 1);
        builder.add_pattern_row(0, 0, 49, 1, 64, 9, 0x21); // I21 (Tremor 2 on, 1 off)
        builder.add_pattern_row(0, 1, 0, 0, 255, 9, 0x00); // I00 (Memory)
        let mut tester = SongTester::new(builder.build());
        
        tester.step_row(); // Row 0: Tremor starts
        // We just verify it doesn't crash for now. 
        // Real tremor verification requires checking volume on every tick.
        tester.step_to_row(2); 
        tester.assert_voice_on(0, true);
    }

    #[test]
    fn test_s3m_effect_memory_p() {
        let mut builder = MockSongBuilder::new(SongType::S3M, 1);
        builder.add_pattern_row(0, 0, 49, 1, 64, 16, 0xF1); // P F1 (Pan slide left)
        builder.add_pattern_row(0, 1, 0, 0, 255, 16, 0x00); // P 00 (Memory)
        let mut tester = SongTester::new(builder.build());
        
        tester.step_row(); // Row 0
        let pan0 = tester.song.voices[0].panning.panning;
        
        tester.step_row(); // Row 1 (Memory)
        let pan1 = tester.song.voices[0].panning.panning;
        
        assert!(pan1 < pan0, "Panning should have slid further left in row 1");
    }

    #[test]
    fn test_s3m_effect_memory_r() {
        let mut builder = MockSongBuilder::new(SongType::S3M, 1);
        builder.add_pattern_row(0, 0, 49, 1, 64, 18, 0x42); // R 42 (Tremolo)
        builder.add_pattern_row(0, 1, 0, 0, 255, 18, 0x00); // R 00 (Memory)
        let mut tester = SongTester::new(builder.build());
        
        tester.step_to_row(2);
        tester.assert_voice_on(0, true);
    }

    #[test]
    fn test_s3m_effect_memory_w() {
        let mut builder = MockSongBuilder::new(SongType::S3M, 1);
        builder.global_volume = 64;
        builder.add_pattern_row(0, 0, 49, 1, 64, 23, 0x02); // W 02 (Global Vol slide down)
        builder.add_pattern_row(0, 1, 0, 0, 255, 23, 0x00); // W 00 (Memory)
        let mut tester = SongTester::new(builder.build());
        
        tester.step_row(); // Row 0
        let vol0 = tester.song.global_volume.volume;
        
        tester.step_row(); // Row 1 (Memory)
        let vol1 = tester.song.global_volume.volume;
        
        assert!(vol1 < vol0, "Global volume should have slid further down in row 1");
    }

    #[test]
    fn test_s3m_special_s8x() {
        let mut builder = MockSongBuilder::new(SongType::S3M, 1);
        builder.add_pattern_row(0, 0, 49, 1, 64, 19, 0x88); // S88 (Panning center-ish: 8*17 = 136)
        let mut tester = SongTester::new(builder.build());
        
        tester.step_row();
        assert_eq!(tester.song.voices[0].panning.panning, 136);
    }

    #[test]
    fn test_s3m_special_scx() {
        let mut builder = MockSongBuilder::new(SongType::S3M, 1);
        builder.add_pattern_row(0, 0, 49, 1, 64, 19, 0xC2); // SC2 (Note cut at tick 2)
        let mut tester = SongTester::new(builder.build());
        
        tester.tick(); // Tick 0
        tester.assert_voice_on(0, true);
        tester.tick(); // Tick 1
        tester.assert_voice_on(0, true);
        tester.tick(); // Tick 2: CUT
        assert!(!tester.song.voices[0].on, "Voice should be cut at tick 2");
    }
}
