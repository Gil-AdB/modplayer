use xmplayer::module_reader::{SongData, SongType, Patterns, FrequencyType};
use xmplayer::instrument::Instrument;
use xmplayer::envelope::EnvelopePoint;
use xmplayer::song_state::SongState;
use xmplayer::song::{InterleavedBufferAdaptar, Song, PlaybackCmd};
use xmplayer::song::test_dump::{dump_tick};
use std::sync::{Arc, Mutex, mpsc};

fn create_test_song_handle(song_data: SongData) -> (Arc<Mutex<Song>>, mpsc::Receiver<PlaybackCmd>) {
    let (sh, _consumer) = SongState::new_from_data(song_data);
    let song = sh.get_song().clone();
    let (_tx, rx) = mpsc::channel();
    (song, rx)
}

fn minimal_song_data(song_type: SongType) -> SongData {
    let mut song_data = SongData::default();
    song_data.song_type = song_type;
    song_data.channel_count = 1;
    song_data.tempo = 6;
    song_data.bpm = 125;
    song_data.frequency_type = if song_type == SongType::XM { FrequencyType::LINEAR } else { FrequencyType::AMIGA };
    
    // Instrument 0 is dummy/empty
    song_data.instruments.push(Instrument::new());
    
    // Instrument 1 is our test instrument
    let mut instrument = Instrument::new();
    let sample = &mut instrument.samples[0];
    sample.length = 100000;
    sample.data = vec![0.0f32; 100000];
    sample.volume = 64;
    sample.panning = 128; // Center
    sample.setup_loops_and_padding();
    
    // Add dummy envelope points so it doesn't crash/bail
    instrument.volume_envelope.points[0] = EnvelopePoint { frame: 0, value: 64 };
    instrument.volume_envelope.points[1] = EnvelopePoint { frame: 100, value: 64 };
    instrument.volume_envelope.size = 2;
    
    song_data.instruments.push(instrument);
    song_data.instrument_count = 2;
    
    song_data
}

#[test]
fn test_arpeggio_xm() {
    let mut song_data = minimal_song_data(SongType::XM);
    song_data.tempo = 3;

    let mut patterns = Patterns::new(1, 1);
    patterns.rows[0].channels[0].note = 49; // C-4
    patterns.rows[0].channels[0].instrument = 1;
    patterns.rows[0].channels[0].effect = 0; // Arpeggio
    patterns.rows[0].channels[0].effect_param = 0x37; // +3, +7
    
    song_data.patterns.push(patterns);
    song_data.pattern_order = vec![0];
    song_data.song_length = 1;

    let (song_arc, mut rx) = create_test_song_handle(song_data);
    
    let mut dummy_buffer = vec![0.0f32; 1920]; // exactly one tick (960 frames)

    let base_du;

    // Tick 0: C-4
    {
        let mut adapter = InterleavedBufferAdaptar { buf: &mut dummy_buffer };
        let mut song = song_arc.lock().unwrap();
        song.get_next_tick(&mut adapter, &mut rx); // Process Tick 0
        let dump = dump_tick(&song);
        let v = &dump.voices[0];
        base_du = v.du;
    }
    
    // Tick 1: C-4 + 3 = D#4
    {
        let mut adapter = InterleavedBufferAdaptar { buf: &mut dummy_buffer };
        let mut song = song_arc.lock().unwrap();
        song.get_next_tick(&mut adapter, &mut rx); // Process Tick 1
        let dump = dump_tick(&song);
        let v = &dump.voices[0];
        // Ratio should be 2^(3/12) ~= 1.189
        assert!((v.du / base_du - 1.1892).abs() < 0.001);
    }

    // Tick 2: C-4 + 7 = G-4
    {
        let mut adapter = InterleavedBufferAdaptar { buf: &mut dummy_buffer };
        let mut song = song_arc.lock().unwrap();
        song.get_next_tick(&mut adapter, &mut rx); // Process Tick 2
        let dump = dump_tick(&song);
        let v = &dump.voices[0];
        // Ratio should be 2^(7/12) ~= 1.498
        assert!((v.du / base_du - 1.4983).abs() < 0.001);
    }
}

#[test]
fn test_set_volume_mod() {
    let mut song_data = minimal_song_data(SongType::MOD);

    let mut patterns = Patterns::new(1, 1);
    patterns.rows[0].channels[0].note = 49;
    patterns.rows[0].channels[0].instrument = 1;
    patterns.rows[0].channels[0].effect = 0xC; // Set Volume
    patterns.rows[0].channels[0].effect_param = 0x20; // 32
    
    song_data.patterns.push(patterns);
    song_data.pattern_order = vec![0];
    song_data.song_length = 1;

    let (song_arc, mut rx) = create_test_song_handle(song_data);
    
    let mut dummy_buffer = vec![0.0f32; 1920];

    let mut adapter = InterleavedBufferAdaptar { buf: &mut dummy_buffer };
    let mut song = song_arc.lock().unwrap();
    song.get_next_tick(&mut adapter, &mut rx);
    let dump = dump_tick(&song);
    let v = &dump.voices[0];
    // Volume 32/64 = 0.5
    assert!((v.output_volume - 0.5).abs() < 0.01);
}

