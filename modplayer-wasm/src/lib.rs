#![feature(generators, generator_trait)]
#![feature(async_closure)]

use std::ops::{Generator, GeneratorState};
use std::pin::Pin;
use std::sync::{Arc, mpsc};
use std::sync::atomic::{AtomicPtr, Ordering, AtomicBool};
use std::sync::mpsc::{Receiver, Sender};

use crossbeam::thread;
// use getch::Getch;

use xmplayer::producer_consumer_queue::{AUDIO_BUF_SIZE, ProducerConsumerQueue};
use xmplayer::song::{Song, PlaybackCmd, PlayData};
use xmplayer::module_reader::{read_module, SongData};
use std::env;
use std::time::{Duration};
use std::io::{Error};
use std::thread::sleep;
use xmplayer::triple_buffer::{TripleBuffer};
// use js_sys;

#[cfg(feature="sdl2-feature")] use sdl2::audio::{AudioSpecDesired, AudioCallback};

use xmplayer::triple_buffer::State::StateNoChange;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn main() {
    if env::args().len() < 2 {return;}
    let path = env::args().nth(1).unwrap();
    //let file = File::open(path).expect("failed to open the file");

    let data = read_module(path.as_str()).unwrap();

    if env::args().len() > 2 {
        // print_xm(&data);
    } else {
        run(data).unwrap();
    }
}


// #[wasm_bindgen]
// pub fn play(path:String) -> fn(&mut js_sys::Float32Array) -> i32 {
//     const SAMPLE_RATE: f32 = 48_000.0;
//     let song_data = read_module(path.as_str()).unwrap();
//
//     let (mut triple_buffer_reader, mut triple_buffer_writer) = triple_buffer::<PlayData>::new();
//
//     let mut song = Song::new(&song_data, triple_buffer_writer, SAMPLE_RATE);
//     let (tx, rx): (Sender<PlaybackCmd>, Receiver<PlaybackCmd>) = mpsc::channel();
//
//     let mut temp_buf = [0.0f32; AUDIO_BUF_SIZE];
//     let buf_ref = Arc::new(AtomicPtr::new(&mut temp_buf as *mut [f32; AUDIO_BUF_SIZE]));
//     let mut generator = song.get_next_tick_callback(buf_ref.clone(), rx);
//
//     let cb = |buf: &mut js_sys::Float32Array| -> i32 {
//
//         return 0i32;
//     };
//
//     cb
// }
//
// pub struct JSCB {
//     song_data: SongData,
//     song: Song,
// }
//
// impl JSCB {
//     fn fill_buffer(buf: &mut [f32]) {
//
//     }
// }
//
// #[wasm_bindgen]
// pub fn load_song(rate: f32) -> JSCB {
//     let path = "mods/introx.xm";
//     //let file = File::open(path).expect("failed to open the file");
//
//     let data = read_xm(path.as_str());
//
//     // if env::args().len() > 2 {
//     //     print_xm(&data);
//     // } else {
//     //     run(data).unwrap();
//     // }
//
//     JSCB{song_data: data}
//
// }


#[cfg(feature="sdl2-feature")]
struct AudioCB {
    q: PCQHolder
}

#[cfg(feature="sdl2-feature")]
impl AudioCallback for AudioCB {
    type Channel = f32;

    fn callback(&mut self, out: &mut [f32]) {
        if out.len() != AUDIO_BUF_SIZE {panic!("unexpected frame size: {}", out.len());}
        if !self.q.get().consume(|buf: &[f32; AUDIO_BUF_SIZE]| { out.clone_from_slice(buf); }) {
            // pa::Complete
        } else {
            // pa::Continue
        }
    }
}

// #[cfg(feature="portaudio-feature")] type ErrorType = pa::Error;
// #[cfg(feature="sdl2-feature")] type ErrorType = Error;

