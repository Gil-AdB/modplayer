// Based on sample source from https://gitlab.com/ThibaultLemaire/rust-sdl-canvas-wasm provided by Thibault Lemaire

#[macro_use]
extern crate lazy_static;

extern crate sdl2;
extern crate xmplayer;

mod emscripten_boilerplate;
mod leak;

use emscripten_boilerplate::{setup_mainloop, emscripten_cancel_main_loop};
use sdl2::pixels::Color;
use sdl2::rect::Rect;
use sdl2::audio::{AudioCallback, AudioSpecDesired, AudioDevice};
use xmplayer::producer_consumer_queue::{AUDIO_BUF_SIZE, ProducerConsumerQueue, AUDIO_BUF_FRAMES};
use xmplayer::producer_consumer_queue::{PCQHolder};
use xmplayer::song::{Song, PlaybackCmd, PlayData, CallbackState};
use xmplayer::module_reader::{SongData, read_module, print_module};
use xmplayer::song_state::{SongState, SongHandle};
use std::sync::{mpsc, Arc};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::ptr::{replace, null, null_mut};
use std::sync::Mutex;
use sdl2::{Sdl, AudioSubsystem};
use std::ffi::c_void;
use crate::emscripten_boilerplate::emscripten_run_script;
use std::ffi::CString;
use std::collections::VecDeque;

pub enum PlayerCmd {
    Stop,
    NewSong
}


// mpsc is just too much hassle. I tried.
lazy_static!(
    static ref CMDS: Mutex<VecDeque<PlayerCmd>> = Mutex::new(VecDeque::new());
);

struct AudioCB {
    q: SongHandle,
    rx: Receiver<PlayerCmd>
}

impl AudioCallback for AudioCB {
    type Channel = f32;

    fn callback(&mut self, out: &mut [f32]) {
        let mut song = self.q.get().song.lock().unwrap();

        let (tx, mut rx) = mpsc::channel();

        song.get_next_tick(out, &mut rx);
    }
}


fn unbox<T>(value: Box<T>) -> T {
    *value
}

struct App {
    tx:             Box<Sender<PlayerCmd>>,
    rx:             Box<Receiver<PlayerCmd>>
}

impl App {
    fn new() -> *mut c_void {

        let (tx, mut rx): (Sender<PlayerCmd>, Receiver<PlayerCmd>) = mpsc::channel();

        let app = Self{
            tx: Box::new(tx),
            rx: Box::new(rx)
        };

        leak!(app)
    }

    pub(crate) fn start(&self) {
        dbg!("Sending New");
        CMDS.lock().unwrap().push_back(PlayerCmd::NewSong);
    }

    pub(crate) fn stop(&self) {
        dbg!("Sending Stop");
        CMDS.lock().unwrap().push_back(PlayerCmd::Stop);
    }

    
    fn resume(audio: *mut c_void) {
        let leaked_pointer = audio as *mut AudioDevice<AudioCB>;
        let audio = unsafe { &mut *leaked_pointer };
        audio.resume();
    }

    fn close_audio(audio: *mut c_void) {
        let leaked_pointer = audio as *mut AudioDevice<AudioCB>;
        let audio = unsafe { &mut *leaked_pointer };
        let audio_unboxed = unsafe { Box::from_raw(audio)};
        audio_unboxed.close_and_get_callback();
    }

    fn run(&self) {

        let sdl_context = sdl2::init().unwrap();
        let audio = sdl_context.audio().unwrap();


        let fps = 10; // call the function as fast as the browser wants to render (typically 60fps)
        let simulate_infinite_loop = 1; // call the function repeatedly

        let leaked_self = leak!(self);

        let mut audio_output: *mut c_void = 0 as *mut c_void;
        let mut started = false;

        setup_mainloop(fps, simulate_infinite_loop, leaked_self, move |self_| unsafe {
            let leaked_pointer = leaked_self as *mut Self;
            let self_ = unsafe { &mut *leaked_pointer };

            let mut cmds = CMDS.lock().unwrap();
            while cmds.len() > 0 {
                let cmd = cmds.pop_front();
                match cmd.unwrap() {
                    PlayerCmd::Stop => {
                        dbg!("Stop");
                        if started {
                            Self::close_audio(audio_output);
                            started = false;
                        }
                    }
                    PlayerCmd::NewSong => {
                        dbg!("Start");

                        let desired_spec = AudioSpecDesired {
                            freq: Some(48000 as i32),
                            channels: Some(2),
                            samples: Some(1024 as u16)
                        };

                        let mut song = SongState::new("/file".to_string());
                        let (t, r) = mpsc::channel();

                        audio_output = leak!(audio.open_playback(None, &desired_spec, |spec| {
                            song.get().song.lock().unwrap().set_sample_rate(spec.freq as f32);
                            let audio_cb = AudioCB { q: song.clone(), rx: r };
                            audio_cb
                        }).unwrap());

                        started = true;

                        Self::resume(audio_output);
                    }
                }
            }
            // handle_input();
        });

    }
}

