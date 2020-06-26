#![feature(generators, generator_trait)]
#![feature(vec_drain_as_slice)]
#![feature(slice_fill)]


mod io_helpers;
mod xm_reader;
mod envelope;
mod instrument;
mod channel;
mod pattern;
mod producer_consumer_queue;
mod song;



extern crate portaudio;

use std::borrow::{Borrow, BorrowMut};
use std::cell::{RefCell, UnsafeCell};
use std::f32::consts::PI;
use std::fmt;
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom, Write, stdout};
use std::iter::FromIterator;
use std::num::Wrapping;
use std::ops::{Deref, DerefMut, Generator, GeneratorState};
use std::os::raw::*;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, mpsc};

use portaudio as pa;

// use crate::LoopType::{ForwardLoop, NoLoop, PingPongLoop};
use crossbeam::thread;
use portaudio::{Error, PortAudio};
use std::cmp::min;
use std::fmt::Debug;
use std::ptr::null;
use std::slice::SliceIndex;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::mpsc::{channel, Sender, Receiver};
use std::thread::sleep;
use std::time;
use crate::xm_reader::{read_xm, SongData};
use crate::song::Song;
// use term::stdout;

use getch::Getch;
use crate::producer_consumer_queue::{AUDIO_BUF_SIZE, ProducerConsumerQueue};

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

    run(read_xm(path));
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
    const FRAMES_PER_BUFFER: u32 = 4096;

    //crossterm::


    let mut song = Song::new(&song_data, SAMPLE_RATE);
    let (tx, rx): (Sender<i32>, Receiver<i32>) = mpsc::channel();


    let mut temp_buf = [0.0f32; AUDIO_BUF_SIZE];
    let mut buf_ref = Arc::new(AtomicPtr::new(&mut temp_buf as *mut [f32; AUDIO_BUF_SIZE]));
    let mut generator = song.get_next_tick_callback(buf_ref.clone(), rx);

    let mut q = ProducerConsumerQueue::new();

    let pa_result: Result<pa::PortAudio, pa::Error> = pa::PortAudio::new();
    let pa = match pa_result {
        Ok(p) => p,
        Err(e) => return Err(e),
    };

    let mut settings =
        pa.default_output_stream_settings(CHANNELS, SAMPLE_RATE as f64, (AUDIO_BUF_SIZE /2) as u32)?;

    let mut qclone = q.clone();

    // This routine will be called by the PortAudio engine when audio is needed. It may called at
    // interrupt level on some machines so don't do anything that could mess up the system like
    // dynamic resource allocation or IO.
    let callback = move |pa::OutputStreamCallbackArgs { buffer, frames, .. }| {
        unsafe { qclone.get().consume(|buf: &[f32; AUDIO_BUF_SIZE]| { buffer.clone_from_slice(buf); }) }
        pa::Continue
    };


    let mut stream = pa.open_non_blocking_stream(settings, callback)?;

    thread::scope(|scope| {
        {
            let mut q = q.clone();
            scope.spawn(move |_| unsafe {
                let mut idx = 0;
                let mut q = q.get();

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
        stream.start();
//

        let getter = Getch::new();
        println!("Play for {} seconds.", NUM_SECONDS);

        loop {
            if let Ok(ch) = getter.getch() {
                if ch == 'q' as u8 {
                    tx.send(-1);
                    break;
                };
                if ch == 'n' as u8 {
                    tx.send(0);
                };
                if ch == 'p' as u8 {
                    tx.send(1);
                };
                if ch == 'r' as u8 {
                    tx.send(2);
                };
            }


            //pa.sleep(1_000);
        }
    });

    stream.stop()?;
    stream.close()?;
    println!("Test finished.");



//    println!("samples: {}", *count.lock().unwrap());
    Ok(())
}
