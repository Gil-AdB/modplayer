#[macro_use]
extern crate lazy_static;

extern crate wasm_bindgen;
// extern crate sdl2;
extern crate xmplayer;

mod leak;
mod display;

use wasm_bindgen::prelude::*;
use xmplayer::song::{PlaybackCmd, PlayData, CallbackState, Song, BufferAdapter};
use xmplayer::song_state::{SongState, SongHandle, StructHolder};
use std::sync::{mpsc, Arc};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Mutex;
// use sdl2::{EventPump};
use std::ffi::c_void;
use std::ffi::CString;
use std::collections::VecDeque;
use std::time::{SystemTime, Duration};
use xmplayer::triple_buffer::State::StateNoChange;
use xmplayer::instrument::Instrument;
use display::Display;
use display::ViewPort;
use std::ops::DerefMut;
use xmplayer::producer_consumer_queue::{AUDIO_BUF_FRAMES};
use std::sync::atomic::Ordering;
use xmplayer::module_reader::{SongData, read_module, open_module};
use xmplayer::simple_error::{SimpleResult};
use xmplayer::triple_buffer::{TripleBufferReader, TripleBuffer};
use xmplayer::song::PlanarBufferAdaptar;
use wasm_bindgen::__rt::std::os::raw::c_char;
use crate::wasm_bindgen::JsCast;


use std::convert::{TryInto};
use std::ops::{Add, Sub, AddAssign, SubAssign};

pub use std::time::*;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant(std::time::Instant);

