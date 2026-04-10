use crate::test_utils::{MockSongBuilder, SongTester};
use crate::module_reader::{SongType};
use crate::pattern::Pattern;

#[test]
fn test_it_nna_continue() {
    let mut builder = MockSongBuilder::new(SongType::IT, 2);
    builder.add_empty_pattern(64);
    
    // Instrument 1: NNA = Continue (1)
    builder.add_instrument("Pad", vec![0.5; 1000]);
    builder.instruments[1].nna = 1; 
    
    // Row 0: Play Note C-5 on Ch 0
    builder.set_pattern_row(0, 0, 0, Pattern { note: 61, instrument: 1, volume: 64, ..Pattern::new() });
    
    // Row 1: Play Note D-5 on Ch 0 (should trigger NNA Continue)
    builder.set_pattern_row(0, 1, 0, Pattern { note: 63, instrument: 1, volume: 64, ..Pattern::new() });
    
    let mut tester = SongTester::new(builder.build());
    
    // Move through Row 0
    tester.step_row(); 
    assert_eq!(tester.get_active_voices(), 1);
    
    // Move to Row 1, Process first tick
    tester.tick();
    
    assert_eq!(tester.get_active_voices(), 2, "Should have 2 active voices due to NNA Continue");
    let voices = tester.get_voices_for_channel(0);
    assert_eq!(voices.len(), 2);
}

#[test]
fn test_it_nna_note_off() {
    let mut builder = MockSongBuilder::new(SongType::IT, 2);
    builder.add_empty_pattern(64);
    
    // Instrument 1: NNA = Note Off (2)
    builder.add_instrument("Pad", vec![0.5; 1000]);
    builder.instruments[1].nna = 2; 
    // IMPORTANT: Note Off only enters background if volume envelope is ON.
    builder.instruments[1].volume_envelope.on = true; 
    builder.instruments[1].volume_envelope.size = 1;
    builder.instruments[1].volume_envelope.points[0] = crate::envelope::EnvelopePoint { frame: 0, value: 64 };
    
    // Row 0: Play Note
    builder.set_pattern_row(0, 0, 0, Pattern { note: 61, instrument: 1, volume: 64, ..Pattern::new() });
    
    // Row 1: Play new note
    builder.set_pattern_row(0, 1, 0, Pattern { note: 63, instrument: 1, volume: 64, ..Pattern::new() });
    
    let mut tester = SongTester::new(builder.build());
    // Row 0 Tick 0
    tester.tick();
    assert!(tester.song.voices[tester.get_voices_for_channel(0)[0]].sustained);
    
    // Move to Row 1 and process its first tick
    tester.step_row();
    tester.tick();
    
    let voices = tester.get_voices_for_channel(0);
    assert_eq!(voices.len(), 2, "Both voices should be active because Note Off enters release phase (envelope is on)");
    
    let current_voice_idx = tester.song.channels[0].voice_idx.unwrap();
    let other_voice_idx = if voices[0] == current_voice_idx { voices[1] } else { voices[0] };
    
    assert!(!tester.song.voices[other_voice_idx].sustained, "Old voice should be in release phase (not sustained)");
    assert!(tester.song.voices[current_voice_idx].sustained, "New voice should be sustained");
}

#[test]
fn test_it_volume_scaling() {
    let mut builder = MockSongBuilder::new(SongType::IT, 2);
    builder.add_empty_pattern(64);
    builder.add_instrument("Test", vec![0.5; 100]);
    builder.instruments[1].global_volume = 64; // Max IT instrument global vol
    
    // Row 0: Play Note
    builder.set_pattern_row(0, 0, 0, Pattern { note: 61, instrument: 1, volume: 64, ..Pattern::new() });
    let mut tester = SongTester::new(builder.build());
    
    tester.tick();
    let voice_idx = tester.song.channels[0].voice_idx.unwrap();
    
    // Set everything to max IT levels
    tester.song.global_volume.volume = 128; // IT internal max global vol
    tester.song.channels[0].channel_volume = 64; // Max IT channel vol
    tester.tick(); // Update volumes
    
    let voice = &tester.song.voices[voice_idx];
    // Output volume should be 1.0
    // (Fadeout 1.0 * Env 1.0 * Vol 64/64 * InstGlobal 64/64 * SampleGlobal 64/64 * Chan 64/64 * Global 128/128)
    assert!( (voice.volume.output_volume - 1.0).abs() < 0.001, "Volume should be 1.0, got {}", voice.volume.output_volume);
}

