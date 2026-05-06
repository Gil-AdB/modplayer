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
