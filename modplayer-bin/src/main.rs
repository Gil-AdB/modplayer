#![feature(generators, generator_trait)]
#![feature(async_closure)]

extern crate portaudio;

use std::ops::{Generator, GeneratorState};
use std::pin::Pin;
use std::sync::{Arc, mpsc};
use std::sync::atomic::{AtomicPtr, Ordering, AtomicBool};
use std::sync::mpsc::{Receiver, Sender};

use crossbeam::thread;
use getch::Getch;
use portaudio as pa;

use xmplayer::producer_consumer_queue::{AUDIO_BUF_SIZE, ProducerConsumerQueue, AUDIO_BUF_FRAMES};
use xmplayer::song::{Song, PlaybackCmd};
use xmplayer::xm_reader::{read_xm, SongData, print_xm};
use std::env;
use std::sync::atomic::Ordering::Release;
use std::time::{Duration, SystemTime};
use std::io::{Read, ErrorKind, Error};


fn main() {
    if env::args().len() < 2 {return;}
    let path = env::args().nth(1).unwrap();
    //let file = File::open(path).expect("failed to open the file");

    let data = read_xm(path.as_str());

    if env::args().len() > 2 {
        print_xm(&data);
    } else {
        run(data).unwrap();
    }
}

fn run(song_data : SongData) -> Result<(), pa::Error> {
    const CHANNELS: i32 = 2;
    const NUM_SECONDS: i32 = 500;
    const SAMPLE_RATE: f32 = 48_000.0;

    let mut song = Song::new(&song_data, SAMPLE_RATE);
    let (tx, rx): (Sender<PlaybackCmd>, Receiver<PlaybackCmd>) = mpsc::channel();


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
        if !qclone.get().consume(|buf: &[f32; AUDIO_BUF_SIZE]| { buffer.clone_from_slice(buf); }) {
            pa::Complete
        } else {
            pa::Continue
        }
    };


    let mut stream = pa.open_non_blocking_stream(settings, callback)?;
    let stopped = Arc::new(AtomicBool::from(false));
    let thread_stopped = stopped.clone();
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

        }



//    settings.flags = pa::stream_flags::CLIP_OFF;
//
        stream.start().unwrap();
//
        println!("Play for {} seconds.", NUM_SECONDS);
        mainloop(tx, stopped);
    }).ok();

    stream.stop().unwrap();
    stream.close().unwrap();

    println!("Test finished.");



//    println!("samples: {}", *count.lock().unwrap());
    Ok(())
}

fn is_num (ch: u8) -> bool {
    ch >= '0' as u8 && ch <= '9' as u8
}

fn mainloop(tx: Sender<PlaybackCmd>, stopped: Arc<AtomicBool>) {

    let getter = Getch::new();
    let mut last_time = SystemTime::now();
    let mut last_char = 0;

    loop {
        if stopped.load(Ordering::Acquire) {break;}

        // let input = tokio::time::timeout(Duration::from_secs(1), getter.getch()).await;
        let input = getter.getch();
        if SystemTime::now() > last_time + Duration::from_secs(1) {
            last_char = 0;
        }

        if let Ok(ch) = input {
            if ch == 'q' as u8 {
                let _ = tx.send(PlaybackCmd::Quit);
                break;
            }
            if is_num(ch) {
                if is_num(last_char) {
                    let channel_number = (last_char - '0' as u8)  * 10 + (ch - '0' as u8);
                    if channel_number > 0  && channel_number <= 32 {
                        let _ = tx.send(PlaybackCmd::ChannelToggle(channel_number - 1));
                    }
                    last_char = 0;
                } else {
                    last_char = ch;
                }
            }
            if ch == '+' as u8 {
                let _ = tx.send(PlaybackCmd::IncSpeed);
            }
            if ch == '-' as u8 {
                let _ = tx.send(PlaybackCmd::DecSpeed);
            }
            if ch == '.' as u8 {
                let _ = tx.send(PlaybackCmd::IncBPM);
            }
            if ch == ',' as u8 {
                let _ = tx.send(PlaybackCmd::DecBPM);
            }

            if ch == 'n' as u8 {
                let _ = tx.send(PlaybackCmd::Next);
            }
            if ch == '/' as u8 {
                let _ = tx.send(PlaybackCmd::LoopPattern);
            }
            if ch == 'p' as u8 {
                let _ = tx.send(PlaybackCmd::Prev);
            }
            if ch == 'r' as u8 {
                let _ = tx.send(PlaybackCmd::Restart);
            }
        }
        //pa.sleep(1_000);
        last_time = SystemTime::now();
    }
}