#[test]
fn test_it_voice_deactivation() {
    let mut builder = MockSongBuilder::new(SongType::IT, 2);
    builder.add_empty_pattern(64);
    
    // Instrument 1: NNA = Note Fade (3), Fadeout = 256 (Slow enough for 2 ticks)
    builder.add_instrument("FastFade", vec![0.5; 1000]);
    builder.instruments[1].nna = 3; // Note Fade
    builder.instruments[1].volume_fadeout = 256; 
    
    // Row 0: Play Note
    builder.set_pattern_row(0, 0, 0, Pattern { note: 61, instrument: 1, volume: 64, ..Pattern::new() });
    
    // Row 1: Play new note (causes old one to fade due to NNA Note Fade)
    builder.set_pattern_row(0, 1, 0, Pattern { note: 63, instrument: 1, volume: 64, ..Pattern::new() });
    
    let mut tester = SongTester::new(builder.build());
    
    tester.step_row(); // Row 0
    tester.tick();     // Row 1 Tick 0: New note triggers, old moves to background and starts fading
    assert_eq!(tester.get_active_voices(), 2, "Should have 2 voices active at start of Row 1");
    
    // Process multiple ticks. Fadeout should hit 0.
    for _ in 0..10 {
        tester.tick();
    }
    
    assert_eq!(tester.get_active_voices(), 1, "Background voice should have deactivated after fadeout completion");
}

#[test]
fn test_it_note_trigger_frequency() {
    let mut builder = MockSongBuilder::new(SongType::IT, 2);
    builder.add_empty_pattern(64);
    builder.add_instrument("MiddleC", vec![0.5; 100]);
    // Set relative note to -12 to simulate IT loader behavior
    builder.instruments[1].samples[0].relative_note = -12;
    
    // IT C-5 (Note 60) should play as Middle C (C-4/8363Hz in my engine)
    // In our remapped engine, IT Note 60 is pattern Note 61.
    builder.set_pattern_row(0, 0, 0, Pattern { note: 61, instrument: 1, volume: 64, ..Pattern::new() });
    
    let mut tester = SongTester::new(builder.build());
    tester.tick();
    
    let voice_idx = tester.song.channels[0].voice_idx.unwrap();
    let voice = &tester.song.voices[voice_idx];
    
    // Middle C in Protracker/S3M/IT is 8363 Hz.
    // Our period table maps Note 49 (C-4) to 8363 Hz.
    // IT Note 60 maps to Pattern Note 61. 
    // real_note = 61 + (rel_note - 12) = 61 - 12 = 49.
    assert!( (voice.frequency - 8363.0).abs() < 1.0, "IT C-5 should result in ~8363 Hz, got {}", voice.frequency);
}

#[test]
fn test_it_note_off_cut_remapped() {
    let mut builder = MockSongBuilder::new(SongType::IT, 2);
    builder.add_empty_pattern(64);
    builder.add_instrument("NoEnv", vec![0.5; 1000]);
    builder.instruments[1].volume_envelope.on = false;
    
    // Row 0: Play Note
    builder.set_pattern_row(0, 0, 0, Pattern { note: 61, instrument: 1, volume: 64, ..Pattern::new() });
    // Row 1: Note Cut (Mapped to 121)
    builder.set_pattern_row(0, 1, 0, Pattern { note: 121, instrument: 0, volume: 255, ..Pattern::new() });
    
    let mut tester = SongTester::new(builder.build());
    tester.tick();
    assert_eq!(tester.get_active_voices(), 1);
    
    tester.step_row(); 
    tester.tick();
    
    assert_eq!(tester.get_active_voices(), 0, "Voice should be CUT immediately on Note 121 (IT Note Cut)");
}

#[test]
fn test_it_note_limit_acceptance() {
    use crate::module_reader::is_note_valid;
    assert!(is_note_valid(120, SongType::IT));
    assert!(!is_note_valid(121, SongType::IT));
}

#[test]
fn test_xm_note_limit_rejection() {
    use crate::module_reader::is_note_valid;
    assert!(is_note_valid(96, SongType::XM));
    assert!(!is_note_valid(97, SongType::XM));
    assert!(!is_note_valid(120, SongType::XM));
}

#[test]
fn test_mod_note_limit_rejection() {
    use crate::module_reader::is_note_valid;
    assert!(is_note_valid(96, SongType::MOD));
    assert!(!is_note_valid(97, SongType::MOD));
}
