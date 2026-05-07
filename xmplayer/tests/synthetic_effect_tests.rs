use xmplayer::module_reader::SongType;
use xmplayer::envelope::EnvelopePoint;
use xmplayer::song::{InterleavedBufferAdaptar, PlaybackCmd, Song};
use xmplayer::song::test_dump::dump_tick;
use xmplayer::song_state::SongState;
use xmplayer::test_utils::MockSongBuilder;
use std::sync::{Arc, Mutex, mpsc};

fn create_test_song_handle(song_data: xmplayer::module_reader::SongData) -> (Arc<Mutex<Song>>, mpsc::Receiver<PlaybackCmd>) {
    let (sh, _consumer) = SongState::new_from_data(song_data);
    let song = sh.get_song().clone();
    let (_tx, rx) = mpsc::channel();
    (song, rx)
}

#[test]
fn test_arpeggio_xm() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(1);
    builder.add_pattern_row(0, 0, 49, 1, 255, 0x0, 0x37);
    let mut tester = builder.get_tester();

    tester.tick(); // tick 0
    let base_du = dump_tick(&tester.song).voices[0].du;
    tester.tick(); // tick 1
    let d_sharp_du = dump_tick(&tester.song).voices[0].du;
    tester.tick(); // tick 2
    let g_du = dump_tick(&tester.song).voices[0].du;
    assert!((d_sharp_du / base_du - 1.1892).abs() < 0.001);
    assert!((g_du / base_du - 1.4983).abs() < 0.001);
}

#[test]
fn test_set_volume_mod() {
    let mut builder = MockSongBuilder::new(SongType::MOD, 1);
    builder.add_empty_pattern(1);
    builder.add_pattern_row(0, 0, 49, 1, 255, 0xC, 0x20);
    let mut tester = builder.get_tester();
    tester.tick();
    tester.assert_voice_volume_near(0, 0.5, 0.01);
}

#[test]
fn test_mod_tremolo_affects_output_volume() {
    // Regression: MOD's volume path used to be hand-rolled and skipped
    // tremolo_shift, so the 0x07 Tremolo effect ran (tremolo_shift was
    // updated) but never reached the output. Switching MOD to
    // compute_base_volume() routes tremolo into the formula like other
    // formats. This test sets up max-depth tremolo on a max-volume note and
    // expects the output to deviate from a constant 1.0 across ticks.
    let mut builder = MockSongBuilder::new(SongType::MOD, 1);
    builder.add_empty_pattern(1);
    // Row 0: C-4, instrument 1, set volume 64, then Tremolo 0xFF (max
    // speed, max depth). MOD effect 7 is Tremolo.
    builder.add_pattern_row(0, 0, 49, 1, 0, 0x07, 0xFF);
    let mut tester = builder.get_tester();

    let mut min_vol: f32 = f32::INFINITY;
    let mut max_vol: f32 = -f32::INFINITY;
    for _ in 0..6 {
        tester.tick();
        let v = tester.song.voices[0].volume.output_volume;
        min_vol = min_vol.min(v);
        max_vol = max_vol.max(v);
    }
    // Without tremolo plumbing, output stays constant at 1.0; with tremolo,
    // depth=15 produces a clearly visible swing across ticks.
    assert!(max_vol - min_vol > 0.05,
            "Tremolo should modulate output volume; saw range {:.3}..{:.3}",
            min_vol, max_vol);
}

#[test]
fn test_sample_offset() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(1);
    builder.add_pattern_row(0, 0, 49, 1, 255, 0x9, 0x10);
    let mut tester = builder.get_tester();
    tester.tick();
    assert_eq!(dump_tick(&tester.song).voices[0].sample_pos, 4096.0 + 4.0);
}

#[test]
fn test_retrig_xm() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(1);
    builder.add_pattern_row(0, 0, 49, 1, 255, 0x1B, 0x02); // R02: retrig every 2 ticks
    builder.instruments[1].samples[0].data = vec![0.0; 100000];
    builder.instruments[1].samples[0].length = 100000;
    builder.instruments[1].samples[0].setup_loops_and_padding();

    let (song_arc, mut rx) = create_test_song_handle(builder.build());
    let mut dummy_buffer = vec![0.0f32; 1920];

    // Tick 0
    let p0 = {
        let mut adapter = InterleavedBufferAdaptar { buf: &mut dummy_buffer };
        let mut song = song_arc.lock().unwrap();
        song.get_next_tick(&mut adapter, &mut rx);
        dump_tick(&song).voices[0].sample_pos
    };

    // Tick 1: position should advance naturally
    let p1 = {
        let mut adapter = InterleavedBufferAdaptar { buf: &mut dummy_buffer };
        let mut song = song_arc.lock().unwrap();
        song.get_next_tick(&mut adapter, &mut rx);
        dump_tick(&song).voices[0].sample_pos
    };

    // Tick 2: retrig resets sample to start, then renders
    let p2 = {
        let mut adapter = InterleavedBufferAdaptar { buf: &mut dummy_buffer };
        let mut song = song_arc.lock().unwrap();
        song.get_next_tick(&mut adapter, &mut rx);
        dump_tick(&song).voices[0].sample_pos
    };

    assert!(p1 > p0);
    assert!(p2 < p1);
}

