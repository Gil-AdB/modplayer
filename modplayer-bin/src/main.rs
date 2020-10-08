#![feature(generators, generator_trait)]
#![feature(async_closure)]
#![feature(box_syntax)]

use std::ops::{Generator, GeneratorState, Deref, DerefMut};
use std::pin::Pin;
use std::sync::{Arc, mpsc, Mutex};
use std::sync::atomic::{AtomicPtr, Ordering, AtomicBool};
use std::sync::mpsc::{Receiver, Sender};

use crossbeam::thread;
use getch::Getch;

use xmplayer::producer_consumer_queue::{AUDIO_BUF_SIZE, ProducerConsumerQueue, AUDIO_BUF_FRAMES};
// #[cfg(feature="sdl2-feature")]
use xmplayer::producer_consumer_queue::{PCQHolder};
#[cfg(feature="sdl2-feature")]
use sdl2::Error;

use xmplayer::song::{Song, PlaybackCmd, PlayData, CallbackState};
use xmplayer::module_reader::{SongData, read_module, print_module};
use std::env;
use std::time::{Duration, SystemTime};
use std::thread::{sleep, spawn, JoinHandle};
use std::io::{stdout, Write};
use xmplayer::triple_buffer::{TripleBuffer, TripleBufferReader};
#[cfg(feature="sdl2-feature")] use sdl2::audio::{AudioSpecDesired, AudioCallback};

use xmplayer::triple_buffer::State::StateNoChange;

#[cfg(feature="portaudio-feature")] extern crate portaudio;
#[cfg(feature="portaudio-feature")] use portaudio as pa;
use crossterm::terminal::ClearType::All;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use crate::display::Display;
use std::borrow::BorrowMut;
use std::marker::PhantomData;
use std::rc::Rc;

mod display;

fn unbox<T>(value: Box<T>) -> T {
    *value
}


#[derive(Clone)]
struct SongState {
    stopped:                            Arc<AtomicBool>,
    // triple_buffer:                      Arc<Box<TripleBuffer<PlayData>>>,
    triple_buffer_reader:               Arc<Mutex<TripleBufferReader<PlayData>>>,
    song_data:                          SongData,
    song:                               Arc<Mutex<Song>>,
    tx:                                 Sender<PlaybackCmd>,
    rx:                                 Arc<Mutex<Receiver<PlaybackCmd>>>,
    q:                                  PCQHolder,
    // display_cb:                         fn (&PlayData),
    // t1:                                 Option<JoinHandle<()>>,
    // t2:                                 Option<JoinHandle<()>>,
    // buf_ref:                            Arc<AtomicPtr<[f32; 2048]>>,
    // gen:                                Pin<Box<dyn Generator<Yield=(), Return=()>>>,

    self_ref:                           Option<StructHolder<SongState>>,

}

#[derive(Clone)]
pub struct StructHolder<T> {
    t: Arc<AtomicPtr<Box<T>>>,
}

impl <T> StructHolder<T> {
    pub fn new(arg: Box<T>) -> Self {
        Self { t: Arc::new(AtomicPtr::new(Box::into_raw(Box::new(arg)))) }
    }

    pub fn get(&mut self) -> &mut T {
        unsafe { &mut *self.t.load(Ordering::Acquire) }
    }
}

type SongHandle = StructHolder<SongState>;

impl SongState {

    fn new(path: String) -> StructHolder<Self> {
        let song_data = read_module(path.as_str()).unwrap();

        // let (mut triple_buffer_reader, triple_buffer_writer) = TripleBuffer::<PlayData>::new();
        let mut triple_buffer = TripleBuffer::<PlayData>::new();
        let (mut triple_buffer_reader, triple_buffer_writer) = triple_buffer.split();
        // //
        let song = Arc::new(Mutex::new(Song::new(&song_data, triple_buffer_writer, 48000.0)));
        let (tx, rx): (Sender<PlaybackCmd>, Receiver<PlaybackCmd>) = mpsc::channel();
        //
        // // let mut generator = song.get_next_tick_callback(buf_ref.clone(), rx);
        // let q = ProducerConsumerQueue::new();

        // Still waiting for generator resume args. Meanwhile, We'll use some ugly hacks...
        // let mut temp_buf = [0.0f32; AUDIO_BUF_SIZE];
        // let buf_ref: Arc<AtomicPtr<[f32; 2048]>> = Arc::new(AtomicPtr::new(&mut temp_buf as *mut [f32; AUDIO_BUF_SIZE]));
        // let mut gen = song.get_next_tick_callback(buf_ref.clone(), Arc::new(Mutex::new(rx)));
        // let mut gen = song_data.as_mut().get_next_tick_callback(buf_ref.clone(), rx);


        let stopped = Arc::new(AtomicBool::from(false));

        let mut sh = StructHolder::new( box Self {
            stopped,
            // triple_buffer: Arc::new(triple_buffer),
            triple_buffer_reader: Arc::new(Mutex::new(triple_buffer_reader)),
            song_data,
            song,
            tx,
            rx: Arc::new(Mutex::new(rx)),
            q: ProducerConsumerQueue::new(),
           // display_cb,
            // buf_ref,
            // gen: Arc::new(gen),
            // display_cb: (),
            // t1: None,
            // t2: None,
            self_ref: None
        });

        sh.get().self_ref = Option::from(sh.clone());
        sh
    }

