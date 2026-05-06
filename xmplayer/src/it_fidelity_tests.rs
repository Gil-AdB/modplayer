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
fn test_it_filter_z_sets_cutoff_and_resonance() {
    // IT Zxx (effect 0x1A): values 0x00..0x7F set the per-voice filter
    // cutoff; values 0x80..=0x8F set the resonance (low nibble << 3).
    // Verified against apply_effect's Filter arm.
    let mut builder = MockSongBuilder::new(SongType::IT, 1);
    builder.add_empty_pattern(64);
    builder.add_instrument("F", vec![0.5; 100]);
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 64, effect: 0x1A, effect_param: 0x40,
    });
    builder.set_pattern_row(0, 1, 0, Pattern {
        note: 0, instrument: 0, volume: 255, effect: 0x1A, effect_param: 0x8F,
    });

    let mut tester = SongTester::new(builder.build());
    tester.tick();
    assert_eq!(tester.song.voices[0].filter_cutoff, 0x40,
               "Z40 should set filter cutoff to 0x40");
    let resonance_before = tester.song.voices[0].filter_resonance;

    tester.step_to_row(1);
    tester.tick();
    assert_eq!(tester.song.voices[0].filter_resonance, 0x0F << 3,
               "Z8F should set resonance to (0x0F << 3) = {} (was {})",
               0x0F << 3, resonance_before);
}

#[test]
fn test_it_sample_global_volume_no_double_apply() {
    // Regression: prior to the fix, the IT formula multiplied by
    // sample_global_volume/64 once inside compute_base_volume() and again in
    // the backend, so a sample with global_volume=32 came out at (32/64)^2.
    let mut builder = MockSongBuilder::new(SongType::IT, 1);
    builder.add_empty_pattern(64);
    builder.add_instrument("HalfVol", vec![0.5; 100]);
    builder.instruments[1].samples[0].global_volume = 32; // half
    builder.instruments[1].global_volume = 128;            // max IT instrument vol
    builder.global_volume = 128;                           // max IT song vol

    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 61, instrument: 1, volume: 64, ..Pattern::new()
    });
    let mut tester = SongTester::new(builder.build());

    tester.tick();
    // Expected: 1.0 (compute_base, after sample_global) * 1.0 (inst) * 1.0 (global) = 0.5
    // Buggy:    0.5 (compute_base) * 32/64 (sample again) * 1.0 * 1.0 = 0.25
    tester.assert_voice_volume_near(0, 0.5, 0.001);
}

#[test]
fn test_it_dxy_both_nibbles_non_zero_slides_up_by_x() {
    // Per IT spec: when both Dxy nibbles are non-zero and neither is F,
    // the lower nibble is ignored. D32 should slide up by 3 per tick after
    // first tick. Previously fell through all four match arms and did
    // nothing.
    let mut builder = MockSongBuilder::new(SongType::IT, 1);
    builder.add_empty_pattern(64);
    builder.add_instrument("Test", vec![0.5; 100]);

    // Row 0: trigger note at half volume so we can slide up and observe.
    builder.set_pattern_row(0, 0, 0, Pattern { note: 61, instrument: 1, volume: 32, ..Pattern::new() });
    // Row 1: D32 (effect 0x04, param 0x32) - slide up by 3 per tick.
    // Pattern::new() defaults volume to 255 (= no vol-col).
    builder.set_pattern_row(0, 1, 0, Pattern { effect: 0x04, effect_param: 0x32, ..Pattern::new() });

    let mut tester = SongTester::new(builder.build());
    tester.song.speed = 4;

    // Step into row 1; tick 0 doesn't slide (IT D is "after first tick").
    // SongTester::tick() processes the current tick THEN advances, so three
    // tick() calls cover ticks 0, 1, 2 - i.e. one no-op tick and two slides.
    tester.step_to_row(1);
    let v0 = tester.song.voices[0].volume.volume;
    tester.tick(); // processes tick 0 (no slide), advances to 1
    tester.tick(); // processes tick 1 (slide +3)
    tester.tick(); // processes tick 2 (slide +3)
    let v1 = tester.song.voices[0].volume.volume;
    // Two slide ticks of +3 = +6. With the bug, no slide ran (0).
    assert_eq!(v1 as i32 - v0 as i32, 6, "D32 should slide up by 3 per non-first tick, got {}", v1 as i32 - v0 as i32);
}

#[test]
fn test_it_volume_scaling() {
    let mut builder = MockSongBuilder::new(SongType::IT, 2);
    builder.add_empty_pattern(64);
    builder.add_instrument("Test", vec![0.5; 100]);
    builder.instruments[1].global_volume = 128; // Max IT instrument global vol
    
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

// Helpers for expressing the old is_note_valid intent against the new
// NoteAction API. is_note_valid was "is this a triggerable note?".
fn note_at(n: u8) -> Pattern { Pattern { note: n, ..Pattern::new() } }
fn is_trigger(p: &Pattern, st: SongType) -> bool {
    use crate::pattern::NoteAction;
    matches!(p.note_action(st), NoteAction::Trigger(_))
}

#[test]
fn test_it_note_limit_acceptance() {
    assert!(is_trigger(&note_at(120), SongType::IT));
    // Note 121 is engine-encoding for Note Cut, not a trigger.
    assert!(!is_trigger(&note_at(121), SongType::IT));
}

#[test]
fn test_xm_note_limit_rejection() {
    assert!(is_trigger(&note_at(96), SongType::XM));
    // 97 is Note Off in XM encoding.
    assert!(!is_trigger(&note_at(97), SongType::XM));
    assert!(!is_trigger(&note_at(120), SongType::XM));
}

#[test]
fn test_xm_relative_note_uses_pattern_note() {
    use crate::test_utils::{MockSongBuilder, SongTester};
    assert!(is_trigger(&note_at(49), SongType::XM));
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.instruments[1].samples[0].relative_note = 12;
    builder.add_pattern_row(0, 0, 49, 1, 255, 0, 0);
    let mut tester = SongTester::new(builder.build());
    tester.tick();
    // XM: RealNote = PatternNote + RelativeTone (49 + 12), not (key_index + RelativeTone).
    assert_eq!(tester.song.channels[0].note.note, 61);
    assert_eq!(tester.song.channels[0].note.original_note, 49);
}

#[test]
fn test_mod_note_limit_rejection() {
    assert!(is_trigger(&note_at(96), SongType::MOD));
    assert!(!is_trigger(&note_at(97), SongType::MOD));
}

#[test]
fn test_s3m_note_above_96_is_valid() {
    // S3M packs octave in the high nybble; decoded values may exceed 96.
    assert!(is_trigger(&note_at(100), SongType::S3M));
    // 121 is Note Cut in engine encoding.
    assert!(!is_trigger(&note_at(121), SongType::S3M));
}