// fn handle_input() {
//     let mut last_time = SystemTime::now();
//     let mut last_char = '\0';
//
//     if song_data.is_stopped() { return Ok(true); }
//
//     // let input = tokio::time::timeout(Duration::from_secs(1), getter.getch()).await;
//     let input;
//     if crossterm::event::poll(Duration::from_millis(100))? {
//         // It's guaranteed that the `read()` won't block when the `poll()`
//         // function returns `true`
//         match crossterm::event::read()? {
//             crossterm::event::Event::Key(event) => input = event,
//             _ => { continue; }
//         }
//     } else {
//         continue;
//     }
//
//
//     if SystemTime::now() > last_time + Duration::from_secs(1) {
//         last_char = '\0';
//     }
//
//     if let KeyCode::Esc = input.code {
//         let tx = song_data.get_sender();
//         let _ = tx.send(PlaybackCmd::Quit);
//         break;
//     }
//
//     if let KeyCode::Char(ch) = input.code {
//         let tx = &mut song_data.get_sender();
//         if ch == 'q' {
//             let _ = tx.send(PlaybackCmd::Quit);
//             break;
//         }
//         if is_num(ch) {
//             if is_num(last_char) {
//                 let channel_number = (last_char as u8 - '0' as u8) * 10 + (ch as u8 - '0' as u8);
//                 if channel_number > 0 && channel_number <= 32 {
//                     let _ = tx.send(PlaybackCmd::ChannelToggle(channel_number - 1));
//                 }
//                 last_char = '\0';
//             } else {
//                 last_char = ch;
//             }
//         }
//         if ch == '+' {
//             let _ = tx.send(PlaybackCmd::IncSpeed);
//         }
//         if ch == '-' {
//             let _ = tx.send(PlaybackCmd::DecSpeed);
//         }
//         if ch == '.' {
//             let _ = tx.send(PlaybackCmd::IncBPM);
//         }
//         if ch == ',' {
//             let _ = tx.send(PlaybackCmd::DecBPM);
//         }
//         if ch == ' ' {
//             let _ = tx.send(PlaybackCmd::PauseToggle);
//         }
//         if ch == 'n' {
//             let _ = tx.send(PlaybackCmd::Next);
//         }
//         if ch == '/' {
//             let _ = tx.send(PlaybackCmd::LoopPattern);
//         }
//         if ch == 'p' {
//             let _ = tx.send(PlaybackCmd::Prev);
//         }
//         if ch == 'r' {
//             let _ = tx.send(PlaybackCmd::Restart);
//         }
//         if ch == 'a' {
//             let _ = tx.send(PlaybackCmd::AmigaTable);
//         }
//         if ch == 'l' {
//             let _ = tx.send(PlaybackCmd::LinearTable);
//         }
//         if ch == 'f' {
//             let _ = tx.send(PlaybackCmd::FilterToggle);
//         }
//         if ch == 'd' {
//             let _ = tx.send(PlaybackCmd::DisplayToggle);
//         }
//     }
//     last_time = SystemTime::now();
// }

#[no_mangle]
extern fn Modplayer_Stop(app_ptr: *mut c_void) {
    let leaked_pointer = app_ptr as *mut App;
    let self_ = unsafe { &mut *leaked_pointer };
    self_.stop();
}

#[no_mangle]
extern fn Modplayer_Start(app_ptr: *mut c_void) {
    dbg!("Modplayer_Start");
    let leaked_pointer = app_ptr as *mut App;
    let self_ = unsafe { &mut *leaked_pointer };
    self_.start();
}

pub fn main() {
    let untyped_pointer = App::new();
    let code = format!("player = {}", untyped_pointer as u64);
    unsafe { emscripten_run_script(CString::new(code).unwrap().as_ptr());};

    let typed_pointer = untyped_pointer as *mut App;
    let self_ = unsafe { &mut *typed_pointer as &mut App };
    self_.run()
}


