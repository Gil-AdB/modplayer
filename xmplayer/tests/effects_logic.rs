use xmplayer::module_reader::SongType;
use xmplayer::test_utils::{MockSongBuilder, SongTester};
use xmplayer::pattern::Pattern;

#[test]
fn test_pattern_break_it() {
    let mut builder = MockSongBuilder::new(SongType::IT, 1);
    
    // Pattern 0: 10 rows
    builder.add_empty_pattern(10);
    // On row 2, set C05 (Break to row 5 of next pattern)
    let pattern_break = Pattern {
        note: 0,
        instrument: 0,
        volume: 255,
        effect: 0x03, // IT C
        effect_param: 0x05,
    };
    builder.set_pattern_row(0, 2, 0, pattern_break);
    
    // Pattern 1: 10 rows
    builder.add_empty_pattern(10);
    
    let mut tester = SongTester::new(builder.build());
    
    // Step from Row 0 to 1
    tester.step_row();
    // Step from Row 1 to 2
    tester.step_row();
    // Step from Row 2 to breakthrough
    tester.step_row();
    
    let (pos, row, _tick) = tester.get_pos();
    assert_eq!(pos, 1, "Should have jumped to next pattern");
    assert_eq!(row, 5, "Should have jumped to row 5");
}

#[test]
fn test_pattern_jump_s3m() {
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    
    // Pattern 0, 1, 2
    builder.add_empty_pattern(64);
    builder.add_empty_pattern(64);
    builder.add_empty_pattern(64);
    
    // On Row 0 of Pattern 0, Jump to Order 2 (Pattern 2)
    let pattern_jump = Pattern {
        note: 0,
        instrument: 0,
        volume: 255,
        effect: 0x82, // S3M B
        effect_param: 0x02,
    };
    builder.set_pattern_row(0, 0, 0, pattern_jump);
    
    let mut tester = SongTester::new(builder.build());
    
    // Step from Row 0 to next
    tester.step_row();
    
    let (pos, row, _) = tester.get_pos();
    assert_eq!(pos, 2, "Should have jumped to order 2");
    assert_eq!(row, 0, "Should be at row 0 of pattern 2");
}

#[test]
fn test_pattern_break_bcd_mod() {
    let mut builder = MockSongBuilder::new(SongType::MOD, 1);
    
    builder.add_empty_pattern(64);
    builder.add_empty_pattern(64);
    
    // D10 in MOD should jump to Row 10 (decimal), not 16
    let pattern_break = Pattern {
        note: 0,
        instrument: 0,
        volume: 255,
        effect: 0x0D, // MOD D
        effect_param: 0x10, // BCD for 10
    };
    builder.set_pattern_row(0, 0, 0, pattern_break);
    
    let mut tester = SongTester::new(builder.build());
    
    tester.step_row();
    
    let (pos, row, _) = tester.get_pos();
    assert_eq!(pos, 1);
    assert_eq!(row, 10, "Should have jumped to row 10 due to BCD parsing");
}
