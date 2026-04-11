use xmplayer::song_state::SongState;
use xmplayer::song::InterleavedBufferAdaptar;
use xmplayer::song::CallbackState;

fn render_test_file(path: &str, num_frames: usize) -> f64 {
    let (song_handle, _consumer) = match SongState::new(path) {
        Ok(s) => s,
        Err(e) => panic!("Failed to load test file: {}", e),
    };

    let mut audio_buffer = vec![0.0f32; num_frames * 2];
    
    // Process frames in chunks
    let chunk_size = 512;
    let mut current_frame = 0;
    while current_frame < num_frames {
        let frames_to_generate = std::cmp::min(chunk_size, num_frames - current_frame);
        
        let mut adapter = InterleavedBufferAdaptar {
            buf: &mut audio_buffer[(current_frame * 2)..((current_frame + frames_to_generate) * 2)],
        };
        
        let mut song = song_handle.get_song().lock().unwrap();
        let (_tx, mut rx): (std::sync::mpsc::Sender<xmplayer::song::PlaybackCmd>, std::sync::mpsc::Receiver<xmplayer::song::PlaybackCmd>) = std::sync::mpsc::channel();
        if let CallbackState::Complete = song.get_next_tick(&mut adapter, &mut rx) {
            break;
        }
        
        let local_max = audio_buffer[(current_frame * 2)..((current_frame + frames_to_generate) * 2)]
            .iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        if local_max > 0.0 {
            println!("Got audio at frame {}: max sample val {}", current_frame, local_max);
        }

        current_frame += frames_to_generate;
    }

    // Compute RMSE locally (against zero for now just to generate a fingerprint sum)
    // A true golden test would check against a pre-computed array, but for now we
    // just return the sum or RMS to easily verify stability across refactors.
    let sum_sq: f64 = audio_buffer.iter().map(|&x| (x as f64) * (x as f64)).sum();
    
    (sum_sq / audio_buffer.len() as f64).sqrt()
}

#[test]
fn test_milky() {
    let rms = render_test_file("test_data/milky.xm", 44100 * 2); // 2 seconds
    println!("milky.xm RMS = {}", rms);
    assert!(rms > 0.0);
}

#[test]
fn test_openmpt_spacedeb() {
    let rms = render_test_file("test_data/spacedeb.mod", 44100 * 2);
    println!("spacedeb.mod RMS = {}", rms);
    assert!(rms > 0.0);
}

#[test]
fn test_openmpt_amiga_limits() {
    let rms = render_test_file("test_data/AmigaLimitsFinetune.mod", 44100 * 2);
    println!("AmigaLimitsFinetune.mod RMS = {}", rms);
    assert!(rms > 0.0);
}
