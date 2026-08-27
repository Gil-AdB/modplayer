use crate::module_reader::SongType;
use crate::test_utils::{MockSongBuilder, SongTester};
use crate::pattern::Pattern;

#[test]
fn test_s3m_speed_bpm_memory() {
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.add_empty_pattern(10);

    // Row 0: A0A (Speed 10)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 0,
        instrument: 0,
        volume: 255,
        effect: 0x01, // A
        effect_param: 0x0A,
    });
    
    // Row 1: A00 (Memory - stays 10)
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 0,
        instrument: 0,
        volume: 255,
        effect: 0x01, // A
        effect_param: 0x00,
    });

    // Row 2: T7D (BPM 125)
    builder.set_pattern_row(0, 2, 0, Pattern {
        note: 0,
        instrument: 0,
        volume: 255,
        effect: 0x14, // T
        effect_param: 0x7D,
    });

    // Row 3: T00 (Memory - stays 125)
    builder.set_pattern_row(0, 3, 0, Pattern {
        note: 0,
        instrument: 0,
        volume: 255,
        effect: 0x14, // T
        effect_param: 0x00,
    });

    let mut tester = builder.get_tester();
    
    // Initial state (from builder build() which sets tempo=6, bpm=125)
    assert_eq!(tester.song.speed, 6);
    assert_eq!(tester.song.bpm.bpm, 125);
    
    // Execute Row 0 (A0A)
    tester.step_row();
    assert_eq!(tester.song.speed, 10);
    
    // Execute Row 1 (A00)
    tester.step_row();
    assert_eq!(tester.song.speed, 10, "Speed memory A00 failed");
    
    // Execute Row 2 (T7D)
    tester.step_row();
    assert_eq!(tester.song.bpm.bpm, 125);
    
    // Execute Row 3 (T00)
    tester.step_row();
    assert_eq!(tester.song.bpm.bpm, 125, "BPM memory T00 failed");
}

#[test]
fn test_s3m_amiga_period_safety() {
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.add_empty_pattern(10);
    builder.add_instrument("Test", vec![0.0; 100]);
    
    let mut tester = builder.get_tester();
    tester.song.song_data.use_amiga = true; // Force Amiga mode
    
    // Artificially inflate the period to force an OOB index access
    // Frequency table is usually 32000-64000 entries
    tester.song.channels[0].note.period = 65535; 
    
    // This previously caused a panic: index out of bounds
    // Now it should return 0.0 frequency safely
    tester.tick();
}

#[test]
fn test_s3m_volume_limit() {
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.add_empty_pattern(10);
    builder.add_instrument("Test", vec![0.0; 100]);
    
    // Row 0: Trigger note with volume 64 (Max S3M)
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, // C-4
        instrument: 1,
        volume: 64, // Native S3M volume
        effect: 0,
        effect_param: 0,
    });
    
    let mut tester = builder.get_tester();
    tester.song.global_volume.volume = 64; // Max S3M global volume
    tester.step_row();
    
    let voice_idx = tester.get_voices_for_channel(0)[0];
    let voice = &tester.song.voices[voice_idx];
    
    // Volume should be exactly 1.0 (64/64)
    let output_vol = voice.volume.output_volume;
    let vol = voice.volume.volume;
    let inst_gv = voice.instrument_global_volume;
    let samp_gv = voice.sample_global_volume;
    let gv = tester.song.global_volume.volume;
    
    assert!((output_vol - 1.0).abs() < 0.001, 
        "S3M volume 64 should be 1.0, got {}. Components: vol={}, inst_gv={}, samp_gv={}, song_gv={}", 
        output_vol, vol, inst_gv, samp_gv, gv);
}

#[test]
fn test_s3m_realtime_advancement() {
    use crate::song::{InterleavedBufferAdaptar, PlaybackCmd};
    use std::sync::mpsc;
    
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.add_empty_pattern(2); // 2 rows
    builder.add_empty_pattern(2); // 2 rows
    
    let mut song_data = builder.build();
    song_data.tempo = 3; // 3 ticks per row
    song_data.bpm = 125;
    
    let mut tester = SongTester::new(song_data);
    let (_tx, mut rx) = mpsc::channel::<PlaybackCmd>();
    
    let mut buffer = [0.0f32; 1024]; // 512 frames * 2 channels
    
    // Song starts at Position 0, Row 0, Tick 0
    assert_eq!(tester.song.row, 0);
    assert_eq!(tester.song.song_position, 0);
    
    // Simulate real-time mixing loop
    let mut found_row_1 = false;
    let mut found_pos_1 = false;
    
    for _ in 0..200 {
        let mut adapter = InterleavedBufferAdaptar { buf: &mut buffer };
        tester.song.get_next_tick(&mut adapter, &mut rx);
        if tester.song.row == 1 { found_row_1 = true; }
        if tester.song.song_position == 1 { found_pos_1 = true; break; }
    }
    
    assert!(found_row_1, "S3M should have advanced to Row 1 in real-time simulation");
    assert!(found_pos_1, "S3M should have advanced to Pattern 1 in real-time simulation");
}

