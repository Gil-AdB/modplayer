use xmplayer::module_reader::SongType;
use xmplayer::test_utils::{MockSongBuilder};
use xmplayer::pattern::Pattern;

#[test]
fn test_arpeggio_memory() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: C-4 with 037 arpeggio
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 255, effect: 0x00, effect_param: 0x37,
    });
    // Row 1: C-4 with 037 arpeggio (XM 000 has no memory, so we must repeat 37)
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 49, instrument: 1, volume: 255, effect: 0x00, effect_param: 0x37,
    });
    
    let mut tester = builder.get_tester();
    
    // Row 0, Tick 0: C-4 (8363 Hz)
    tester.tick();
    tester.assert_pitch_near(0, 8363.0, 1.0);
    // Tick 1: C-4 + 3 semitones (D#4)
    tester.tick();
    tester.assert_pitch_near(0, 8363.0 * 2.0f32.powf(3.0/12.0), 10.0);
    // Tick 2: C-4 + 7 semitones (G-4)
    tester.tick();
    tester.assert_pitch_near(0, 8363.0 * 2.0f32.powf(7.0/12.0), 10.0);
    
    // Skip remaining ticks of row 0
    while tester.song.tick != 0 { tester.tick(); }
    
    // Now we are at Row 1, Tick 0. But process_tick for it hasn't run yet.
    tester.tick(); // Process Row 1, Tick 0
    // Row 1, Tick 0: C-4
    tester.assert_pitch_near(0, 8363.0, 1.0);
    // Tick 1: Should be D#4 again because of memory
    tester.tick();
    tester.assert_pitch_near(0, 8363.0 * 2.0f32.powf(3.0/12.0), 10.0);
}

#[test]
fn test_portamento_up_memory() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: C-4 with 104 Portamento Up
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 255, effect: 0x01, effect_param: 0x04,
    });
    // Row 1: Continue with 100 Portamento Up
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 0, instrument: 0, volume: 255, effect: 0x01, effect_param: 0x00,
    });
    
    let mut tester = builder.get_tester();
    
    // Row 0, Tick 0: C-4 (8363 Hz, Period 4608)
    tester.tick();
    tester.assert_pitch_near(0, 8363.0, 1.0);
    
    // Tick 1: Period should be 4608 - 4*4 = 4592
    tester.tick();
    // Period 4592 corresponds to a slightly higher frequency
    // (7744 - idx*4) = 4592 => idx*4 = 3152 => idx = 788
    // freq = 2^((9216-4592)%768 / 768) * 8363 / 2^(14-(9216-4592)/768)
    // 9216-4592 = 4624. 4624 / 768 = 6. 4624 % 768 = 16.
    // freq = 2^(16/768) * 8363 = 1.0145 * 8363 = 8484 Hz.
    tester.assert_pitch_near(0, 8484.0, 10.0);
    
    // Row 1, Tick 1: Should continue sliding using memory
    tester.step_row(); // now at Row 1, Tick 0
    tester.tick(); // Tick 1
    // It should have slid more by now.
    let freq_after_row_0 = tester.song.voices[0].frequency;
    assert!(freq_after_row_0 > 8484.0);
}

#[test]
fn test_tone_portamento_exact_target() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: C-4
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 255, effect: 0, effect_param: 0,
    });
    // Row 1: Tone Portamento to D-4 (Note 51) with speed 0xFF (very fast)
    // D-4 idx = (51-1)*16 + 16 = 50*16 + 16 = 816.
    // D-4 Period = 7744 - 816*4 = 7744 - 3264 = 4480.
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 51, instrument: 0, volume: 255, effect: 0x03, effect_param: 0xFF,
    });
    
    let mut tester = builder.get_tester();
    
    tester.step_row(); // Row 0
    tester.tick(); // Row 1, Tick 0 - should still be C-4
    tester.assert_pitch_near(0, 8363.0, 1.0);
    
    tester.tick(); // Row 1, Tick 1 - should have jumped to D-4 and stopped
    // D-4 frequency: 9216 - 4480 = 4736. 4736/768 = 6. 4736%768 = 128.
    // freq = 2^(128/768) * 8363 = 1.1224 * 8363 = 9387 Hz.
    tester.assert_pitch_near(0, 9387.0, 10.0);
    
    tester.tick(); // Row 1, Tick 2 - should NOT overshoot
    tester.assert_pitch_near(0, 9387.0, 10.0);
}

