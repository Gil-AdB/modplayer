#![feature(generators, generator_trait)]

extern crate portaudio;

use std::ops::{Generator, GeneratorState};
use std::pin::Pin;
use std::sync::{Arc, mpsc};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::mpsc::{Receiver, Sender};

use crossbeam::thread;
use getch::Getch;
use portaudio as pa;

use xmplayer::producer_consumer_queue::{AUDIO_BUF_SIZE, ProducerConsumerQueue, AUDIO_BUF_FRAMES};
use xmplayer::song::Song;
use xmplayer::xm_reader::{read_xm, SongData, print_xm};
use std::env;

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

    }).ok();

    stream.stop().unwrap();
    stream.close().unwrap();
    println!("Test finished.");



//    println!("samples: {}", *count.lock().unwrap());
    Ok(())
}
