#[cfg(test)]
mod tests {
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
        tester.step_to_row(2);
        tester.assert_voice_on(0, true);
    }

    #[test]
    fn test_s3m_tremor_silences_voice_during_off_phase() {
        // Regression: S3M effect 9 (I = Tremor) was parsed but never
        // dispatched. tremor() also overloaded channel.on which the volume
        // formula didn't observe. Now tremor sets channel.tremor_silenced
        // and the post-loop zeros output volume on those ticks.
        //
        // I21 = 2 ticks on, 1 tick off, repeating.
        let mut builder = MockSongBuilder::new(SongType::S3M, 1);
        builder.master_volume = 128 | 64;
        builder.global_volume = 64;
        builder.add_pattern_row(0, 0, 49, 1, 64, 9, 0x21); // I21
        let mut tester = SongTester::new(builder.build());

        let mut had_audible = false;
        let mut had_silent = false;
        for _ in 0..6 {
            tester.tick();
            let v = tester.song.voices[0].volume.output_volume;
            if v > 0.01 { had_audible = true; }
            if v < 1e-6 { had_silent = true; }
        }
        assert!(had_audible, "Tremor should let the voice through during the on phase");
        assert!(had_silent,  "Tremor should silence the voice during the off phase");
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
        println!("pan0: {}, pan1: {}", pan0, pan1);
        
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
    fn test_s3m_tremolo_modulates_output_volume() {
        // Regression: S3M effect 18 (R = Tremolo) was parsed but never
        // dispatched, so tremolo on S3M modules played silently.
        let mut builder = MockSongBuilder::new(SongType::S3M, 1);
        builder.master_volume = 128 | 64;
        builder.global_volume = 64;
        // Note + Tremolo with max-ish swing.
        builder.add_pattern_row(0, 0, 49, 1, 64, 18, 0xFF);
        let mut tester = SongTester::new(builder.build());

        let mut min_v: f32 = f32::INFINITY;
        let mut max_v: f32 = -f32::INFINITY;
        for _ in 0..6 {
            tester.tick();
            let v = tester.song.voices[0].volume.output_volume;
            min_v = min_v.min(v);
            max_v = max_v.max(v);
        }
        assert!(max_v - min_v > 0.05,
                "S3M tremolo should modulate output; saw range {:.3}..{:.3}",
                min_v, max_v);
    }

    #[test]
    fn test_s3m_fine_vibrato_smaller_swing_than_vibrato() {
        // S3M U (effect 21) is fine vibrato: same params as H but the depth
        // multiplier is 1 instead of 4. Compare voice frequency excursion
        // (vibrato modulates frequency, not channel.note.period) of H88 vs
        // U88; H should sweep wider than U.
        fn max_freq_excursion(effect: u8) -> f32 {
            let mut builder = MockSongBuilder::new(SongType::S3M, 1);
            builder.add_pattern_row(0, 0, 49, 1, 64, effect, 0x88); // speed 8, depth 8
            let mut tester = SongTester::new(builder.build());
            tester.tick();
            let base = tester.song.voices[0].frequency;
            let mut max_dev: f32 = 0.0;
            for _ in 0..32 {
                tester.tick();
                let f = tester.song.voices[0].frequency;
                let d = (f - base).abs();
                if d > max_dev { max_dev = d; }
            }
            max_dev
        }

        let h_swing = max_freq_excursion(8);   // H = regular vibrato
        let u_swing = max_freq_excursion(21);  // U = fine vibrato
        assert!(u_swing > 0.0, "U should produce some pitch swing, got {}", u_swing);
        assert!(h_swing > u_swing,
                "H vibrato swing ({}) should exceed U fine vibrato swing ({})",
                h_swing, u_swing);
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
