use xmplayer::module_reader::SongType;
use xmplayer::test_utils::MockSongBuilder;
use xmplayer::pattern::Pattern;

#[test]
fn test_arpeggio_wrapping() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: B-8 (very high note) with 037 arpeggio
    // B-8 is note 108.
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 108, instrument: 1, volume: 255, effect: 0x00, effect_param: 0x37,
    });
    
    let mut tester = builder.get_tester();
    
    tester.tick(); // Tick 0: B-8
    let freq0 = tester.song.voices[0].frequency;
    
    tester.tick(); // Tick 1: B-8 + 3 semitones.
    let freq1 = tester.song.voices[0].frequency;
    assert!(freq1 >= freq0, "Frequency decreased on tick 1: freq0={}, freq1={}", freq0, freq1);
    
    tester.tick(); // Tick 2: B-8 + 7 semitones.
    let freq2 = tester.song.voices[0].frequency;
    assert!(freq2 >= freq1, "Frequency decreased on tick 2: freq1={}, freq2={}", freq1, freq2);
}

#[test]
fn test_portamento_down_memory() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: C-4 with 208 Portamento Down
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 255, effect: 0x02, effect_param: 0x08,
    });
    // Row 1: Continue with 200 Portamento Down
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 0, instrument: 0, volume: 255, effect: 0x02, effect_param: 0x00,
    });
    
    let mut tester = builder.get_tester();
    
    tester.tick(); // Row 0, Tick 0: C-4 (8363 Hz)
    tester.assert_pitch_near(0, 8363.0, 1.0);
    
    tester.tick(); // Tick 1: Slide down
    let freq1 = tester.song.voices[0].frequency;
    assert!(freq1 < 8363.0);
    
    tester.step_row(); // Move to Row 1, Tick 0
    tester.tick(); // Process Tick 0
    let freq_start_row1 = tester.song.voices[0].frequency;
    
    tester.tick(); // Row 1, Tick 1: Should continue sliding down using memory
    let freq_row1_tick1 = tester.song.voices[0].frequency;
    assert!(freq_row1_tick1 < freq_start_row1);
}

#[test]
fn test_portamento_limits() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: C-1 (lowest note) with 2FF (fast slide down)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 13, instrument: 1, volume: 255, effect: 0x02, effect_param: 0xFF,
    });
    
    let mut tester = builder.get_tester();
    
    for _ in 0..100 {
        tester.tick();
    }
    
    let freq_low = tester.song.voices[0].frequency;
    assert!(freq_low > 0.0); // Should not become 0 or negative
    
    // Row 1: B-8 (highest note) with 1FF (fast slide up)
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 108, instrument: 1, volume: 255, effect: 0x01, effect_param: 0xFF,
    });
    
    tester.step_row();
    for _ in 0..100 {
        tester.tick();
    }
    
    let freq_high = tester.song.voices[0].frequency;
    assert!(freq_high < 1_000_000.0); // Should be bounded by a reasonable maximum
}

#[test]
fn test_tone_portamento_glissando() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: C-4
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 255, effect: 0, effect_param: 0,
    });
    // Row 1: E31 (Enable Glissando)
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 0, instrument: 0, volume: 255, effect: 0x0E, effect_param: 0x31,
    });
    // Row 2: 304 (Slide to D-4, speed 4)
    builder.set_pattern_row(0, 2, 0, Pattern {
        note: 51, instrument: 0, volume: 255, effect: 0x03, effect_param: 0x04,
    });
    
    let mut tester = builder.get_tester();
    
    tester.run_row(); // Row 0
    tester.run_row(); // Row 1 (Enables glissando)
    
    tester.tick(); // Row 2, Tick 0: C-4
    tester.assert_pitch_near(0, 8363.0, 1.0);
    
    // With glissando, it should jump in semitones.
    // C-4 = 8363 Hz. C#4 = 8860 Hz. D-4 = 9387 Hz.
    // Without glissando, it would be somewhere in between.
    // Let's see if it snaps.
    
    for _ in 0..10 {
        tester.tick();
        let freq = tester.song.voices[0].frequency;
        // Frequency should be close to a semitone
        let note = 12.0f32 * (freq / 8363.0f32).log2();
        let note_rounded = note.round();
        assert!((note - note_rounded).abs() < 0.1f32, "Frequency {} Hz is not snapped to semitone (note offset {})", freq, note);
    }
}