#[test]
fn test_vibrato_execution() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: C-4 with 444 Vibrato (Speed 4, Depth 4)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 255, effect: 0x04, effect_param: 0x44,
    });
    
    let mut tester = builder.get_tester();
    
    // Row 0, Tick 0: Base pitch
    tester.tick();
    tester.assert_pitch_near(0, 8363.0, 1.0);
    
    // Tick 1: Pitch should change
    tester.tick();
    let freq1 = tester.song.voices[0].frequency;
    assert!((freq1 - 8363.0).abs() > 1.0);
    
    // Tick 2: Pitch should change more
    tester.tick();
    let freq2 = tester.song.voices[0].frequency;
    assert!((freq2 - freq1).abs() > 1.0);
}

#[test]
fn test_volume_slides() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: C-4 with Volume C-40 (64)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 0x10 + 64, effect: 0, effect_param: 0,
    });
    // Row 1: Volume Slide Down A01 (decrements by 1 on ticks 1..5)
    // Speed is 6, so it should be 64, 63, 62, 61, 60, 59
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 0, instrument: 0, volume: 0, effect: 0x0A, effect_param: 0x01,
    });
    
    let mut tester = builder.get_tester();
    
    tester.step_row(); // Row 0
    tester.tick(); // Row 1, Tick 0: Should be 64
    assert_eq!(tester.song.voices[0].volume.volume, 64);
    
    tester.tick(); // Row 1, Tick 1: Should be 63
    assert_eq!(tester.song.voices[0].volume.volume, 63);
    
    tester.tick(); // Row 1, Tick 2: Should be 62
    assert_eq!(tester.song.voices[0].volume.volume, 62);
}

#[test]
fn test_fine_volume_slides() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: C-4 with Volume 32
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 0x10 + 32, effect: 0, effect_param: 0,
    });
    // Row 1: Fine Volume Slide Up EA1 (increments by 1 on tick 0 only)
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 0, instrument: 0, volume: 0, effect: 0x0E, effect_param: 0xA1,
    });
    
    let mut tester = builder.get_tester();
    
    tester.step_row(); // Row 0
    tester.tick(); // Row 1, Tick 0: Should have incremented to 33 immediately
    assert_eq!(tester.song.voices[0].volume.volume, 33);
    
    tester.tick(); // Row 1, Tick 1: Should still be 33
    assert_eq!(tester.song.voices[0].volume.volume, 33);
}

#[test]
fn test_panning_set() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: Set Panning to 0x00 (Left)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 0, effect: 0x08, effect_param: 0x00,
    });
    // Row 1: Set Panning to 0xFF (Right)
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 0, instrument: 0, volume: 0, effect: 0x08, effect_param: 0xFF,
    });
    
    let mut tester = builder.get_tester();
    
    // Row 0, Tick 0
    tester.tick();
    assert_eq!(tester.song.voices[0].panning.panning, 0x00);
    
    // Finish row 0
    while tester.song.tick != 0 { tester.tick(); }
    
    // Row 1, Tick 0
    tester.tick();
    assert_eq!(tester.song.voices[0].panning.panning, 0xFF);
}