#[test]
fn test_key_off() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(1);
    builder.instruments[1].volume_envelope.on = true;
    builder.add_pattern_row(0, 0, 49, 1, 255, 0x14, 0x03); // K03
    let mut tester = builder.get_tester();

    tester.tick(); // tick 0
    assert!(tester.song.voices[0].sustained);
    tester.tick(); // tick 1
    assert!(tester.song.voices[0].sustained);
    tester.tick(); // tick 2
    assert!(tester.song.voices[0].sustained);
    tester.tick(); // tick 3
    assert!(!tester.song.voices[0].sustained);
}

#[test]
fn test_panning_slide() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(1);
    builder.add_pattern_row(0, 0, 49, 1, 255, 0x19, 0x10); // P10
    let mut tester = builder.get_tester();

    tester.tick(); // tick 0
    assert_eq!(dump_tick(&tester.song).voices[0].panning, 128);
    tester.tick(); // tick 1
    assert_eq!(dump_tick(&tester.song).voices[0].panning, 129);
}

#[test]
fn test_envelope_position() {
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.add_empty_pattern(1);
    builder.instruments[1].volume_envelope.on = true;
    builder.instruments[1].volume_envelope.points[0] = EnvelopePoint { frame: 0, value: 64 };
    builder.instruments[1].volume_envelope.points[1] = EnvelopePoint { frame: 100, value: 64 };
    builder.instruments[1].volume_envelope.size = 2;
    builder.add_pattern_row(0, 0, 49, 1, 255, 0x15, 0x28); // L28
    let mut tester = builder.get_tester();
    tester.tick();
    assert_eq!(dump_tick(&tester.song).voices[0].volume_envelope_pos, 41);
}
#[test]
fn test_s3m_note_off_cut() {
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.add_empty_pattern(3);
    builder.add_pattern_row(0, 0, 49, 1, 64, 0, 0); // trigger
    builder.add_pattern_row(0, 1, 97, 0, 255, 0, 0); // note off
    builder.add_pattern_row(0, 2, 121, 0, 255, 0, 0); // note cut
    let mut tester = builder.get_tester();

    tester.tick(); // row 0 tick 0
    tester.assert_voice_on(0, true);
    tester.step_to_row(1);
    tester.tick(); // row 1 tick 0
    tester.assert_voice_on(0, false);
}

#[test]
fn test_s3m_porta_with_instrument_retrigs_volume() {
    // Regression: on a porta-to-note row with an instrument number, the
    // instrument byte must re-read the sample's default volume on the
    // existing voice. The note itself doesn't audibly retrigger (no
    // sample_position reset, no envelope phase reset of `sustained`),
    // but the instrument number does — matching ST3 / FT2 / IT and the
    // canonical block in master at song.rs:1539.
    //
    // Without this, vol-col changes between porta+inst rows would stick
    // forever instead of oscillating back to the sample default. This
    // test reproduces the observed 2ND_PM.S3M order-18 channel-7 pattern.
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.instruments[1].samples[0].data = vec![0.0; 100000];
    builder.instruments[1].samples[0].length = 100000;
    builder.add_empty_pattern(4);
    // Row 0: regular trigger; voice.volume.volume <- sample default (64).
    builder.add_pattern_row(0, 0, 49, 1, 255, 0, 0);
    // Row 1: vol-col 20 → voice.volume.volume = 20.
    builder.add_pattern_row(0, 1, 0, 0, 20, 0, 0);
    // Row 2: porta-to-note (S3M effect G == 7) + same instrument byte, no
    //   vol col. The retrig path must reset volume back to 64 even though
    //   the audio doesn't retrigger.
    builder.add_pattern_row(0, 2, 51, 1, 255, 7, 4);
    let mut tester = builder.get_tester();

    tester.run_row();
    assert_eq!(tester.song.voices[0].volume.volume, 64,
               "row 0 trigger should set volume to sample default");
    tester.run_row();
    assert_eq!(tester.song.voices[0].volume.volume, 20,
               "row 1 vol-col 20 should land");
    tester.run_row();
    assert_eq!(tester.song.voices[0].volume.volume, 64,
               "row 2 porta+instrument should retrig volume to sample default");
}