#[test]
fn test_sample_offset() {
    let mut song_data = minimal_song_data(SongType::XM);

    let mut patterns = Patterns::new(1, 1);
    patterns.rows[0].channels[0].note = 49;
    patterns.rows[0].channels[0].instrument = 1;
    patterns.rows[0].channels[0].effect = 0x9; // Sample Offset
    patterns.rows[0].channels[0].effect_param = 0x10; // 16 * 256 = 4096
    
    song_data.patterns.push(patterns);
    song_data.pattern_order = vec![0];
    song_data.song_length = 1;

    let (song_arc, mut rx) = create_test_song_handle(song_data);
    
    let mut dummy_buffer = vec![0.0f32; 1920];

    let mut adapter = InterleavedBufferAdaptar { buf: &mut dummy_buffer };
    let mut song = song_arc.lock().unwrap();
    song.get_next_tick(&mut adapter, &mut rx);
    let dump = dump_tick(&song);
    let v = &dump.voices[0];
    // Offset 4096 + 4.0 sinc prefix + 960 * du
    let expected = 4096.0 + 4.0 + 960.0 * v.du;
    assert!((v.sample_pos - expected).abs() < 1.0);
}

#[test]
fn test_retrig_xm() {
    let mut song_data = minimal_song_data(SongType::XM);
    song_data.tempo = 6;

    let mut patterns = Patterns::new(1, 1);
    patterns.rows[0].channels[0].note = 49;
    patterns.rows[0].channels[0].instrument = 1;
    patterns.rows[0].channels[0].effect = 0xE; 
    patterns.rows[0].channels[0].effect_param = 0x92; // Retrig every 2 ticks
    
    song_data.patterns.push(patterns);
    song_data.pattern_order = vec![0];
    song_data.song_length = 1;

    let (song_arc, mut rx) = create_test_song_handle(song_data);
    
    let mut dummy_buffer = vec![0.0f32; 1920];

    // Tick 0: Trigger
    {
        let mut adapter = InterleavedBufferAdaptar { buf: &mut dummy_buffer };
        let mut song = song_arc.lock().unwrap();
        song.get_next_tick(&mut adapter, &mut rx);
        let dump = dump_tick(&song);
        assert!(dump.voices[0].sample_pos > 100.0);
    }
    
    // Tick 1: Playing
    {
        let mut adapter = InterleavedBufferAdaptar { buf: &mut dummy_buffer };
        let mut song = song_arc.lock().unwrap();
        song.get_next_tick(&mut adapter, &mut rx);
        let dump = dump_tick(&song);
        assert!(dump.voices[0].sample_pos > 300.0);
    }

    // Tick 2: Retrig!
    {
        let mut adapter = InterleavedBufferAdaptar { buf: &mut dummy_buffer };
        let mut song = song_arc.lock().unwrap();
        song.get_next_tick(&mut adapter, &mut rx);
        let dump = dump_tick(&song);
        // Should be reset to prefix + generated audio
        let expected = 4.0 + 960.0 * dump.voices[0].du;
        assert!((dump.voices[0].sample_pos - expected).abs() < 1.0);
    }
}

#[test]
fn test_key_off() {
    let mut song_data = minimal_song_data(SongType::XM);
    song_data.tempo = 6;

    let mut patterns = Patterns::new(1, 1);
    patterns.rows[0].channels[0].note = 49;
    patterns.rows[0].channels[0].instrument = 1;
    patterns.rows[0].channels[0].effect = 0x14; // Key Off
    patterns.rows[0].channels[0].effect_param = 0x03; // at tick 3
    
    song_data.patterns.push(patterns);
    song_data.pattern_order = vec![0];
    song_data.song_length = 1;

    let (song_arc, mut rx) = create_test_song_handle(song_data);
    let mut dummy_buffer = vec![0.0f32; 1920];

    // Tick 0, 1, 2: Sustain
    for _ in 0..3 {
        let mut adapter = InterleavedBufferAdaptar { buf: &mut dummy_buffer };
        let mut song = song_arc.lock().unwrap();
        song.get_next_tick(&mut adapter, &mut rx);
        assert!(dump_tick(&song).voices[0].sustained == true); 
    }

    // Tick 3: Key Off
    {
        let mut adapter = InterleavedBufferAdaptar { buf: &mut dummy_buffer };
        let mut song = song_arc.lock().unwrap();
        song.get_next_tick(&mut adapter, &mut rx);
        let dump = dump_tick(&song);
    assert!(dump.voices[0].sustained == false);
    }
}