#[test]
fn test_s3m_voice_survival() {
    use crate::song::{InterleavedBufferAdaptar, PlaybackCmd};
    use std::sync::mpsc;
    
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.add_instrument("Test", vec![0.0; 44100]); // 1 second sample
    builder.add_empty_pattern(2); // 2 rows
    builder.set_pattern_row(0, 0, 0, Pattern { note: 60, instrument: 1, volume: 64, effect: 0, effect_param: 0 }); // Row 0: Note 60
    
    let mut song_data = builder.build();
    song_data.tempo = 3; // 3 ticks per row
    
    let mut tester = SongTester::new(song_data);
    let (_tx, mut rx) = mpsc::channel::<PlaybackCmd>();
    let mut buffer = [0.0f32; 1024];
    
    // Step 1: Advance Row 0 using the mixing loop
    let mut adapter = InterleavedBufferAdaptar { buf: &mut buffer };
    for _ in 0..20 {
        tester.song.get_next_tick(&mut adapter, &mut rx);
        if !tester.get_voices_for_channel(0).is_empty() { break; }
    }
    
    let voices = tester.get_voices_for_channel(0);
    assert!(!voices.is_empty(), "Voice should be triggered on Row 0");
    let v_idx = voices[0];
    assert!(tester.song.voices[v_idx].on, "Voice should be ON");
    
    // Step 2: Simulate real-time mixing into Row 1
    // At 125 BPM, 1 tick is ~882 samples. 3 ticks ~ 2646 samples.
    // Our buffer is 512 samples. 6 calls to get_next_tick should pass Row 0.
    for _ in 0..10 {
        let mut adapter = InterleavedBufferAdaptar { buf: &mut buffer };
        tester.song.get_next_tick(&mut adapter, &mut rx);
        if tester.song.row == 1 { break; }
    }
    
    assert_eq!(tester.song.row, 1, "Should have reached row 1");
    // We check for ANY active voice on the channel, because the state machine might have re-triggered it
    let voices_row_1 = tester.get_voices_for_channel(0);
    assert!(!voices_row_1.is_empty(), "Voice should still be active on Row 1");
    let v_idx_row_1 = voices_row_1[0];
    assert!(tester.song.voices[v_idx_row_1].on, "Voice should still be ON on Row 1 (long sample)");
}

#[test]
fn test_s3m_bpm_change() {
    use crate::song::{InterleavedBufferAdaptar, PlaybackCmd};
    use std::sync::mpsc;
    
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.add_empty_pattern(2);
    // Row 0: Set BPM to 150 (Mapped from Txx in S3M loader)
    builder.set_pattern_row(0, 0, 0, Pattern { note: 0, instrument: 0, volume: 255, effect: 1, effect_param: 150 });
    
    let mut song_data = builder.build();
    song_data.bpm = 125;
    
    let mut tester = SongTester::new(song_data);
    let (_tx, mut rx) = mpsc::channel::<PlaybackCmd>();
    let mut buffer = [0.0f32; 1024];
    
    // Initial BPM
    assert_eq!(tester.song.bpm.bpm, 125);
    
    // Simulation
    let mut adapter = InterleavedBufferAdaptar { buf: &mut buffer };
    tester.song.get_next_tick(&mut adapter, &mut rx);
    
    assert_eq!(tester.song.bpm.bpm, 150, "S3M BPM change (mapped to IT effect 1) should work");
}

#[test]
fn test_s3m_volume_slide_down() {
    use crate::song::{InterleavedBufferAdaptar, PlaybackCmd};
    use std::sync::mpsc;
    
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.add_instrument("Test", vec![1.0; 44100]);
    builder.add_empty_pattern(1);
    // Row 0: Note C-5 (60), Vol 64, Effect D04 (Slide down 4 units per tick)
    builder.set_pattern_row(0, 0, 0, Pattern { note: 60, instrument: 1, volume: 64, effect: 4, effect_param: 0x04 });
    
    let mut song_data = builder.build();
    song_data.tempo = 3; // 3 ticks per row
    
    let mut tester = SongTester::new(song_data);
    
    // Tick 0: Trigger note at vol 64 (1.0)
    tester.tick();
    let v_idx = tester.get_voices_for_channel(0)[0];
    let vol0 = tester.song.voices[v_idx].volume.volume;

    // Tick 1: Volume should slide down by 4 units (S3M units 0-64)
    tester.tick();
    let vol1 = tester.song.voices[v_idx].volume.volume;
    
    // Tick 2: Volume should slide down another 4 units
    tester.tick();
    let vol2 = tester.song.voices[v_idx].volume.volume;
    
    assert!(vol1 < vol0, "Volume should decrease on Tick 1");
    assert!(vol2 < vol1, "Volume should decrease on Tick 2");
}

#[test]
fn test_s3m_panning_set() {
    use crate::song::{InterleavedBufferAdaptar, PlaybackCmd};
    use std::sync::mpsc;
    
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.add_instrument("Test", vec![1.0; 44100]);
    builder.add_empty_pattern(1);
    // Row 0: S8F (Set panning to 15, which is right in S3M 0-15 scale? No, S3M uses 0-15 for some and 0-255 for others)
    // S3M S8x sets panning where x is 0-F (0=left, 8=center, F=right)
    // Loader maps S8x to effect 19 (S), param 8x.
    builder.set_pattern_row(0, 0, 0, Pattern { note: 60, instrument: 1, volume: 255, effect: 19, effect_param: 0x8F });
    
    let mut song_data = builder.build();
    let mut tester = SongTester::new(song_data);
    let (_tx, mut rx) = mpsc::channel::<PlaybackCmd>();
    let mut buffer = [0.0f32; 1024];
    
    let mut adapter = InterleavedBufferAdaptar { buf: &mut buffer };
    tester.song.get_next_tick(&mut adapter, &mut rx);
    
    let v_idx = tester.get_voices_for_channel(0)[0];
    let pan = tester.song.voices[v_idx].panning.panning;
    
    println!("Panning: {}", pan);
    // S8F should be panned far right.
    assert!(pan >= 128, "S8F should result in right panning, got {}", pan);
}
