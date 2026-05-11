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
fn test_mod_instrument_does_not_overwrite_channel_panning() {
    // ProTracker hard-pans channels in LRRL and never moves them.
    // Triggering an instrument on a MOD channel must NOT overwrite
    // the voice's panning with the sample's panning field.
    let mut builder = MockSongBuilder::new(SongType::MOD, 1);
    builder.instruments[1].samples[0].panning = 200; // anomalous
    builder.add_empty_pattern(1);
    builder.add_pattern_row(0, 0, 49, 1, 255, 0, 0);
    let mut tester = builder.get_tester();
    tester.tick();
    let pan = tester.song.voices[0].panning.panning as u8;
    assert_eq!(pan, 128, "MOD voice panning should stay at the channel default, got {}", pan);
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

#[test]
fn test_seek_forward_pattern_terminates_when_looping() {
    // Regression: seeking forward while `loop_pattern` is set used to
    // hot-spin the player at 100% CPU. next_pattern() is a no-op when
    // loop_pattern is true, so song_position never advances past
    // `current` and the seek_forward_pattern condition never becomes
    // true. The fix saves the flag, clears it around fast_forward_until,
    // and restores it.
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.add_empty_pattern(4);
    builder.add_empty_pattern(4);
    builder.set_order(vec![0, 1]);
    let mut tester = builder.get_tester();

    tester.song.loop_pattern = true;
    let pos_before = tester.song.song_position;

    tester.song.seek_forward_pattern();

    assert!(tester.song.song_position > pos_before,
            "seek_forward_pattern should advance even with loop_pattern set");
    assert!(tester.song.loop_pattern, "loop_pattern must be restored after seek");
}

#[test]
fn test_s3m_note_delay_vol_col_at_trigger() {
    // S3M SDx note-delay: vol col fires at the trigger tick (matches XM
    // EDx and ft2-clone). The previous voice keeps its old volume during
    // the delay window; at the trigger tick the new voice is allocated,
    // retrig loads the instrument default, then the vol col overrides.
    //
    // The alternative (ST3-style: vol col at first_tick on the previous
    // voice, retrig dominates at trigger) was tested against master and
    // produced an audible one-tick volume spike at the SDx row → next
    // row boundary in 2ND_PM.S3M order 0x23 (the next row's vol col
    // dropped voice volume back from instrument-default 64 to 12). The
    // .xm version of the same song doesn't have this because XM EDx
    // applies vol col at the trigger. We match XM. Controlled by
    // `S3M_DELAY.vol_col_at_trigger = true` in backend.rs.
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.instruments[1].samples[0].data = vec![0.0; 100000];
    builder.instruments[1].samples[0].length = 100000;
    builder.add_empty_pattern(2);
    // Row 0: trigger C-4 at full vol so we have something ringing.
    builder.add_pattern_row(0, 0, 49, 1, 64, 0, 0);
    // Row 1: D-4 with vol=12 + SD3 (note delay 3 ticks).
    // S3M effect 0x13 = S, param 0xD3 = SD3.
    builder.add_pattern_row(0, 1, 51, 1, 12, 0x13, 0xD3);
    let mut tester = builder.get_tester();

    tester.run_row();
    assert_eq!(tester.song.voices[0].volume.volume, 64);

    // Ticks 0..2 of row 1: vol col deferred; old voice keeps old vol=64.
    tester.tick();
    assert_eq!(tester.song.voices[0].volume.volume, 64,
               "vol col must NOT fire before the note-delay trigger; got {}",
               tester.song.voices[0].volume.volume);
    tester.tick();
    assert_eq!(tester.song.voices[0].volume.volume, 64);
    tester.tick();
    assert_eq!(tester.song.voices[0].volume.volume, 64);

    // Tick 3: trigger fires. Retrig sets vol=64 (inst default); vol col
    // then overrides to 12. Voice plays the new note at vol col value.
    tester.tick();
    assert_eq!(tester.song.voices[0].volume.volume, 12,
               "vol col should land at the trigger tick (over retrig); got {}",
               tester.song.voices[0].volume.volume);
}

#[test]
fn test_s3m_c5_speed_formula_period() {
    // S3M loader records full-precision c5_speed; the s3m backend uses the
    // closed-form OpenMPT formula instead of the LUT so c5_speed != 8363
    // doesn't accumulate 1/16-semitone quantization error.
    //
    // Reference: OpenMPT Snd_fx.cpp:6456:
    //   period = 8363 * 32 * FreqS3MTable[note0 % 12] / (c5_speed << (note0 / 12))
    // FreqS3MTable[0] = 1712. For our engine note 49 (S3M file 0x40, "C-5"),
    // formula note0 = 49 + 11 = 60. note0 % 12 = 0; note0 / 12 = 5.
    //   period = 8363 * 32 * 1712 / (c5_speed * 32)
    //          = 8363 * 1712 / c5_speed
    //
    // c5=8363 → 1712 (unity, also matches LUT exactly)
    // c5=10000 → 1431
    // c5=16000 → 894
    let cases: &[(u32, u16)] = &[(8363, 1712), (10000, 1431), (16000, 894)];
    for (c5, expected) in cases {
        let mut builder = MockSongBuilder::new(SongType::S3M, 1);
        builder.instruments[1].samples[0].data = vec![0.0; 4096];
        builder.instruments[1].samples[0].length = 4096;
        builder.instruments[1].samples[0].c5_speed = *c5;
        builder.add_empty_pattern(2);
        // Engine note 49 = S3M file byte 0x40 = "C-5".
        builder.add_pattern_row(0, 0, 49, 1, 64, 0, 0);
        let mut tester = builder.get_tester();
        tester.tick();
        let p = tester.get_channel_period(0);
        assert_eq!(p, *expected,
            "c5_speed={} expected period {} got {}", c5, expected, p);
    }
}

#[test]
fn test_it_c5_speed_formula_period() {
    // IT amiga-mode c5_speed formula path. IT pattern byte 60 → engine note 61,
    // mapped through the instrument's keyboard map to it_mapping.0 = 60, then
    // mapped_note = 61 reaches the formula with offset -1 → formula note 60.
    // OpenMPT expectation matches the S3M case at the same c5_speed.
    // Linear-mode IT goes through `Note::it_linear_frequency` instead — see
    // `test_it_linear_c5_speed_freq`.
    let cases: &[(u32, u16)] = &[(8363, 1712), (10000, 1431), (16000, 894)];
    for (c5, expected) in cases {
        let mut builder = MockSongBuilder::new(SongType::IT, 1);
        builder.use_amiga_freq(true);
        builder.instruments[1].samples[0].data = vec![0.0; 4096];
        builder.instruments[1].samples[0].length = 4096;
        builder.instruments[1].samples[0].c5_speed = *c5;
        builder.add_empty_pattern(2);
        // IT engine note 61 = pattern byte 60 = "C-5".
        builder.add_pattern_row(0, 0, 61, 1, 64, 0, 0);
        let mut tester = builder.get_tester();
        tester.tick();
        let p = tester.get_channel_period(0);
        assert_eq!(p, *expected,
            "IT c5_speed={} expected period {} got {}", c5, expected, p);
    }
}

#[test]
fn test_it_linear_c5_speed_freq() {
    // IT linear-mode pitch path (default for IT files with flag 8 set).
    // OpenMPT Snd_fx.cpp:6446: at C-5 (engine note 61) the freq equals
    // c5_speed; each octave doubles. Pre-fix our pipeline computed an
    // amiga-scale period and looked it up in the FT2-linear period table,
    // producing ultrasonic output (dU 8.323 = 399 kHz). With the
    // `linear_hz` override the trigger stashes the computed Hz directly
    // and `Note::frequency` returns it without the period-table lookup.
    let cases: &[(u32, u8, f32)] = &[
        (8363, 61, 8363.0),    // C-5 = c5_speed
        (10000, 61, 10000.0),  // unit at C-5
        (8363, 49, 4181.5),    // C-4 = c5/2 (one octave below C-5)
        (8363, 73, 16726.0),   // C-6 = c5*2
    ];
    for &(c5, note, expected_hz) in cases {
        let mut builder = MockSongBuilder::new(SongType::IT, 1);
        // use_amiga_freq(false) is the default; explicit for clarity.
        builder.use_amiga_freq(false);
        builder.instruments[1].samples[0].data = vec![0.0; 4096];
        builder.instruments[1].samples[0].length = 4096;
        builder.instruments[1].samples[0].c5_speed = c5;
        builder.add_empty_pattern(2);
        builder.add_pattern_row(0, 0, note, 1, 64, 0, 0);
        let mut tester = builder.get_tester();
        tester.tick();
        let v_idx = tester.song.voices.iter().position(|v| v.on).expect("voice on");
        let actual = tester.song.voices[v_idx].frequency;
        // Tolerate ~1 Hz from integer-table rounding in the formula.
        assert!((actual - expected_hz).abs() < 1.5,
            "IT linear c5={} note={} expected {} Hz, got {}",
            c5, note, expected_hz, actual);
    }
}

#[test]
fn test_s3m_instrument_only_row_reloads_sample_volume() {
    // S3M quirk: a row with `instrument != 0` and `note = 0` reloads the
    // sample's default volume on the live voice. Without this, a perpetual
    // D0A volume slide (note slid down silently every other row) drains
    // voice volume to 0 and never recovers — repro is 2ND_PM.S3M ch1
    // around order 64. OpenMPT does this in Snd_fx.cpp:2873-2964
    // (`retrigEnv = note == NOTE_NONE && instr != 0` →
    //  chn.nVolume = oldSample->nVolume).
    //
    // Setup: trigger note + instr at row 0. At row 1 we slide volume down
    // hard (D0F → -15/tick × 6 ticks would overflow). At row 2 we put just
    // the instrument byte. After row 2's first tick the voice volume must
    // be back at the sample default (64), not the depleted slide remnant.
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.instruments[1].samples[0].data = vec![0.0; 4096];
    builder.instruments[1].samples[0].length = 4096;
    builder.instruments[1].samples[0].volume = 64;
    builder.add_empty_pattern(3);
    // Row 0: trigger.
    builder.add_pattern_row(0, 0, 49, 1, 64, 0, 0);
    // Row 1: D0F = volume slide down by 15/tick (very fast).
    // vol=255 means "no vol col present" (S3M sentinel).
    builder.add_pattern_row(0, 1, 0, 0, 255, 0x04, 0x0F);
    // Row 2: instrument byte only, no vol col — should reload default vol=64.
    builder.add_pattern_row(0, 2, 0, 1, 255, 0, 0);
    let mut tester = builder.get_tester();

    tester.run_row();
    assert_eq!(tester.song.voices[0].volume.volume, 64, "row0 trigger");
    tester.run_row();
    assert!(tester.song.voices[0].volume.volume < 64, "row1 slide drops vol; got {}", tester.song.voices[0].volume.volume);
    tester.tick(); // first tick of row 2: instr-only reload
    assert_eq!(tester.song.voices[0].volume.volume, 64,
        "row2 instr-only must reload sample default volume; got {}",
        tester.song.voices[0].volume.volume);
}

#[test]
fn test_voice_cut_reason_records_sample_end() {
    // A no-loop sample played past its end gets cut by the mixer with
    // VoiceCutReason::SampleEnd. Without instrumentation, this state
    // looks identical to "voice still alive" in state_dump output
    // (sample_position is frozen at the trigger value because the dump
    // path never invokes the mixer). The cut_reason field gives an
    // unambiguous answer.
    use xmplayer::channel_state::VoiceCutReason;
    let mut builder = MockSongBuilder::new(SongType::XM, 1);
    let short_data = vec![0.5f32; 64];
    builder.instruments[1].samples[0].data = short_data;
    builder.instruments[1].samples[0].length = 64;
    builder.instruments[1].samples[0].setup_loops_and_padding();
    builder.add_empty_pattern(2);
    builder.add_pattern_row(0, 0, 60, 1, 64, 0, 0); // C-5
    let mut tester = builder.get_tester();

    tester.tick();
    let v_idx = tester.song.voices.iter().position(|v| v.on).expect("voice on after trigger");
    assert!(tester.song.voices[v_idx].cut_reason.is_none(),
        "cut_reason still None during playback");

    // Force the sample position past the end and run a render pass via
    // output_channels so the mixer's SampleEnd branch fires.
    use xmplayer::song::InterleavedBufferAdaptar;
    tester.song.voices[v_idx].sample_position = 1000.0;
    tester.song.is_fast_forwarding = false;
    let mut buf = vec![0.0f32; 32];
    let mut adapter = InterleavedBufferAdaptar { buf: &mut buf };
    tester.song.output_channels(0, &mut adapter, 8);

    assert!(!tester.song.voices[v_idx].on, "voice cut by mixer");
    assert_eq!(tester.song.voices[v_idx].cut_reason, Some(VoiceCutReason::SampleEnd),
        "cut_reason recorded as SampleEnd");
    assert_eq!(tester.song.voices[v_idx].last_render_tick, 0,
        "last_render_tick stamped by mixer (current_buf_position=0 here)");
}

#[test]
fn test_s3m_s2_finetune_changes_period_via_table() {
    // S3M S2x sets channel c5_speed from S3M_FINETUNE_TABLE and recomputes
    // the live period. Reference: OpenMPT Snd_fx.cpp:5189-5206 +
    // Tables.cpp:340 (S3MFineTuneTable).
    //
    // Trigger note 49 with sample c5_speed = 8363 → period = 1712.
    // Row 1: S20 → c5_speed becomes table[0] = 7895 (lower → period higher)
    //   formula(60, 7895) = 8363 * 32 * 1712 / (7895 * 32) = 8363*1712/7895 ≈ 1813
    // Row 2: S2F → c5_speed becomes table[15] = 8757
    //   formula(60, 8757) = 8363*1712/8757 ≈ 1635
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.use_amiga_freq(true);
    builder.instruments[1].samples[0].data = vec![0.0; 4096];
    builder.instruments[1].samples[0].length = 4096;
    builder.instruments[1].samples[0].c5_speed = 8363;
    builder.add_empty_pattern(3);
    builder.add_pattern_row(0, 0, 49, 1, 64, 0, 0);
    // S3M effect 'S' = 0x13 in normalized numbering, S2x param 0x20 (param=0).
    builder.add_pattern_row(0, 1, 0, 0, 255, 0x13, 0x20);
    builder.add_pattern_row(0, 2, 0, 0, 255, 0x13, 0x2F);
    let mut tester = builder.get_tester();

    tester.run_row();
    assert_eq!(tester.get_channel_period(0), 1712, "row0 trigger period");
    tester.tick(); // first tick of row 1
    let p1 = tester.get_channel_period(0);
    // 8363*1712/7895 = 14318056/7895 = 1813.55... → 1813 (truncated by integer div)
    assert_eq!(p1, 1813, "S20 should set period via table[0]=7895; got {}", p1);
    tester.run_row(); // finish row 1
    tester.tick(); // first tick of row 2
    let p2 = tester.get_channel_period(0);
    // 8363*1712/8757 = 14317456/8757 = 1634.97... → 1634
    assert_eq!(p2, 1634, "S2F should set period via table[15]=8757; got {}", p2);
}

#[test]
fn test_s3m_arpeggio_uses_formula_in_amiga_mode() {
    // S3M arpeggio J37 at engine note 49 (S3M file 0x40 → formula note 60).
    // tick%3==0 → period at +0 semitones = formula(60) = 1712
    // tick%3==1 → period at +3 semitones = formula(63) = 32*1440/2^5 = 1440
    // tick%3==2 → period at +7 semitones = formula(67) = 32*1140/2^5 = 1140
    //
    // Pre-fix (FT2-style -(x*64) shift in amiga mode):
    // tick==1 would give period 1712 - 192 = 1520 (≈+200 cents instead of 300),
    // tick==2 would give 1712 - 448 = 1264 (≈+520 cents instead of 700).
    // The amiga FT2 quirk is preserved for XM/MOD; S3M/IT amiga use the
    // formula via the override in backend.rs Arpeggio handler.
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.use_amiga_freq(true);
    builder.instruments[1].samples[0].data = vec![0.0; 4096];
    builder.instruments[1].samples[0].length = 4096;
    builder.instruments[1].samples[0].c5_speed = 8363;
    builder.add_empty_pattern(1);
    // S3M effect 'J' = 0x0A in our normalized effect numbering. Param 0x37
    // means x=3, y=7.
    builder.add_pattern_row(0, 0, 49, 1, 64, 0x0A, 0x37);
    let mut tester = builder.get_tester();

    tester.tick(); // tick 0
    assert_eq!(tester.get_channel_effective_period(0), 1712, "tick0: base period");
    tester.tick(); // tick 1
    assert_eq!(tester.get_channel_effective_period(0), 1440, "tick1: +3 semitones");
    tester.tick(); // tick 2
    assert_eq!(tester.get_channel_effective_period(0), 1140, "tick2: +7 semitones");
}

#[test]
fn test_porta_to_note_does_not_underflow_on_large_speed() {
    // Regression: PortaToNoteState::next_tick used u16 wrapping arithmetic
    // and a post-subtract `< target` check. When the slide speed exceeded
    // the current period, the subtraction wrapped to ~65000 and the check
    // misread that as "still above target" — leaving the period stuck at
    // a huge value (a sub-Hz drone, i.e. channel ON but inaudible). The
    // fix widens to i32 so .min/.max clamp correctly. Reproduces against
    // 2ND_PM.S3M order 0x14 row 0x3C, where porta-to-A-5 from period 551
    // with speed-from-memory overshot to period 65511.
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.instruments[1].samples[0].data = vec![0.0; 100000];
    builder.instruments[1].samples[0].length = 100000;
    builder.add_empty_pattern(4);
    // Row 0: trigger a high note so the period is small.
    builder.add_pattern_row(0, 0, 73, 1, 64, 0, 0);
    // Row 1: porta-to-note even higher (smaller period) with huge speed
    // (FF * 4 stored), forcing speed > current_period and an underflow
    // in the (buggy) u16 subtraction.
    builder.add_pattern_row(0, 1, 85, 1, 255, 7, 0xFF);
    builder.add_pattern_row(0, 2, 0, 0, 255, 7, 0);
    builder.add_pattern_row(0, 3, 0, 0, 255, 0, 0);
    let mut tester = builder.get_tester();

    tester.run_row();
    let trigger_period = tester.song.channels[0].note.period;
    assert!(trigger_period > 0);

    tester.run_row();
    tester.run_row();

    // After overshoot ticks, the period must have clamped to the target
    // (a small but non-zero value), not wrapped to a huge u16.
    let target = tester.song.channels[0].porta_to_note.target_note.period;
    let current = tester.song.channels[0].note.period;
    assert!(target > 0, "target_period should be set");
    assert_eq!(current, target,
               "porta should clamp to target; got period={} target={}",
               current, target);
}

#[test]
fn test_step_forward_row_helper() {
    // Unit test for the silent step helper (still public, used elsewhere).
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.add_empty_pattern(8);
    let mut tester = builder.get_tester();

    let row_before = tester.song.row;
    tester.song.step_forward_row();
    assert_eq!(tester.song.row, row_before + 1);
    assert_eq!(tester.song.tick, 0);
}

#[test]
fn test_play_one_row_when_paused_auto_pauses() {
    // PlaybackCmd::Next while paused triggers "play one row of audio then
    // re-pause" by setting play_rows_remaining=1 and clearing pause. We
    // simulate that flag state directly and drive ticks; auto-pause must
    // trip on the row boundary.
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.add_empty_pattern(8);
    let mut tester = builder.get_tester();
    let speed = tester.song.speed;

    tester.song.pause = false;
    tester.song.play_rows_remaining = 1;

    // `speed` ticks worth of process_tick + next_tick should land us on
    // row 1 tick 0, with the row-advance hook having tripped the pause.
    for _ in 0..speed {
        tester.tick();
    }

    assert!(tester.song.pause, "auto-pause should trip on row boundary");
    assert_eq!(tester.song.play_rows_remaining, 0);
    assert_eq!(tester.song.row, 1, "should land on the next row");
    assert_eq!(tester.song.tick, 0);
}

#[test]
fn test_step_backward_row_when_paused() {
    // Paused-mode UX: PlaybackCmd::Prev rewinds by exactly one row.
    // Implemented via reset + walk forward to (target_pos, target_row),
    // so we verify it lands precisely — not buffer-granular over-run.
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.add_empty_pattern(8);
    let mut tester = builder.get_tester();

    // Walk forward to row 3 first.
    tester.run_row();
    tester.run_row();
    tester.run_row();
    assert_eq!(tester.song.row, 3);

    tester.song.pause = true;
    tester.song.step_backward_row();

    assert_eq!(tester.song.row, 2,
               "step_backward_row should land on row - 1");
    assert_eq!(tester.song.song_position, 0);
    assert!(tester.song.pause, "pause flag must persist after row-step");
}

#[test]
fn test_step_forward_row_terminates_at_song_end() {
    // The internal walk has a sanity bound to prevent UI hangs from
    // pathological row-progression. More importantly, hitting end-of-
    // song must just stop, not loop.
    let mut builder = MockSongBuilder::new(SongType::S3M, 1);
    builder.add_empty_pattern(2);
    let mut tester = builder.get_tester();
    tester.song.pause = true;

    // Step through all rows + past end.
    for _ in 0..10 {
        tester.song.step_forward_row();
    }
    // No assertion on final position — just that we returned cleanly.
}
