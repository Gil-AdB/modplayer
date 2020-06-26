#![feature(generators, generator_trait)]
#![feature(vec_drain_as_slice)]
#![feature(slice_fill)]
#![feature(in_band_lifetimes)]


extern crate portaudio;

use std::ops::{Generator, GeneratorState};
use std::pin::Pin;
use std::sync::{Arc, mpsc};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::mpsc::{Receiver, Sender};

use crossbeam::thread;
use getch::Getch;
use portaudio as pa;

use crate::producer_consumer_queue::{AUDIO_BUF_SIZE, ProducerConsumerQueue, AUDIO_BUF_FRAMES};
use crate::song::Song;
use crate::xm_reader::{read_xm, SongData};

mod io_helpers;
mod xm_reader;
mod envelope;
mod instrument;
mod channel_state;
mod pattern;
mod producer_consumer_queue;
mod song;


// use term::stdout;

// #[repr(C)]
// #[repr(packed)]
// struct NativeXmheader {
//     id:                 [c_char;17usize],
//     name:               [c_char;20usize],
//     sig:                c_char,
//     tracker_name:       [c_char;20usize],
//     ver:                c_ushort,
//     header_size:        c_uint,
//     song_length:        c_ushort,
//     restart_position:   c_ushort,
//     channel_count:      c_ushort,
//     pattern_count:      c_ushort,
//     instrument_count:   c_ushort,
//     flags:              c_ushort,
//     tempo:              c_ushort,
//     bpm:                c_ushort,
//     pattern_order:      [c_uchar;256usize],
// }
//
// struct XMPatternHeader {
//     header_length:      c_uint,
//     packing:            c_uchar,
//     row_count:          c_ushort,
//     packed_size:        c_ushort,
// }

fn main() {
    let path = "children.XM";
    //let file = File::open(path).expect("failed to open the file");

    run(read_xm(path)).unwrap();
//    let mmap = unsafe { Mmap::map(&file).expect("failed to map the file") };
//
//    println!("File Size: {}", mmap.len());
//
//    let _header =  unsafe {&*(mmap.as_ptr() as * const XMHeader)};
//
//    let mut _pattern_offset = mem::size_of::<XMHeader>() as isize;
//    for pattern_idx in 0.._header.pattern_count {
//        let _pattern = unsafe {{&*(mmap.as_ptr().offset(_pattern_offset) as * const XMPatternHeader)}};
//        _pattern_offset = _pattern_offset + _pattern.packed_size as isize;
//    }
//
//    let _banana = 1;

}

fn run(song_data : SongData) -> Result<(), pa::Error> {
    const CHANNELS: i32 = 2;
    const NUM_SECONDS: i32 = 500;
    const SAMPLE_RATE: f32 = 48_000.0;


    let mut song = Song::new(&song_data, SAMPLE_RATE);
    let (tx, rx): (Sender<i32>, Receiver<i32>) = mpsc::channel();


    let mut temp_buf = [0.0f32; AUDIO_BUF_SIZE];
    let buf_ref = Arc::new(AtomicPtr::new(&mut temp_buf as *mut [f32; AUDIO_BUF_SIZE]));
    let mut generator = song.get_next_tick_callback(buf_ref.clone(), rx);

    let q = ProducerConsumerQueue::new();

    // let pa_result: Result<pa::PortAudio, pa::Error> = pa::PortAudio::new();
    // let pa = match pa_result {
    //     Ok(p) => p,
    //     Err(e) => return Err(e),
    // };

    let pa = pa::PortAudio::new()?;

    let settings =
        pa.default_output_stream_settings(CHANNELS, SAMPLE_RATE as f64, (AUDIO_BUF_SIZE / 2) as u32)?;

    let mut qclone = q.clone();

    // This routine will be called by the PortAudio engine when audio is needed. It may called at
    // interrupt level on some machines so don't do anything that could mess up the system like
    // dynamic resource allocation or IO.
    let callback = move |pa::OutputStreamCallbackArgs { buffer, frames, .. }| {
        if frames != AUDIO_BUF_FRAMES {panic!("unexpected frame size: {}", frames);}
        qclone.get().consume(|buf: &[f32; AUDIO_BUF_SIZE]| { buffer.clone_from_slice(buf); });
        pa::Continue
    };


    let mut stream = pa.open_non_blocking_stream(settings, callback)?;

    thread::scope(|scope| {
        {
            let mut q = q.clone();
            scope.spawn(move |_| {
                let q = q.get();

                q.produce(|buf: &mut [f32; AUDIO_BUF_SIZE]| -> bool {
                    // println!("produce {}", AUDIO_BUF_SIZE);
                    buf_ref.store(buf as *mut [f32; AUDIO_BUF_SIZE], Ordering::Release);
                    if let GeneratorState::Complete(_) = Pin::new(&mut generator).resume(()) { return false; }
                    true
                });
            });
        }



//    settings.flags = pa::stream_flags::CLIP_OFF;
//
        stream.start().unwrap();
//

        let getter = Getch::new();
        println!("Play for {} seconds.", NUM_SECONDS);

        loop {
            if let Ok(ch) = getter.getch() {
                if ch == 'q' as u8 {
                    tx.send(-1).ok();
                    break;
                };
                if ch == 'n' as u8 {
                    tx.send(0).ok();
                };
                if ch == 'p' as u8 {
                    tx.send(1).ok();
                };
                if ch == 'r' as u8 {
                    tx.send(2).ok();
                };
            }


            //pa.sleep(1_000);
        }
        stream.stop().unwrap();
        stream.close().unwrap();
    }).ok();

    println!("Test finished.");



//    println!("samples: {}", *count.lock().unwrap());
    Ok(())
}