    fn fill_buffer(&mut self, buffer : &mut [f32]) -> bool {
        // self.q.clone().get().consume(|buf: &[f32; AUDIO_BUF_SIZE]| { buffer.clone_from_slice(buf); })

        false
    }

    // fn produce_callback(song: &mut Song, mut q: PCQHolder) {
    //     let q = q.get();
    //     q.produce(|buf: &mut [f32; AUDIO_BUF_SIZE]| -> bool {
    //         // println!("produce {}", AUDIO_BUF_SIZE);
    //         //buf_ref.store(buf as *mut [f32; AUDIO_BUF_SIZE], Ordering::Release);
    //         let (tx, rx) = mpsc::channel();
    //         if let CallbackState::Complete = song.get_next_tick(buf, rx) { return false; }
    //         // if let GeneratorState::Complete(_) = gen.as_mut().resume(()) { return false; }
    //         // if let GeneratorState::Complete(_) = gen.resume(()) { return false; }
    //         true
    //     });
    // }

    fn callback(&mut self) {
        // let triple_buffer = TripleBuffer::<PlayData>::new();
        // let (mut triple_buffer_reader, triple_buffer_writer) = triple_buffer.split();
        //
        let mut song = self.song.lock().unwrap();
        // let mut qclone = self.q.clone();

        // Self::produce_callback(&m, qclone)
        let mut rx = self.rx.lock().unwrap();
        self.q.get().produce(|buf: &mut [f32; AUDIO_BUF_SIZE]| -> bool {
            // println!("produce {}", AUDIO_BUF_SIZE);
            //buf_ref.store(buf as *mut [f32; AUDIO_BUF_SIZE], Ordering::Release);
            if let CallbackState::Complete = song.get_next_tick(buf, rx.deref_mut()) { return false; }
            // if let GeneratorState::Complete(_) = gen.as_mut().resume(()) { return false; }
            // if let GeneratorState::Complete(_) = gen.resume(()) { return false; }
            true
        });
        // thread_stopped.store(true, Ordering::Release);
    }

    fn start(&mut self, sample_rate: f32, display_cb: fn (&PlayData)){

        let triple_buffer = TripleBuffer::<PlayData>::new();
        let (mut triple_buffer_reader, triple_buffer_writer) = triple_buffer.split();

        // let mut song = Song::new(&self.song_data, triple_buffer_writer, sample_rate);

        // let (tx, rx): (Sender<PlaybackCmd>, Receiver<PlaybackCmd>) = mpsc::channel();

        // let q = ProducerConsumerQueue::new();
        // let mut qclone = q.clone();

        // Still waiting for generator resume args. Meanwhile, We'll use some ugly hacks...
        // let mut temp_buf = [0.0f32; AUDIO_BUF_SIZE];
        // let buf_ref: Arc<AtomicPtr<[f32; 2048]>> = Arc::new(AtomicPtr::new(&mut temp_buf as *mut [f32; AUDIO_BUF_SIZE]));

        // let mut song = self.song;

        let thread_stopped = self.stopped.clone();
        let thread_stopped_reader = self.stopped.clone();


        let mut s = self.self_ref.as_mut().unwrap().clone();
           // .as_ref().clone();

        thread::scope(|scope|
            {
            // let mut q = self.q.clone();
            spawn(move || Self::callback(s.get()));
            //     Option::from(spawn(|| {
            //    // let mut gen =  Song::get_next_tick_callback(song,buf_ref.clone(), Arc::new(Mutex::new(rx)));
            //     let (tx, rx): (Sender<PlaybackCmd>, Receiver<PlaybackCmd>) = mpsc::channel();
            //     let q = self.q.clone().get();
            //     q.produce(|buf: &mut [f32; AUDIO_BUF_SIZE]| -> bool {
            //         Self::produce_callback(&mut song, buf)
            //     });
            //     // q.produce(|buf: &mut [f32; AUDIO_BUF_SIZE]| -> bool {
            //     //     // println!("produce {}", AUDIO_BUF_SIZE);
            //     //     //buf_ref.store(buf as *mut [f32; AUDIO_BUF_SIZE], Ordering::Release);
            //     //     if let CallbackState::Complete = song.get_next_tick(buf, rx) { return false; }
            //     //     // if let GeneratorState::Complete(_) = gen.as_mut().resume(()) { return false; }
            //     //     // if let GeneratorState::Complete(_) = gen.resume(()) { return false; }
            //     //     true
            //     // });
            //     thread_stopped.store(true, Ordering::Release);
            // }))
        });
                // self.t2 = Option::from(spawn(move |_| {
                //     let mut song_row = 0;
                //     let mut song_tick = 2000;
                //     crossterm::terminal::Clear(All);
                //     loop {
                //         if thread_stopped_reader.load(Ordering::Acquire) == true {
                //             break;
                //         }
                //         sleep(Duration::from_millis(30));
                //         let (play_data, state) = triple_buffer_reader.read();
                //         if StateNoChange == state { continue; }
                //         if play_data.tick != song_tick || play_data.row != song_row {
                //             (self.display_cb)(play_data);
                //             song_row = play_data.row;
                //             song_tick = play_data.tick;
                //         }
                //     }
                // }
                // ));
        // tx
    }
}