#[test]
fn test_tone_portamento_target_replacement() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: C-4
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 255, effect: 0, effect_param: 0,
    });
    // Row 1: 304 (Slide to D-4)
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 51, instrument: 0, volume: 255, effect: 0x03, effect_param: 0x04,
    });
    // Row 2: E-4 (New Note, No Effect) - should reset target?
    // In FT2, a new note trigger REPLACES the target if followed by a 3xx? 
    // Actually, a new note trigger on its own plays normally.
    builder.set_pattern_row(0, 2, 0, Pattern {
        note: 53, instrument: 1, volume: 255, effect: 0, effect_param: 0,
    });
    // Row 3: 304 (Slide to F-4)
    builder.set_pattern_row(0, 3, 0, Pattern {
        note: 54, instrument: 0, volume: 255, effect: 0x03, effect_param: 0x04,
    });
    
    let mut tester = builder.get_tester();
    
    tester.run_row(); // Row 0
    tester.run_row(); // Row 1 (Sliding to D-4)
    
    // Now we are at Row 2, Tick 0.
    tester.tick(); // Row 2, Tick 0: E-4
    // E-4 freq: 8363 * 2^(4/12) = 10536 Hz.
    tester.assert_pitch_near(0, 10536.0, 10.0);
    
    tester.run_row(); // Finish Row 2
    
    // Now we are at Row 3, Tick 0.
    tester.tick(); // Row 3, Tick 0: Still E-4
    tester.assert_pitch_near(0, 10536.0, 10.0);
    
    tester.tick(); // Row 3, Tick 1: Sliding to F-4 (Note 54)
    // F-4 freq: 8363 * 2^(5/12) = 11162 Hz.
    let freq_after = tester.song.voices[0].frequency;
    assert!(freq_after > 10536.0);
    assert!(freq_after < 11162.0);
}

#[test]
fn test_vibrato_parameter_memory() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: C-4 with 442 (Speed 4, Depth 2)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 255, effect: 0x04, effect_param: 0x42,
    });
    // Row 1: 400 (Speed last, Depth last) - should remember both
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 0, instrument: 0, volume: 255, effect: 0x04, effect_param: 0x00,
    });
    // Row 2: 400 (Use last Speed 4, last Depth 2)
    builder.set_pattern_row(0, 2, 0, Pattern {
        note: 0, instrument: 0, volume: 255, effect: 0x04, effect_param: 0x00,
    });
    
    let mut tester = builder.get_tester();
    
    tester.run_row(); // Row 0
    tester.run_row(); // Row 1
    tester.tick(); // Row 2, Tick 0
    tester.tick(); // Row 2, Tick 1
    // Vibrato should be active with Speed 4, Depth 2
    let freq = tester.song.voices[0].frequency;
    assert!((freq - 8363.0f32).abs() > 0.1f32);
}

#[test]
fn test_vibrato_waveforms() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);

    // Square wave vibrato (E42): the wave generator returns ±max with no
    // intermediate values, so the magnitude of the frequency deviation
    // from the unmodulated baseline must be constant across ticks (only
    // the sign flips at the half-cycle boundary). The previous version of
    // this test asserted f1 == f2 outright, which only held when speed was
    // stored ×1 — now that speed is ×4 (matching master / ST3 / FT2), pos
    // can wrap inside two ticks and the sign legitimately flips.
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 255, effect: 0x0E, effect_param: 0x42,
    });
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 0, instrument: 0, volume: 255, effect: 0x04, effect_param: 0x44,
    });

    let mut tester = builder.get_tester();
    tester.tick();    // Row 0 tick 0: E42 only; no vibrato yet.
    let base_unmod = tester.song.voices[0].frequency;
    tester.run_row(); // finish row 0; advance to row 1 tick 0.

    // Walk row 1 long enough for the square wave to sweep both halves —
    // with speed×4 (=16), pos wraps within 2 next_ticks, so a full ± pair
    // is visible in 6 ticks. Square produces only ±one magnitude relative
    // to the *unmodulated* baseline; check exactly that.
    let mut max_dev: f32 = 0.0;
    let mut min_nonzero_dev: f32 = f32::INFINITY;
    for _ in 0..6 {
        tester.tick();
        let dev = (tester.song.voices[0].frequency - base_unmod).abs();
        if dev > max_dev { max_dev = dev; }
        if dev > 0.001 && dev < min_nonzero_dev { min_nonzero_dev = dev; }
    }

    assert!(max_dev > 0.0, "square vibrato should deviate from baseline");
    assert!((max_dev - min_nonzero_dev).abs() < 0.001,
            "square wave deviations must share one magnitude: max={} min_nonzero={}",
            max_dev, min_nonzero_dev);
}

#[test]
fn test_vibrato_retrig() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: 444 Vibrato
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 255, effect: 0x04, effect_param: 0x44,
    });
    // Row 1: C-4 (New note) - does it reset vibrato?
    // By default (E40), it resets.
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 49, instrument: 1, volume: 255, effect: 0x04, effect_param: 0x44,
    });
    
    let mut tester = builder.get_tester();
    tester.run_row(); // Row 0
    
    tester.tick(); // Row 1, Tick 0: Should reset to base frequency
    tester.assert_pitch_near(0, 8363.0, 1.0);
}
