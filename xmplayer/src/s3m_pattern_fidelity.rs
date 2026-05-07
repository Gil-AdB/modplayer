#[cfg(test)]
mod s3m_pattern_tests {
    use crate::test_utils::{MockSongBuilder, SongTester};
    use crate::module_reader::SongType;

    #[test]
    fn test_s3m_pattern_break_decimal() {
        let mut builder = MockSongBuilder::new(SongType::S3M, 1);
        builder.add_empty_pattern(64); // Pattern 0
        builder.add_empty_pattern(64); // Pattern 1
        
        // Row 0 of Pattern 0: Break to Row 16 of next pattern
        // In S3M, C10 is row 16. If it was BCD, it would be row 10.
        builder.add_pattern_row(0, 0, 0, 0, 255, 3, 16); 
        
        let mut tester = SongTester::new(builder.build());
        tester.song.speed = 3;
        
        tester.tick(); // Row 0, Tick 0
        tester.tick(); // Row 0, Tick 1
        tester.tick(); // Row 0, Tick 2
        
        // After 3 ticks, it should have moved to Row 16 of next pattern
        assert_eq!(tester.song.song_position, 1);
        assert_eq!(tester.song.row, 16);
    }

    #[test]
    fn test_s3m_pattern_delay_last_row() {
        let mut builder = MockSongBuilder::new(SongType::S3M, 1);
        builder.add_empty_pattern(64); // Pattern 0
        builder.add_empty_pattern(64); // Pattern 1
        
        // Row 63 of Pattern 0: Delay 1 row (SxE where x=1)
        // Row 63 should play twice.
        builder.add_pattern_row(0, 63, 49, 1, 64, 19, 0xE1); 
        
        let mut tester = SongTester::new(builder.build());
        tester.song.speed = 3;
        
        tester.step_to_row(63);
        assert_eq!(tester.song.row, 63);
        
        // Row 63, cycle 1 (3 ticks)
        tester.tick(); tester.tick(); tester.tick();
        assert_eq!(tester.song.row, 63, "Should still be at Row 63 due to delay");
        
        // Row 63, cycle 2 (3 ticks)
        tester.tick(); tester.tick(); tester.tick();
        assert_eq!(tester.song.song_position, 1, "Should have moved to next pattern");
        assert_eq!(tester.song.row, 0);
    }

    #[test]
    fn test_s3m_pattern_break_bcd_regression() {
        let mut builder = MockSongBuilder::new(SongType::S3M, 1);
        builder.add_empty_pattern(64);
        builder.add_empty_pattern(64);
        
        // C14 should go to row 20 (decimal 20 is 0x14).
        // If treated as BCD, it goes to row 14.
        builder.add_pattern_row(0, 0, 0, 0, 255, 3, 0x14);
        
        let mut tester = SongTester::new(builder.build());
        tester.step_row();
        
        assert_eq!(tester.song.row, 20, "S3M Cxy should be decimal/hex, not BCD");
    }

    #[test]
    fn test_xm_pattern_break_bcd() {
        let mut builder = MockSongBuilder::new(SongType::XM, 1);
        builder.add_empty_pattern(64);
        builder.add_empty_pattern(64);
        
        // D10 in XM is Row 10 (BCD). Stored as 0x10.
        builder.add_pattern_row(0, 0, 0, 0, 255, 13, 0x10);
        
        let mut tester = SongTester::new(builder.build());
        tester.step_row();
        
        assert_eq!(tester.song.row, 10, "XM Dxy MUST be BCD");
    }
}
