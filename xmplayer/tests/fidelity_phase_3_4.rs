use xmplayer::module_reader::{SongType};
use xmplayer::pattern::Pattern;
use xmplayer::test_utils::{MockSongBuilder};

#[test]
fn test_tremolo_basic() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Set Tremolo: Speed 4, Depth 8, Vol 32
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 1, instrument: 1, volume: 32 + 0x10, effect: 0x07, effect_param: 0x48,
    });
    
    let mut tester = builder.get_tester();
    
    // Row 0, Tick 0
    tester.song.process_tick();
    let v0 = tester.song.voices[0].volume.output_volume;
    tester.song.next_tick();
    
    // Row 0, Tick 1
    tester.song.process_tick();
    let v1 = tester.song.voices[0].volume.output_volume;
    
    assert!(v1 != v0, "Volume should change during tremolo. v0={}, v1={}", v0, v1);
}

#[test]
fn test_tremolo_parameter_memory() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: Set Tremolo Speed 4, Depth 8, Vol 32
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 1, instrument: 1, volume: 32 + 0x10, effect: 0x07, effect_param: 0x48,
    });
    // Row 1: Use Memory (700)
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 0, instrument: 0, volume: 255, effect: 0x07, effect_param: 0x00,
    });
    
    let mut tester = builder.get_tester();
    
    // Run Row 0
    for _ in 0..6 {
        tester.song.process_tick();
        tester.song.next_tick();
    }
    
    // Row 1, Tick 0
    tester.song.process_tick();
    tester.song.next_tick();
    
    // Row 1, Tick 1
    tester.song.process_tick();
    let v_mem = tester.song.voices[0].volume.output_volume;
    let v_base = 32.0 / 64.0;
    
    assert!((v_mem - v_base).abs() > 0.001, "Tremolo memory not working. v_mem={}, v_base={}", v_mem, v_base);
}

#[test]
fn test_volume_slide_basic() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: Volume Slide Down (A02), Start with Vol 64
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 1, instrument: 1, volume: 64 + 0x10, effect: 0x0A, effect_param: 0x02,
    });
    
    let mut tester = builder.get_tester();
    
    // Row 0, Tick 0
    tester.song.process_tick();
    let v0 = tester.song.voices[0].volume.output_volume;
    assert_eq!(v0, 1.0);
    tester.song.next_tick();
    
    // Row 0, Tick 1
    tester.song.process_tick();
    let v1 = tester.song.voices[0].volume.output_volume;
    assert!(v1 < 1.0, "Volume should decrease on tick 1. v1={}", v1);
    tester.song.next_tick();
    
    // Row 0, Tick 2
    tester.song.process_tick();
    let v2 = tester.song.voices[0].volume.output_volume;
    assert!(v2 < v1, "Volume should decrease further on tick 2. v1={}, v2={}", v1, v2);
}

#[test]
fn test_volume_column_slide() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: Volume Column Slide Up (0x74 = Slide Up 4)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 1, instrument: 1, volume: 0x74, effect: 0x00, effect_param: 0x00,
    });
    
    let mut tester = builder.get_tester();
    // Start with low volume
    tester.song.voices[0].volume.set_volume(32);
    
    // Row 0, Tick 0
    tester.song.process_tick();
    tester.song.next_tick();
    
    // Row 0, Tick 1
    tester.song.process_tick();
    let v1 = tester.song.voices[0].volume.get_volume();
    assert!(v1 > 32, "Volume column slide up failed. v1={}", v1);
}

#[test]
fn test_panning_basic() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Set Panning: 0x80 (Center)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 1, instrument: 1, volume: 255, effect: 0x08, effect_param: 0x80,
    });
    
    let mut tester = builder.get_tester();
    
    // Row 0, Tick 0
    tester.song.process_tick();
    let p0 = tester.song.voices[0].panning.panning;
    assert_eq!(p0, 0x80);
}

#[test]
fn test_panning_slide() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Set Panning to 128, then slide Right (P10)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 1, instrument: 1, volume: 255, effect: 0x08, effect_param: 0x80,
    });
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 0, instrument: 0, volume: 255, effect: 0x19, effect_param: 0x10,
    });
    
    let mut tester = builder.get_tester();
    
    // Row 0
    for _ in 0..6 {
        tester.song.process_tick();
        tester.song.next_tick();
    }
    
    // Row 1, Tick 0
    tester.song.process_tick();
    let p0 = tester.song.voices[0].panning.panning;
    assert_eq!(p0, 0x80);
    tester.song.next_tick();
    
    // Row 1, Tick 1: Panning should increase (Right)
    tester.song.process_tick();
    let p1 = tester.song.voices[0].panning.panning;
    assert!(p1 > p0, "Panning slide right failed. p0={}, p1={}", p0, p1);
}

#[test]
fn test_sample_offset() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: Sample Offset 0x01 (256 samples)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 1, instrument: 1, volume: 255, effect: 0x09, effect_param: 0x01,
    });
    
    let mut tester = builder.get_tester();
    
    // Row 0, Tick 0
    tester.song.process_tick();
    let pos = tester.song.voices[0].sample_position;
    // Offset 0x01 means 256 samples + 4 prefix samples
    assert!((pos - 260.0).abs() < 0.1, "Sample offset failed. pos={}", pos);
}

#[test]
fn test_retrig_note() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(64);
    
    // Row 0: Retrig every 2 ticks (E92)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 1, instrument: 1, volume: 255, effect: 0x0E, effect_param: 0x92,
    });
    
    let mut tester = builder.get_tester();
    
    // Row 0, Tick 0
    tester.song.process_tick();
    tester.song.next_tick();
    
    // Row 0, Tick 1: Nothing
    tester.song.process_tick();
    tester.song.next_tick();
    
    // Row 0, Tick 2: Retrig!
    tester.song.process_tick();
    let pos = tester.song.voices[0].sample_position;
    // Trigger resets sample position to 4.0
    assert!((pos - 4.0).abs() < 0.1, "Retrig failed. pos={}", pos);
}
