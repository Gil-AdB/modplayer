use xmplayer::module_reader::{SongType};
use xmplayer::pattern::Pattern;
use xmplayer::test_utils::{MockSongBuilder};

#[test]
fn test_global_volume_basic() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: Set Global Volume 32 (G20)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 0, instrument: 0, volume: 0, effect: 0x10, effect_param: 0x20,
    });
    
    let mut tester = builder.get_tester();
    
    // Row 0, Tick 0
    tester.song.process_tick();
    assert_eq!(tester.song.global_volume.volume, 32);
}

#[test]
fn test_global_volume_slide() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: Set Global Vol 32, Row 1: Slide Up 2 (H20)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 0, instrument: 0, volume: 0, effect: 0x10, effect_param: 0x20,
    });
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 0, instrument: 0, volume: 0, effect: 0x11, effect_param: 0x20,
    });
    
    let mut tester = builder.get_tester();
    
    // Row 0
    for _ in 0..6 {
        tester.song.process_tick();
        tester.song.next_tick();
    }
    
    // Row 1, Tick 0
    tester.song.process_tick();
    assert_eq!(tester.song.global_volume.volume, 32);
    tester.song.next_tick();
    
    // Row 1, Tick 1: Slide Up
    tester.song.process_tick();
    assert_eq!(tester.song.global_volume.volume, 34);
}

#[test]
fn test_pattern_jump() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64); // Pattern 0
    builder.add_empty_pattern(64); // Pattern 1
    
    // Row 0: Jump to Pattern 1 (B01)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 0, instrument: 0, volume: 0, effect: 0x0B, effect_param: 0x01,
    });
    
    let mut tester = builder.get_tester();
    
    // Row 0, Tick 0: Command processed
    tester.song.process_tick();
    tester.song.next_tick();
    
    // Should jump to Pattern 1, Row 0 after this row finishes
    for _ in 0..5 {
        tester.song.process_tick();
        tester.song.next_tick();
    }
    
    assert_eq!(tester.song.song_position, 1);
    assert_eq!(tester.song.row, 0);
}

#[test]
fn test_pattern_break() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64); // Pattern 0
    builder.add_empty_pattern(64); // Pattern 1
    
    // Row 0: Break to Row 10 (D0A or D10 depending on BCD)
    // If FT2/XM uses BCD, D10 -> Row 10.
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 0, instrument: 0, volume: 0, effect: 0x0D, effect_param: 0x10,
    });
    
    let mut tester = builder.get_tester();
    
    // Row 0, Tick 0
    tester.song.process_tick();
    tester.song.next_tick();
    
    // Finish row
    for _ in 0..5 {
        tester.song.process_tick();
        tester.song.next_tick();
    }
    
    assert_eq!(tester.song.song_position, 1);
    // If it's BCD, it should be 10. If it's Hex, it should be 16.
    assert_eq!(tester.song.row, 10, "XM Pattern Break should be BCD");
}

#[test]
fn test_pattern_loop() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 2: Loop Begin (E60)
    builder.set_pattern_row(0, 2, 0, Pattern {
        note: 0, instrument: 0, volume: 0, effect: 0x0E, effect_param: 0x60,
    });
    // Row 3: Loop End, 1 time (E61)
    builder.set_pattern_row(0, 3, 0, Pattern {
        note: 0, instrument: 0, volume: 0, effect: 0x0E, effect_param: 0x61,
    });
    
    let mut tester = builder.get_tester();
    
    // Row 0, 1
    tester.run_row();
    tester.run_row();
    
    // Row 2: Loop Begin
    assert_eq!(tester.song.row, 2);
    tester.run_row();
    
    // Row 3: Loop End
    assert_eq!(tester.song.row, 3);
    tester.run_row();
    
    // Should jump back to Row 2
    assert_eq!(tester.song.row, 2, "Loop failed to jump back");
    tester.run_row();
    
    // Row 3: Loop End (second time, should not jump)
    assert_eq!(tester.song.row, 3);
    tester.run_row();
    
    assert_eq!(tester.song.row, 4, "Loop failed to exit");
}

#[test]
fn test_pattern_delay() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: Pattern Delay 2 rows (EE2)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 0, instrument: 0, volume: 0, effect: 0x0E, effect_param: 0xE2,
    });
    
    let mut tester = builder.get_tester();
    
    // Row 0: Process all ticks (Speed is 6 by default)
    tester.run_ticks(6);
    
    // Should stay on Row 0 for 2 more times (total 3 times)
    assert_eq!(tester.song.row, 0, "Should still be on row 0 (delay 1/2)");
    tester.run_ticks(6);
    
    assert_eq!(tester.song.row, 0, "Should still be on row 0 (delay 2/2)");
    tester.run_ticks(6);
    
    // Finally move to row 1
    assert_eq!(tester.song.row, 1, "Failed to move after pattern delay");
}