#[test]
fn test_panning_slide() {
    let mut song_data = minimal_song_data(SongType::XM);
    song_data.tempo = 6;

    let mut patterns = Patterns::new(1, 1);
    patterns.rows[0].channels[0].note = 49;
    patterns.rows[0].channels[0].instrument = 1;
    patterns.rows[0].channels[0].effect = 0x19; // Panning Slide
    patterns.rows[0].channels[0].effect_param = 0x10; // Slide Right by 1 (* 4 = 4)
    
    song_data.patterns.push(patterns);
    song_data.pattern_order = vec![0];
    song_data.song_length = 1;
    // initial_channel_panning is multiplied by 4 in Song::new
    song_data.initial_channel_panning[0] = 32; // 32 * 4 = 128 (center)

    let (song_arc, mut rx) = create_test_song_handle(song_data);
    let mut dummy_buffer = vec![0.0f32; 1920];

    // Tick 0: Initial
    {
        let mut adapter = InterleavedBufferAdaptar { buf: &mut dummy_buffer };
        let mut song = song_arc.lock().unwrap();
        song.get_next_tick(&mut adapter, &mut rx);
        let dump = dump_tick(&song);
        assert_eq!(dump.voices[0].panning, 128);
    }

    // Tick 1: Slide Up
    {
        let mut adapter = InterleavedBufferAdaptar { buf: &mut dummy_buffer };
        let mut song = song_arc.lock().unwrap();
        song.get_next_tick(&mut adapter, &mut rx);
        let dump = dump_tick(&song);
        // 128 + 4 = 132
    assert_eq!(dump.voices[0].panning, 132);
    }
}

#[test]
fn test_envelope_position() {
    let mut song_data = minimal_song_data(SongType::XM);
    song_data.tempo = 6;
    song_data.instruments[1].volume_envelope.on = true;

    let mut patterns = Patterns::new(1, 1);
    patterns.rows[0].channels[0].note = 49;
    patterns.rows[0].channels[0].instrument = 1;
    patterns.rows[0].channels[0].effect = 0x15; // Set Envelope Position
    patterns.rows[0].channels[0].effect_param = 0x28; // 40
    
    song_data.patterns.push(patterns);
    song_data.pattern_order = vec![0];
    song_data.song_length = 1;

    let (song_arc, mut rx) = create_test_song_handle(song_data);
    let mut dummy_buffer = vec![0.0f32; 1920];

    let mut adapter = InterleavedBufferAdaptar { buf: &mut dummy_buffer };
    let mut song = song_arc.lock().unwrap();
    song.get_next_tick(&mut adapter, &mut rx);
    let dump = dump_tick(&song);
    assert_eq!(dump.voices[0].volume_envelope_pos, 41);
}
#[test]
fn test_s3m_note_off_cut() {
    let mut song_data = minimal_song_data(SongType::S3M);
    song_data.tempo = 3;

    let mut patterns = Patterns::new(3, 1);
    
    // Row 0: Trigger C-4
    patterns.rows[0].channels[0].note = 49;
    patterns.rows[0].channels[0].instrument = 1;
    
    // Row 1: Note Off (==) - 253
    patterns.rows[1].channels[0].note = 253;
    
    // Row 2: Note Cut (^^) - 254
    patterns.rows[2].channels[0].note = 254;
    
    song_data.patterns.push(patterns);
    song_data.pattern_order = vec![0];
    song_data.song_length = 1;
    song_data.song_type = SongType::S3M;

    let (song_arc, mut rx) = create_test_song_handle(song_data);
    let mut dummy_buffer = vec![0.0f32; 2000];

    // Row 0 Tick 0: Trigger
    {
        let mut adapter = InterleavedBufferAdaptar { buf: &mut dummy_buffer };
        let mut song = song_arc.lock().unwrap();
        song.get_next_tick(&mut adapter, &mut rx);
        let dump = dump_tick(&song);
        assert!(dump.voices.iter().any(|v| v.is_on && v.channel_idx == 0));
        
        // Finish Row 0 (Ticks 1-5)
        for _ in 1..6 {
            song.get_next_tick(&mut adapter, &mut rx);
        }
    }

    // Row 1 Tick 0: Note Off (==)
    {
        let mut adapter = InterleavedBufferAdaptar { buf: &mut dummy_buffer };
        let mut song = song_arc.lock().unwrap();
        song.get_next_tick(&mut adapter, &mut rx);
        let dump = dump_tick(&song);
        // S3M Note Off should kill the voice if no envelopes
        assert!(!dump.voices.iter().any(|v| v.is_on && v.channel_idx == 0));
    }
}