#[test]
fn test_xm_porta_with_instrument_retrigs_volume() {
    // XM mirror of test_s3m_porta_with_instrument_retrigs_volume. XM uses
    // effect 0x03 for tone porta (S3M uses 0x07) and encodes vol-col set
    // volume as 0x10..=0x50 (raw value + 0x10), so a vol-col byte of
    // 0x10+20 = 0x24 sets the voice volume to 20. Same audible bug
    // observed in 2ND_PM.xm; same fix path (porta_retrig_for_instrument).
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    builder.instruments[1].samples[0].data = vec![0.0; 100000];
    builder.instruments[1].samples[0].length = 100000;
    builder.add_empty_pattern(4);
    builder.add_pattern_row(0, 0, 49, 1, 0, 0, 0);
    // Vol-col 0x24 (= 0x10 + 20) → set volume to 20.
    builder.add_pattern_row(0, 1, 0, 0, 0x24, 0, 0);
    // XM tone porta: effect 0x03.
    builder.add_pattern_row(0, 2, 51, 1, 0, 3, 4);
    let mut tester = builder.get_tester();

    tester.run_row();
    assert_eq!(tester.song.voices[0].volume.volume, 64);
    tester.run_row();
    assert_eq!(tester.song.voices[0].volume.volume, 20);
    tester.run_row();
    assert_eq!(tester.song.voices[0].volume.volume, 64,
               "XM porta+instrument should retrig volume to sample default");
}

#[test]
fn test_s3m_vibrato_no_persistent_detune_after_stop() {
    // Regression: with the previous depth-×4 / speed-×1 vibrato scaling,
    // the wave was 4× wider amplitude and ~4× slower oscillation than
    // master / ST3 / FT2. When the H effect ended, the wave often froze
    // mid-cycle and left a constant ~25-70 cents detune on the voice
    // until something else moved the period. Audible as wrong pitch on
    // 2ND_PM.S3M order 0x13 ch7.
    //
    // With the speed-×4 / depth-raw fix the wave cycles fast enough that
    // when H stops, the residual shift resolves close to zero within a
    // tick. Assert the post-H freq matches the unmodulated baseline.
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.instruments[1].samples[0].data = vec![0.0; 100000];
    builder.instruments[1].samples[0].length = 100000;
    builder.add_empty_pattern(4);
    // Row 0: regular trigger; baseline freq.
    builder.add_pattern_row(0, 0, 49, 1, 64, 0, 0);
    // Rows 1-2: vibrato H84 (speed 8, depth 4) running.
    builder.add_pattern_row(0, 1, 0, 0, 255, 8, 0x84);
    builder.add_pattern_row(0, 2, 0, 0, 255, 8, 0x00); // recall
    // Row 3: no effect — voice should settle back at baseline.
    builder.add_pattern_row(0, 3, 0, 0, 255, 0, 0);
    let mut tester = builder.get_tester();

    tester.run_row(); // row 0 (trigger). Freq is the post-trigger baseline.
    let base = tester.song.voices[0].frequency;
    tester.run_row(); // row 1: vibrato active
    tester.run_row(); // row 2: vibrato active
    tester.run_row(); // row 3: no effect
    let after = tester.song.voices[0].frequency;
    let delta = (after - base).abs();

    // Allow a tiny rounding window (the wave end-position may not land
    // *exactly* on zero), but anything more than ~5 Hz at this period
    // would be the audible bug returning.
    assert!(delta < 5.0,
            "post-vibrato freq should match unmodulated baseline {}, got {} (Δ={})",
            base, after, delta);
}

#[test]
fn test_s3m_porta_without_instrument_keeps_volume() {
    // Counterpart guard: porta-to-note WITHOUT an instrument number must
    // NOT touch the voice volume — only the instrument byte triggers the
    // retrig path. This catches a too-eager retrig that fires on every
    // porta row regardless of instrument presence.
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.instruments[1].samples[0].data = vec![0.0; 100000];
    builder.instruments[1].samples[0].length = 100000;
    builder.add_empty_pattern(4);
    builder.add_pattern_row(0, 0, 49, 1, 255, 0, 0);
    builder.add_pattern_row(0, 1, 0, 0, 20, 0, 0);
    // Row 2: porta-to-note WITHOUT instrument byte (instrument = 0).
    builder.add_pattern_row(0, 2, 51, 0, 255, 7, 4);
    let mut tester = builder.get_tester();

    tester.run_row();
    tester.run_row();
    assert_eq!(tester.song.voices[0].volume.volume, 20);
    tester.run_row();
    assert_eq!(tester.song.voices[0].volume.volume, 20,
               "porta without instrument byte must not retrig volume");
}