#[cfg(not(target_arch = "wasm32"))]
impl Instant {
    pub fn now() -> Self { Self(std::time::Instant::now()) }
    pub fn duration_since(&self, earlier: Instant) -> Duration { self.0.duration_since(earlier.0) }
    pub fn elapsed(&self) -> Duration { self.0.elapsed() }
    pub fn checked_add(&self, duration: Duration) -> Option<Self> { self.0.checked_add(duration).map(|i| Self(i)) }
    pub fn checked_sub(&self, duration: Duration) -> Option<Self> { self.0.checked_sub(duration).map(|i| Self(i)) }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(inline_js = r#"
export function performance_now() {
  return performance.now();
}"#)]
extern "C" {
    fn performance_now() -> f64;
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant(u64);

#[cfg(target_arch = "wasm32")]
impl Instant {
    pub fn now() -> Self { Self((performance_now() * 1000.0) as u64) }
    pub fn duration_since(&self, earlier: Instant) -> Duration { Duration::from_micros(self.0 - earlier.0) }
    pub fn elapsed(&self) -> Duration { Self::now().duration_since(*self) }
    pub fn checked_add(&self, duration: Duration) -> Option<Self> {
        match duration.as_micros().try_into() {
            Ok(duration) => self.0.checked_add(duration).map(|i| Self(i)),
            Err(_) => None,
        }
    }
    pub fn checked_sub(&self, duration: Duration) -> Option<Self> {
        match duration.as_micros().try_into() {
            Ok(duration) => self.0.checked_sub(duration).map(|i| Self(i)),
            Err(_) => None,
        }
    }
}

impl Add<Duration> for Instant { type Output = Instant; fn add(self, other: Duration) -> Instant { self.checked_add(other).unwrap() } }
impl Sub<Duration> for Instant { type Output = Instant; fn sub(self, other: Duration) -> Instant { self.checked_sub(other).unwrap() } }
impl Sub<Instant>  for Instant { type Output = Duration; fn sub(self, other: Instant) -> Duration { self.duration_since(other) } }
impl AddAssign<Duration> for Instant { fn add_assign(&mut self, other: Duration) { *self = *self + other; } }
impl SubAssign<Duration> for Instant { fn sub_assign(&mut self, other: Duration) { *self = *self - other; } }




// First up let's take a look of binding `console.log` manually, without the
// help of `web_sys`. Here we're writing the `#[wasm_bindgen]` annotations
// manually ourselves, and the correctness of our program relies on the
// correctness of these annotations!

#[wasm_bindgen]
extern "C" {
    // Use `js_namespace` here to bind `console.log(..)` instead of just
    // `log(..)`
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);

    // The `console.log` is quite polymorphic, so we can bind it with multiple
    // signatures. Note that we need to use `js_name` to ensure we always call
    // `log` in JS.
    #[wasm_bindgen(js_namespace = console, js_name = log)]
    fn log_u32(a: u32);

    // Multiple arguments too!
    #[wasm_bindgen(js_namespace = console, js_name = log)]
    fn log_many(a: &str, b: &str);
}

// Next let's define a macro that's like `println!`, only it works for
// `console.log`. Note that `println!` doesn't actually work on the wasm target
// because the standard library currently just eats all output. To get
// `println!`-like behavior in your app you'll likely want a macro like this.

macro_rules! console_log {
    // Note that this is using the `log` function imported above during
    // `bare_bones`
    ($($t:tt)*) => (log(&format_args!($($t)*).to_string()))
}

pub enum PlayerCmd {
    Stop,
    NewSong(String)
}

// mpsc is just too much hassle. I tried.
lazy_static!(
    static ref CMDS: Mutex<VecDeque<PlayerCmd>> = Mutex::new(VecDeque::new());
    static ref PLAYBACK_CMDS: Mutex<VecDeque<PlaybackCmd>> = Mutex::new(VecDeque::new());
);

struct AudioCB {
    q: SongHandle,
}

#[wasm_bindgen]
pub struct SongJs {
    song:                               Song,
    triple_buffer_reader:               Arc<Mutex<TripleBufferReader<PlayData>>>,
    song_row:                           usize,
    song_tick:                          u32,
    tx:                                 Sender<PlaybackCmd>,
    rx:                                 Receiver<PlaybackCmd>,
    last_time:                          Instant,
    last_char:                          char,
}

use js_sys::{Array, JsString};

#[wasm_bindgen(module = "/export.js")]
extern "C" {
    pub fn term_writeln(str: String);
}

#[wasm_bindgen]
impl SongJs {
    pub fn new(sample_rate:f32, data: &[u8]) -> Self {
        let data = open_module(data).unwrap();
        let triple_buffer = TripleBuffer::<PlayData>::new();
        let (triple_buffer_reader, triple_buffer_writer) = triple_buffer.split();
        let song = Song::new(&data, triple_buffer_writer, sample_rate);
        let (tx, mut rx): (Sender<PlaybackCmd>, Receiver<PlaybackCmd>) = mpsc::channel();
        Self {
            song,
            triple_buffer_reader: Arc::new(Mutex::new(triple_buffer_reader)),
            song_row: 0,
            song_tick: 2000,
            tx,
            rx,
            last_time: Instant::now(),
            last_char: '\0'
        }
    }

    pub fn display(&mut self) /*-> Array*/ {
//        let mut result: Vec<String> = vec!();
        let mut tbr = self.triple_buffer_reader.lock().unwrap();
        let (play_data, state) = tbr.read();
        if StateNoChange == state {
            return;// result.into_iter().map(JsValue::from).collect();
        }

        if play_data.tick != self.song_tick || play_data.row != self.song_row {

            let view_port = ViewPort {
                x1: 0,
                y1: 0,
                width: 200,
                height: 35
            };

            let instruments = self.song.get_instruments();

            unsafe {
                let s = Display::move_to(1, 1);
                term_writeln(s);
            }

            Display::display(play_data, &instruments, view_port, &mut|str| {
                //    result.push(str);
                unsafe { term_writeln(str); }
            });
            self.song_row = play_data.row;
            self.song_tick = play_data.tick;
        }
    }

    // true  - continue playing
    // false - song finished
    pub fn get_next_tick(&mut self, left: &mut [f32], right: &mut [f32], sample_rate: f32) -> bool {

        self.song.set_sample_rate(sample_rate);
        let mut adaptar = PlanarBufferAdaptar{buf:[left, right]};
        match self.song.get_next_tick(&mut adaptar, &mut self.rx) {

            CallbackState::Ok => {true}
            CallbackState::Complete => {false}
        }
    }

    pub fn handle_input(&mut self, events: &Array) -> bool {

        let now = Instant::now();

        let tx = &self.tx;

        for event in events.iter() {

            if now > self.last_time + Duration::from_secs(1) {
                self.last_char = '\0';
            }

            let key = String::from(event.dyn_ref::<JsString>().unwrap());

            match key.as_ref() {
                "Escape" | "q" => {
                    let _ = tx.send(PlaybackCmd::Quit);
                    return true;
                }
                "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" => {
                    let ch = ((key.as_bytes()[0] as i32 - '0' as i32) as u8 + '0' as u8) as char; // FIXME: Blachhh
                    if self.last_char != '\0' {
                        let channel_number = (self.last_char as u8 - '0' as u8) * 10 + (ch as u8 - '0' as u8);
                        if channel_number > 0 && channel_number <= 32 {
                            let _ = tx.send(PlaybackCmd::ChannelToggle(channel_number - 1));
                        }
                        self.last_char = '\0';
                    } else {
                        self.last_char = ch;
                    }
                }
                "+" => {
                    let _ = tx.send(PlaybackCmd::IncSpeed);
                }

                "-" => {
                    let _ = tx.send(PlaybackCmd::DecSpeed);
                }
                "." => {
                    let _ = tx.send(PlaybackCmd::IncBPM);
                }
                "," => {
                    let _ = tx.send(PlaybackCmd::DecBPM);
                }
                " " => {
                    let _ = tx.send(PlaybackCmd::PauseToggle);
                }
                "n" => {
                    let _ = tx.send(PlaybackCmd::Next);
                }
                "/" => {
                    let _ = tx.send(PlaybackCmd::LoopPattern);
                }
                "p" => {
                    let _ = tx.send(PlaybackCmd::Prev);
                }
                "r" => {
                    let _ = tx.send(PlaybackCmd::Restart);
                }
                "a" => {
                    let _ = tx.send(PlaybackCmd::AmigaTable);
                }
                "l" => {
                    let _ = tx.send(PlaybackCmd::LinearTable);
                }
                "f" => {
                    let _ = tx.send(PlaybackCmd::FilterToggle);
                }
                "d" => {
                    let _ = tx.send(PlaybackCmd::DisplayToggle);
                }
                _ => {}
            }
        }
        self.last_time = now;
        return false;
    }
}

struct App {
    song_row:       usize,
    song_tick:      u32,
}

impl App {
    fn new() -> *mut c_void {

        // let (tx, mut rx): (Sender<PlayerCmd>, Receiver<PlayerCmd>) = mpsc::channel();

        let app = Self{
            // tx: Box::new(tx),
            // rx: Box::new(rx)
            song_row: 0,
            song_tick: 2000
        };

        leak!(app)
    }
    
    // fn resume(audio: *mut c_void) {
    //     let leaked_pointer = audio as *mut AudioDevice<AudioCB>;
    //     let audio = unsafe { &mut *leaked_pointer };
    //     audio.resume();
    // }
    //
    // fn close_audio(audio: *mut c_void) {
    //     let leaked_pointer = audio as *mut AudioDevice<AudioCB>;
    //     let audio = unsafe { &mut *leaked_pointer };
    //     let audio_boxed = unsafe { Box::from_raw(audio)};
    //     audio_boxed.close_and_get_callback();
    // }

    // fn handle_display(&mut self, triple_buffer_reader: &mut TripleBufferReader<PlayData>, instruments: &Vec<Instrument>) {
    //     let (play_data, state) = triple_buffer_reader.read();
    //     if StateNoChange == state { return; }
    //     if play_data.tick != self.song_tick || play_data.row != self.song_row {
    //
    //         let view_port = ViewPort {
    //             x1: 0,
    //             y1: 0,
    //             width: 200,
    //             height: 35
    //         };
    //
    //
    //         unsafe { term_writeln(CString::new(Display::move_to(1, 1)).unwrap().as_ptr()); }
    //
    //         Display::display(play_data, instruments, view_port, &mut|str| {
    //             unsafe { term_writeln(CString::new(str).unwrap().as_ptr()); }
    //         });
    //         self.song_row = play_data.row;
    //         self.song_tick = play_data.tick;
    //     }
    // }

    fn run(&self) {
        // let sdl_context = sdl2::init().unwrap();
        // let audio = sdl_context.audio().unwrap();

        // Must init video subsytem in order for keyboard input to work
        // let video_subsystem = sdl_context.video().unwrap();
        // let window = video_subsystem
        //     .window("Mod Player", 0, 0)
        //     .build()
        //     .unwrap();
        // let _canvas = window.into_canvas().build().unwrap();

        // let event_pump = leak!(sdl_context.event_pump().unwrap());

        // let fps = -1; // call the function as fast as the browser wants to render (typically 60fps)
        // let simulate_infinite_loop = 1; // call the function repeatedly
        //
        // let leaked_self = leak!(self);
        //
        // let mut audio_output: *mut c_void = 0 as *mut c_void;
        // let mut triple_buffer_reader: Option<Arc<Mutex<TripleBufferReader<PlayData>>>> = None;
        // let mut instruments: Vec<Instrument> = vec![];
        //
        // setup_mainloop(fps, simulate_infinite_loop, leaked_self, move |self_| unsafe {
        //     let leaked_pointer = leaked_self as *mut Self;
        //     let self_ = &mut *leaked_pointer;
        //
        //     if triple_buffer_reader.is_some() {
        //         self_.handle_display(&mut triple_buffer_reader.as_ref().unwrap().lock().unwrap().deref_mut(), &instruments);
        //     }
        //
        //     let mut cmds = CMDS.lock().unwrap();
        //     while cmds.len() > 0 {
        //         let cmd = cmds.pop_front();
        //         match cmd.unwrap() {
        //             PlayerCmd::Stop => {
        //                 dbg!("Stop");
        //                 App::stop_audio(&mut audio_output, &mut triple_buffer_reader);
        //             }
        //             PlayerCmd::NewSong(cb) => {
        //                 dbg!("Start");
        //
        //                 App::stop_audio(&mut audio_output, &mut triple_buffer_reader);
        //
        //                 term_writeln(CString::new(Display::clear()).unwrap().as_ptr());
        //
        //                 let desired_spec = AudioSpecDesired {
        //                     freq: Some(48000 as i32),
        //                     channels: Some(2),
        //                     samples: Some(AUDIO_BUF_FRAMES as u16)
        //                 };
        //
        //                 let mut song = match SongState::new("/file".to_string()) {
        //                     Ok(s) => {s}
        //                     Err(_) => {return;}
        //                 };
        //                 instruments = song.get_mut().song.lock().unwrap().get_instruments();
        //
        //                 triple_buffer_reader =  Option::from(song.get_mut().get_triple_buffer_reader());
        //                 audio_output = leak!(audio.open_playback(None, &desired_spec, |spec| {
        //                     song.get_mut().song.lock().unwrap().set_sample_rate(spec.freq as f32);
        //                     let audio_cb = AudioCB { q: song.clone()};
        //                     audio_cb
        //                 }).unwrap());
        //
        //                 Self::resume(audio_output);
        //             }
        //         }
        //     }
        //     // let leaked_event_pump = event_pump as *mut EventPump;
        //     // let event_pump = &mut *leaked_event_pump;
        //     // if handle_input(event_pump) {Self::stop();}
        // });

    }

    // unsafe fn stop_audio(audio_output: &mut *mut c_void, triple_buffer_reader: &mut Option<Arc<Mutex<TripleBufferReader<PlayData>>>>) {
    //     if *audio_output != 0 as *mut c_void {
    //         *triple_buffer_reader = None;
    //         // Self::close_audio(*audio_output);
    //         // *audio_output = 0 as *mut c_void;
    //         on_module_stop();
    //     }
    // }
}
