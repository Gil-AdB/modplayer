// Render a module file to a 48kHz stereo float32 WAV through our
// engine, so the output can be compared against OpenMPT or ft2-clone
// renders of the same file. No interactive playback / display.
//
// Usage:
//   render_wav <module> <output.wav> [--start-time SEC] [--end-time SEC]
//                                    [--mute-channels a,b,c]
//
//   --start-time SEC      Skip first SEC seconds (forward via fast-forward).
//   --end-time SEC        Stop at SEC seconds (default: full song).
//   --mute-channels list  Comma-separated channel indices to silence by setting
//                         channel.force_off = true. Useful for A/B testing
//                         which channel produces a divergence vs OpenMPT.
//   --amiga-filter        MOD only: apply Paula's analog LP (~4.4 kHz) + HP
//                         (~5 Hz) chain post-mix. Off by default. Use when
//                         comparing against pt2-clone, which always filters.
//
// WAV format: PCM IEEE float32, 2 channels, 48000 Hz.

use shared_sync_primitives::TripleBuffer;
use std::env;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::mpsc;
use xmplayer::module_reader::read_module;
use xmplayer::song::{CallbackState, InterleavedBufferAdaptar, PlaybackCmd, Song};

const RATE: f32 = 48000.0;
const CHANNELS: u16 = 2;
const BITS_PER_SAMPLE: u16 = 32; // float32

fn write_wav_header(w: &mut impl Write, total_frames: u32) -> std::io::Result<()> {
    // PCM IEEE float (format code 0x0003) with WAVEFORMATEXTENSIBLE-equivalent
    // simple form. Many tools accept the simple fmt-3 layout.
    let byte_rate = RATE as u32 * CHANNELS as u32 * (BITS_PER_SAMPLE as u32 / 8);
    let block_align = CHANNELS * (BITS_PER_SAMPLE / 8);
    let data_bytes = total_frames * CHANNELS as u32 * (BITS_PER_SAMPLE as u32 / 8);

    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data_bytes).to_le_bytes())?;
    w.write_all(b"WAVE")?;

    w.write_all(b"fmt ")?;
    w.write_all(&16u32.to_le_bytes())?;          // fmt chunk size
    w.write_all(&3u16.to_le_bytes())?;           // format = IEEE float
    w.write_all(&CHANNELS.to_le_bytes())?;
    w.write_all(&(RATE as u32).to_le_bytes())?;
    w.write_all(&byte_rate.to_le_bytes())?;
    w.write_all(&block_align.to_le_bytes())?;
    w.write_all(&BITS_PER_SAMPLE.to_le_bytes())?;

    w.write_all(b"data")?;
    w.write_all(&data_bytes.to_le_bytes())?;
    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: render_wav <module> <output.wav> [--start-time SEC] [--end-time SEC]");
        std::process::exit(2);
    }
    let path = &args[1];
    let out_path = &args[2];

    let mut start_time = 0.0f32;
    let mut end_time = f32::INFINITY;
    let mut muted_channels: Vec<usize> = Vec::new();
    let mut amiga_filter = false;
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--start-time" => {
                i += 1;
                start_time = args.get(i).and_then(|s| s.parse().ok()).expect("bad --start-time");
            }
            "--end-time" => {
                i += 1;
                end_time = args.get(i).and_then(|s| s.parse().ok()).expect("bad --end-time");
            }
            "--mute-channels" => {
                i += 1;
                let v = args.get(i).expect("--mute-channels needs a list");
                muted_channels = v.split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
            }
            "--amiga-filter" => {
                amiga_filter = true;
            }
            other => {
                eprintln!("unknown flag: {}", other);
                std::process::exit(2);
            }
        }
        i += 1;
    }

    let song_data = match read_module(path) {
        Ok(d) => d,
        Err(e) => { eprintln!("read_module {}: {:?}", path, e); std::process::exit(1); }
    };
    let (_reader, writer) = TripleBuffer::new().split();
    let mut song = Song::new(&song_data, writer, RATE);
    if amiga_filter {
        song.set_amiga_filter(true);
        eprintln!("amiga filter: enabled (Paula LP@4.42kHz + HP@5Hz)");
    }

    // Apply channel mutes via force_off. Each backend's per-voice loop
    // observes `channel.force_off || channel.tremor_silenced` and zeros
    // the voice output for that channel. The flag stays set for the
    // whole render — re-asserted every tick is unnecessary.
    for &ch in &muted_channels {
        if ch < song.channels.len() {
            song.channels[ch].force_off = true;
            eprintln!("mute: channel {} force_off=true", ch);
        }
    }

    // Optional seek
    if start_time > 0.0 {
        song.seek_forward_seconds(start_time);
    }

    let f = File::create(out_path).expect("create wav");
    let mut w = BufWriter::new(f);
    // Reserve header space; we patch the size at the end.
    write_wav_header(&mut w, 0).expect("header");

    // Mix into a small buffer; loop until end_time reached or song completes.
    let buf_frames = 4096usize;
    let mut buf = vec![0.0f32; buf_frames * CHANNELS as usize];
    let (_tx, mut rx) = mpsc::channel::<PlaybackCmd>();

    let max_samples = if end_time.is_finite() {
        ((end_time - start_time).max(0.0) * RATE) as u64
    } else {
        u64::MAX
    };
    let mut frames_written = 0u64;

    'outer: loop {
        for v in buf.iter_mut() { *v = 0.0; }
        let mut adapter = InterleavedBufferAdaptar { buf: &mut buf };
        let cb = song.get_next_tick(&mut adapter, &mut rx);

        let frames_in_buf = buf_frames as u64;
        let take = frames_in_buf.min(max_samples - frames_written);
        let bytes = (take as usize) * CHANNELS as usize * 4;
        w.write_all(&bytemuck_to_bytes(&buf[..(take as usize) * CHANNELS as usize])).expect("write");
        frames_written += take;
        let _ = bytes;

        if frames_written >= max_samples { break; }
        if let CallbackState::Complete = cb { break 'outer; }
    }

    // Patch the WAV size fields. Reopen the file for in-place patching.
    drop(w);
    use std::io::{Seek, SeekFrom};
    let mut f = std::fs::OpenOptions::new().write(true).open(out_path).expect("reopen");
    let data_bytes = (frames_written as u32) * CHANNELS as u32 * (BITS_PER_SAMPLE as u32 / 8);
    f.seek(SeekFrom::Start(4)).unwrap();
    f.write_all(&(36 + data_bytes).to_le_bytes()).unwrap();
    f.seek(SeekFrom::Start(40)).unwrap();
    f.write_all(&data_bytes.to_le_bytes()).unwrap();

    eprintln!("wrote {:.2}s ({} frames) to {}", frames_written as f32 / RATE, frames_written, out_path);
}

/// Manual f32 slice -> &[u8] for WAV writing (avoids pulling in bytemuck).
fn bytemuck_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for &s in v {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}
