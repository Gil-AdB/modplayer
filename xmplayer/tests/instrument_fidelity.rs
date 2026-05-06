use xmplayer::module_reader::{SongType};
use xmplayer::pattern::Pattern;
use xmplayer::test_utils::{MockSongBuilder};
use xmplayer::envelope::{Envelope, EnvelopePoint};

#[test]
fn test_it_instrument_global_volume() {
    let mut builder = MockSongBuilder::new(SongType::IT, 1);
    builder.add_empty_pattern(64);
    
    // Instrument 1: Global Volume = 32 (50% of 64, but IT uses 0..128)
    // Wait! SongData::instruments[1].global_volume is used.
    // IT Instrument Global Volume is 0..128.
    builder.instruments[1].global_volume = 32; // 25% of 128
    
    // Set song global volume to 64 (50% of 128)
    builder.global_volume = 64; 

    // Row 0: C-4 with Instrument 1
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 255, effect: 0, effect_param: 0,
    });
    
    let mut tester = builder.get_tester();
    
    // Tick 0: Note triggered
    tester.tick();
    
    // Final volume calculation for IT:
    // compute_base_volume() * inst_vol/128 * sample_vol/64 * global_vol/128
    // compute_base_volume() = 1.0 (fadeout=1.0, envelope=1.0, sample_vol=64/64=1.0)
    // inst_vol = 32 / 128 = 0.25
    // sample_vol = 64 / 64 = 1.0
    // global_vol = 64 / 128 = 0.5
    // Result = 1.0 * 0.25 * 1.0 * 0.5 = 0.125
    
    tester.assert_voice_volume_near(0, 0.125, 0.001);
}

#[test]
fn test_auto_vibrato_execution() {
    let mut builder = MockSongBuilder::new(SongType::IT, 1);
    builder.add_empty_pattern(64);
    
    // Instrument 1: Sine, no sweep, depth 16, rate 64
    builder.instruments[1].vibrato_envelope.vibrato_type = 0;
    builder.instruments[1].vibrato_envelope.vibrato_sweep = 0;
    builder.instruments[1].vibrato_envelope.vibrato_depth = 16;
    builder.instruments[1].vibrato_envelope.vibrato_rate = 64;

    // Row 0: C-4 with Instrument 1
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 255, effect: 0, effect_param: 0,
    });
    
    let mut tester = builder.get_tester();

    // Tick 0: Note triggered
    tester.tick();
    let initial_freq = tester.song.voices[0].frequency;
    
    // Tick 1: Auto vibrato should move pos by 64.
    // Sine table at 64 is 1.0? 
    // Wait! VIB_SINE_TAB has 256 entries.
    // VIB_SINE_TAB[64] = 255?
    tester.tick();
    let freq1 = tester.song.voices[0].frequency;
    
    // Auto vibrato should change frequency
    assert_ne!(initial_freq, freq1);
}

#[test]
fn test_it_sustain_loop() {
    let mut builder = MockSongBuilder::new(SongType::IT, 1);
    builder.add_empty_pattern(64);
    
    // Create an envelope with a sustain loop between points 1 and 2
    let mut points = [EnvelopePoint::new(); 25];
    points[0] = EnvelopePoint { frame: 0, value: 64 };
    points[1] = EnvelopePoint { frame: 10, value: 32 };
    points[2] = EnvelopePoint { frame: 20, value: 64 };
    points[3] = EnvelopePoint { frame: 30, value: 0 };
    
    let env = Envelope::create(
        points,
        4,    // size
        1,    // sustain_point (not used if sustain_loop is present)
        1,    // sustain_loop_start
        2,    // sustain_loop_end
        0,    // loop_start
        0,    // loop_end
        1 | 8 // on | has_sustain_loop
    );
    builder.instruments[1].volume_envelope = env;

    // Row 0: Note ON
    builder.set_pattern_row(0, 0, 0, Pattern {
        note: 49, instrument: 1, volume: 255, effect: 0, effect_param: 0,
    });
    
    let mut tester = builder.get_tester();

    // Tick 0: Note triggered, frame 0
    tester.tick();
    assert_eq!(tester.song.voices[0].volume_envelope_state.frame, 1);
    
    // Skip to frame 20 (Sustain Loop End)
    for _ in 1..20 { tester.tick(); }
    assert_eq!(tester.song.voices[0].volume_envelope_state.frame, 20);
    
    // Next tick: should jump to frame 10 (Sustain Loop Start)
    tester.tick();
    assert_eq!(tester.song.voices[0].volume_envelope_state.frame, 10);
    
    // Now Key Off
    tester.song.voices[0].key_off(&builder.instruments, false);
    
    // Should now continue past frame 20
    for _ in 0..15 { tester.tick(); }
    assert!(tester.song.voices[0].volume_envelope_state.frame > 20);
    assert!(tester.song.voices[0].volume_envelope_state.frame <= 30);
}