#[test]
fn test_panning_slide() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: C-4 with Panning 128 (Center)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 0, effect: 0x08, effect_param: 128,
    });
    // Row 1: Panning Slide Right 1902 (increments by 2 on ticks 1..5)
    // Speed is 6. Ticks 0..5. Panning should be: 128, 130, 132, 134, 136, 138
    // Wait, 1902 is slide LEFT (y=2). So 128, 126, 124, 122, 120, 118.
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 0, instrument: 0, volume: 0, effect: 0x19, effect_param: 0x02,
    });
    
    let mut tester = builder.get_tester();
    
    // Row 0, Tick 0
    tester.tick();
    assert_eq!(tester.song.voices[0].panning.panning, 128);
    
    // Finish row 0
    while tester.song.tick != 0 { tester.tick(); }
    
    // Row 1, Tick 0: 128
    tester.tick();
    assert_eq!(tester.song.voices[0].panning.panning, 128);
    
    tester.tick(); // Tick 1: 126
    assert_eq!(tester.song.voices[0].panning.panning, 126);
    
    tester.tick(); // Tick 2: 124
    assert_eq!(tester.song.voices[0].panning.panning, 124);
}

#[test]
fn test_set_speed_tempo() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: F03 (Set Speed to 3)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 0, instrument: 0, volume: 0, effect: 0x0F, effect_param: 0x03,
    });
    // Row 1: F40 (Set BPM to 64, which is 0x40)
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 0, instrument: 0, volume: 0, effect: 0x0F, effect_param: 0x40,
    });
    
    let mut tester = builder.get_tester();
    
    tester.step_row(); // Row 0
    tester.tick(); // Tick 0
    assert_eq!(tester.song.speed, 3);
    
    tester.step_row(); // Row 1
    tester.tick(); // Tick 0
    assert_eq!(tester.song.bpm.bpm, 64);
}

#[test]
fn test_position_jump() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64); // Pattern 0
    builder.add_empty_pattern(64); // Pattern 1
    
    // Row 0 of Pattern 0: B01 (Jump to Pattern 1)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 0, instrument: 0, volume: 0, effect: 0x0B, effect_param: 0x01,
    });
    
    let mut tester = builder.get_tester();
    
    // Run row 0
    tester.run_row();
    
    // After Row 0, we should be at Row 0 of Pattern 1
    assert_eq!(tester.song.song_position, 1);
    assert_eq!(tester.song.row, 0);
}

#[test]
fn test_pattern_break() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64); // Pattern 0
    builder.add_empty_pattern(64); // Pattern 1
    
    // Row 0 of Pattern 0: D16 (Break to Row 16 of next pattern)
    // 0x16 in BCD is 16.
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 0, instrument: 0, volume: 0, effect: 0x0D, effect_param: 0x16,
    });
    
    let mut tester = builder.get_tester();
    
    // Run row 0
    tester.run_row();
    
    // After Row 0, we should be at Row 16 of Pattern 1
    assert_eq!(tester.song.song_position, 1);
    assert_eq!(tester.song.row, 16);
}

#[test]
fn test_pattern_loop() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 1: E60 (Set loop start)
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 0, instrument: 0, volume: 0, effect: 0x0E, effect_param: 0x60,
    });
    // Row 2: E61 (Loop once)
    builder.set_pattern_row(0, 2, 0, Pattern {
        note: 0, instrument: 0, volume: 0, effect: 0x0E, effect_param: 0x61,
    });
    
    let mut tester = builder.get_tester();
    
    tester.run_row(); // Row 0
    tester.run_row(); // Row 1 (sets loop start at 1)
    tester.run_row(); // Row 2 (jumps to 1)
    
    assert_eq!(tester.song.row, 1);
    
    tester.run_row(); // Row 1 again
    tester.run_row(); // Row 2 again (loop finished, goes to 3)
    
    assert_eq!(tester.song.row, 3);
}