fn main() {
    if env::args().len() < 2 {return;}

	dbg!(env::args());

    let path = env::args().nth(1).unwrap();
    //let file = File::open(path).expect("failed to open the file");

   // let data = read_module(path.as_str()).unwrap();

    let mut song = SongState::new(path);
    if env::args().len() > 2 {
        // print_module(&song.song_data, env::args().skip(2));
    } else {
        run(&mut song).unwrap();
    }
}

#[cfg(feature="sdl2-feature")]
struct AudioCB {
   q: SongHandle
}

#[cfg(feature="sdl2-feature")]
impl AudioCallback for AudioCB {
   type Channel = f32;

   fn callback(&mut self, out: &mut [f32]) {
       if out.len() != AUDIO_BUF_SIZE {panic!("unexpected frame size: {}", out.len());}

       if !self.q.get().q.get().consume(|buf: &[f32; AUDIO_BUF_SIZE]| { out.clone_from_slice(buf); }) {
           // pa::Complete
       } else {
           // pa::Continue
       }
   }
}

#[cfg(feature="portaudio-feature")] type ErrorType = pa::Error;
#[cfg(feature="sdl2-feature")] type ErrorType = Error;

fn run(song_data: &mut StructHolder<SongState>) -> Result<(), ErrorType> {
    const CHANNELS: i32 = 2;
    // const NUM_SECONDS: i32 = 500;
    const SAMPLE_RATE: f32 = 48_000.0;

    // let mut triple_buffer = TripleBuffer::<PlayData>::new();
    // let (mut triple_buffer_reader, triple_buffer_writer) = triple_buffer.split();
    //
    // let mut song = Song::new(song_data, triple_buffer_writer, SAMPLE_RATE);
    // let (tx, rx): (Sender<PlaybackCmd>, Receiver<PlaybackCmd>) = mpsc::channel();
    // let mut temp_buf = [0.0f32; AUDIO_BUF_SIZE];
    // let buf_ref = Arc::new(AtomicPtr::new(&mut temp_buf as *mut [f32; AUDIO_BUF_SIZE]));
    // let mut generator = song.get_next_tick_callback(buf_ref.clone(), rx);
    //
    // let q = ProducerConsumerQueue::new();

    #[cfg(feature="portaudio-feature")]
    let pa_result: Result<pa::PortAudio, pa::Error> = pa::PortAudio::new();
    #[cfg(feature="portaudio-feature")]
    let _pa = match pa_result {
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

    // let mut qclone = q.clone();

    // This routine will be called by the PortAudio engine when audio is needed. It may called at
    // interrupt level on some machines so don't do anything that could mess up the system like
    // dynamic resource allocation or IO.
    #[cfg(feature="portaudio-feature")]
    let callback = move |pa::OutputStreamCallbackArgs { buffer, frames, .. }| {
        if frames != AUDIO_BUF_FRAMES {panic!("unexpected frame size: {}", frames);}
        if !song_data.fill_buffer(buffer) { //qclone.get().consume(|buf: &[f32; AUDIO_BUF_SIZE]| { buffer.clone_from_slice(buf); }) {
            pa::Complete
        } else {
            pa::Continue
        }
    };

    #[cfg(feature="sdl2-feature")]
    let audio_output = audio.open_playback(None, &desired_spec, |spec| {
       AudioCB{ q: song_data.clone() }
    }).unwrap();

    #[cfg(feature="portaudio-feature")]
    let mut stream = pa.open_non_blocking_stream(settings, callback)?;

    // let stopped = Arc::new(AtomicBool::from(false));
    // let thread_stopped = stopped.clone();
    // let thread_stopped_reader = stopped.clone();
    // thread::scope(|scope| {
    //     {
    //         let mut q = q.clone();
    //         scope.spawn(move |_| {
    //             let q = q.get();
    //
    //             q.produce(|buf: &mut [f32; AUDIO_BUF_SIZE]| -> bool {
    //                 // println!("produce {}", AUDIO_BUF_SIZE);
    //                 buf_ref.store(buf as *mut [f32; AUDIO_BUF_SIZE], Ordering::Release);
    //                 if let GeneratorState::Complete(_) = Pin::new(&mut generator).resume(()) { return false; }
    //                 true
    //             });
    //            thread_stopped.store(true, Ordering::Release);
    //         });
    //
    //         if let Err(_e) = crossterm::execute!(stdout(), EnterAlternateScreen) {}
    //
    //         scope.spawn( move |_| {
    //             let mut song_row = 0;
    //             let mut song_tick = 2000;
    //             crossterm::terminal::Clear(All);
    //             loop {
    //                 if thread_stopped_reader.load(Ordering::Acquire) == true {
    //                     break;
    //                 }
    //                 sleep(Duration::from_millis(30));
    //                 let (play_data, state) = triple_buffer_reader.read();
    //                 if StateNoChange == state { continue; }
    //                 if play_data.tick != song_tick || play_data.row != song_row {
    //                     Display::display(play_data, 0);
    //                     song_row = play_data.row;
    //                     song_tick = play_data.tick;
    //                 }
    //             }
    //         }
    //         );
    //
    //     }

    song_data.get().start(SAMPLE_RATE, |data| {});

    let mut triple_buffer_reader = song_data.get().triple_buffer_reader.clone();
    let thread_stopped_reader = song_data.get().stopped.clone();

        if let Err(_e) = crossterm::execute!(stdout(), EnterAlternateScreen) {}

        spawn( move || {
            let mut song_row = 0;
            let mut song_tick = 2000;
            crossterm::terminal::Clear(All);
            let mut triple_buffer_reader = triple_buffer_reader.lock().unwrap();
            // let thread
            loop {
                // if thread_stopped_reader.load(Ordering::Acquire) == true {
                //     break;
                // }
                sleep(Duration::from_millis(30));
                let (play_data, state) = triple_buffer_reader.deref_mut().read();
                if StateNoChange == state { continue; }
                if play_data.tick != song_tick || play_data.row != song_row {
                    Display::display(play_data, 0);
                    song_row = play_data.row;
                    song_tick = play_data.tick;
                }
            }
        }
        );


//    settings.flags = pa::stream_flags::CLIP_OFF;
//
        #[cfg(feature="portaudio-feature")]
         stream.start().unwrap();
//
        #[cfg(feature="sdl2-feature")]
        audio_output.resume();

        // println!("Play for {} seconds.", NUM_SECONDS);
        let stopped = Arc::new(AtomicBool::new(false));
        mainloop(&mut song_data.get().tx, stopped);
    // }).ok();

    #[cfg(feature="portaudio-feature")]
    stream.stop().unwrap();
    #[cfg(feature="portaudio-feature")]
    stream.close().unwrap();

    #[cfg(feature="sdl2-feature")]
    audio_output.close_and_get_callback();
    if let Err(_e) = crossterm::execute!(stdout(), LeaveAlternateScreen) {}
    println!("Test finished.");



//    println!("samples: {}", *count.lock().unwrap());
    Ok(())
}

fn is_num (ch: u8) -> bool {
    ch >= '0' as u8 && ch <= '9' as u8
}

fn mainloop(tx: &mut Sender<PlaybackCmd>, stopped: Arc<AtomicBool>) {

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
            if ch == ' ' as u8 {
                let _ = tx.send(PlaybackCmd::PauseToggle);
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
            if ch == 'a' as u8 {
                let _ = tx.send(PlaybackCmd::AmigaTable);
            }
            if ch == 'l' as u8 {
                let _ = tx.send(PlaybackCmd::LinearTable);
            }
            if ch == 'f' as u8 {
                let _ = tx.send(PlaybackCmd::FilterToggle);
            }
            if ch == 'd' as u8 {
                let _ = tx.send(PlaybackCmd::DisplayToggle);
            }
        }
        //pa.sleep(1_000);
        last_time = SystemTime::now();
    }
}