fn run(song_data : SongData) -> Result<(), Error> {
    // const CHANNELS: i32 = 2;
    const NUM_SECONDS: i32 = 500;
    const SAMPLE_RATE: f32 = 48_000.0;

    let (mut triple_buffer_reader, triple_buffer_writer) = TripleBuffer::<PlayData>::new();

    let mut song = Song::new(&song_data, triple_buffer_writer, SAMPLE_RATE);
    let (tx, rx): (Sender<PlaybackCmd>, Receiver<PlaybackCmd>) = mpsc::channel();


    let mut temp_buf = [0.0f32; AUDIO_BUF_SIZE];
    let buf_ref = Arc::new(AtomicPtr::new(&mut temp_buf as *mut [f32; AUDIO_BUF_SIZE]));
    let mut generator = song.get_next_tick_callback(buf_ref.clone(), rx);

    let q = ProducerConsumerQueue::new();

    #[cfg(feature="portaudio-feature")]
        let pa_result: Result<pa::PortAudio, pa::Error> = pa::PortAudio::new();
    #[cfg(feature="portaudio-feature")]
        let pa = match pa_result {
        Ok(p) => p,
        Err(e) => return Err(e),
    };

    #[cfg(feature="sdl2-feature")]
        let desired_spec = AudioSpecDesired {
        freq: Some(SAMPLE_RATE as i32),
        channels: Some(2),
        samples: Some((AUDIO_BUF_SIZE / 2) as u16)
    };

    #[cfg(feature="sdl2-feature")] let sdl_context = sdl2::init().unwrap();
    #[cfg(feature="sdl2-feature")] let audio = sdl_context.audio().unwrap();
    #[cfg(feature="portaudio-feature")]
        let pa = pa::PortAudio::new()?;
    #[cfg(feature="portaudio-feature")]
        let settings =
        pa.default_output_stream_settings(CHANNELS, SAMPLE_RATE as f64, (AUDIO_BUF_SIZE / 2) as u32)?;


    // This routine will be called by the PortAudio engine when audio is needed. It may called at
    // interrupt level on some machines so don't do anything that could mess up the system like
    // dynamic resource allocation or IO.
    #[cfg(feature="portaudio-feature")]
        let mut qclone = q.clone();
    #[cfg(feature="portaudio-feature")]
        let callback = move |pa::OutputStreamCallbackArgs { buffer, frames, .. }| {
        if frames != AUDIO_BUF_FRAMES {panic!("unexpected frame size: {}", frames);}
        if !qclone.get().consume(|buf: &[f32; AUDIO_BUF_SIZE]| { buffer.clone_from_slice(buf); }) {
            pa::Complete
        } else {
            pa::Continue
        }
    };

    #[cfg(feature="sdl2-feature")]
        let qclone = q.clone();
    #[cfg(feature="sdl2-feature")]
        let audio_output = audio.open_playback(None, &desired_spec, |spec| {
        AudioCB{ q: qclone }
    }).unwrap();

    #[cfg(feature="portaudio-feature")]
        let mut stream = pa.open_non_blocking_stream(settings, callback)?;

    let stopped = Arc::new(AtomicBool::from(false));
    let thread_stopped = stopped.clone();
    let thread_stopped_reader = stopped.clone();
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
                thread_stopped.store(true, Ordering::Release);
            });


            scope.spawn( move |_| {
                let mut song_row = 0;
                let mut song_tick = 2000;
                loop {
                    if thread_stopped_reader.load(Ordering::Acquire) == true {
                        break;
                    }
                    sleep(Duration::from_millis(30));
                    let (play_data, state) = triple_buffer_reader.read();
                    if StateNoChange == state { continue; }
                    if play_data.tick != song_tick || play_data.row != song_row {
                        // Display::display(play_data, 0);
                        song_row = play_data.row;
                        song_tick = play_data.tick;
                    }
                }
            }
            );

        }



//    settings.flags = pa::stream_flags::CLIP_OFF;
//
        #[cfg(feature="portaudio-feature")]
            stream.start().unwrap();
//
        #[cfg(feature="sdl2-feature")]
            audio_output.resume();

        println!("Play for {} seconds.", NUM_SECONDS);
        mainloop(tx, stopped);
    }).ok();

    #[cfg(feature="portaudio-feature")]
        stream.stop().unwrap();
    #[cfg(feature="portaudio-feature")]
        stream.close().unwrap();

    #[cfg(feature="sdl2-feature")]
        audio_output.close_and_get_callback();
    println!("Test finished.");



//    println!("samples: {}", *count.lock().unwrap());
    Ok(())
}

// fn is_num (ch: u8) -> bool {
//     ch >= '0' as u8 && ch <= '9' as u8
// }

fn mainloop(_tx: Sender<PlaybackCmd>, stopped: Arc<AtomicBool>) {

    // let getter = Getch::new();
    // let mut last_time = SystemTime::now();
    // let mut last_char = 0;

    loop {
        if stopped.load(Ordering::Acquire) {break;}

        // let input = tokio::time::timeout(Duration::from_secs(1), getter.getch()).await;
        // let input = getter.getch();
        std::thread::sleep(std::time::Duration::from_secs(1));
        // last_time = SystemTime::now();
    }
}