#[test]
fn test_pattern_delay() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: EE1 (Pattern delay 1 -> row 0 plays twice)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 0, instrument: 0, volume: 0, effect: 0x0E, effect_param: 0xE1,
    });
    
    let mut tester = builder.get_tester();
    
    // Row 0, Ticks 0..5 (Initial play)
    for _ in 0..6 { tester.tick(); }
    assert_eq!(tester.song.row, 0);
    assert_eq!(tester.song.tick, 0); // Should have reset to 0 because of delay
    
    // Row 0, Ticks 0..5 (Delayed play)
    for _ in 0..5 { tester.tick(); }
    assert_eq!(tester.song.row, 0);
    
    tester.tick(); // This should finally move to Row 1, Tick 0
    assert_eq!(tester.song.row, 1);
}

#[test]
fn test_note_delay() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: C-4 with ED3 (Note Delay 3 ticks)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 255, effect: 0x0E, effect_param: 0xD3,
    });
    
    let mut tester = builder.get_tester();
    
    // Tick 0: Note should NOT be triggered yet
    tester.tick();
    assert!(!tester.song.voices[0].on);
    
    // Tick 1: Still nothing
    tester.tick();
    assert!(!tester.song.voices[0].on);
    
    // Tick 2: Still nothing
    tester.tick();
    assert!(!tester.song.voices[0].on);
    
    // Tick 3: Note SHOULD trigger now
    tester.tick();
    assert!(tester.song.voices[0].on);
    tester.assert_pitch_near(0, 8363.0, 1.0);
}

#[test]
fn test_note_cut() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: C-4 with EC2 (Note Cut at tick 2)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 255, effect: 0x0E, effect_param: 0xC2,
    });
    
    let mut tester = builder.get_tester();
    
    // Tick 0: Note triggered
    tester.tick();
    assert!(tester.song.voices[0].on);
    
    // Tick 1: Still on
    tester.tick();
    assert!(tester.song.voices[0].on);
    
    // Tick 2: Should be cut
    tester.tick();
    assert!(!tester.song.voices[0].on);
}

#[test]
fn test_sample_offset() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: C-4 with 901 (Sample Offset 1*256)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 255, effect: 0x09, effect_param: 0x01,
    });
    
    let mut tester = builder.get_tester();
    
    // Row 0, Tick 0: Trigger note with offset
    tester.tick();
    assert!(tester.song.voices[0].on);
    // Offset 1 means position should be 1 * 256.0 + 4.0 = 260.0
    assert_eq!(tester.song.voices[0].sample_position, 260.0);
}

#[test]
fn test_nna_fade() {
    let mut builder = MockSongBuilder::new(SongType::IT, 1);
    builder.add_empty_pattern(64);
    
    // Instrument 1: NNA = Fade (3), Fadeout speed = 64
    builder.instruments[1].nna = 3;
    builder.instruments[1].volume_fadeout = 64;
    
    // Row 0: C-4 with Instrument 1
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 255, effect: 0, effect_param: 0,
    });
    // Row 1: D-4 with Instrument 1 (triggers NNA for Row 0's voice)
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 51, instrument: 1, volume: 255, effect: 0, effect_param: 0,
    });
    
    let mut tester = builder.get_tester();
    
    // Row 0, Tick 0: Voice 0 starts
    tester.tick();
    assert!(tester.song.voices[0].on);
    assert_eq!(tester.song.voices[0].instrument, 1);
    assert!(tester.song.voices[0].sustained);
    
    // Skip to Row 1, Tick 0
    tester.step_row();
    tester.tick();
    
    // Now there should be TWO voices on!
    // Voice 0 should be fading out (sustained = false)
    // Voice 1 should be the new note
    assert!(tester.song.voices[0].on);
    assert!(!tester.song.voices[0].sustained);
    assert!(tester.song.voices[1].on);
    assert!(tester.song.voices[1].sustained);
    
    // After some ticks, Voice 0 should turn off because of fadeout
    for _ in 0..100 {
        tester.tick();
    }
    assert!(!tester.song.voices[0].on);
}
