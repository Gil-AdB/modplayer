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
use crate::leak::{leak, leak_mut};

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

        leak(&app)
        // let on_the_heap = Box::new(app);
        // let leaked_pointer = Box::into_raw(on_the_heap);
        // let untyped_pointer = leaked_pointer as *mut c_void;
        //
        // untyped_pointer
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

        // let on_the_heap = Box::new(audio_output);
        // let leaked_pointer = Box::into_raw(on_the_heap);
        // let untyped_pointer = leaked_pointer as *mut c_void;
        // self.audio_output = untyped_pointer;

        let fps = 10; // call the function as fast as the browser wants to render (typically 60fps)
        let simulate_infinite_loop = 1; // call the function repeatedly

        let untyped_pointer = leak(self);
        // let on_the_heap = Box::new(self);
        // let leaked_pointer = Box::into_raw(on_the_heap);
        // let untyped_pointer = leaked_pointer as *mut c_void;

        let mut audio_output: *mut c_void = 0 as *mut c_void;
        let mut started = false;

        setup_mainloop(fps, simulate_infinite_loop, untyped_pointer,move |self_| unsafe {
            let leaked_pointer = untyped_pointer as *mut Self;
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

                        // audio_output = Box::into_raw(Box::new(audio.open_playback(None, &desired_spec, |spec| {
                        //     song.get().song.lock().unwrap().set_sample_rate(spec.freq as f32);
                        //     let (t, r) = mpsc::channel();
                        //     let audio_cb = AudioCB { q: song.clone(), rx: r };
                        //     audio_cb
                        // }).unwrap())) as *mut c_void;
                        audio_output = leak_mut(&mut audio.open_playback(None, &desired_spec, |spec| {
                            song.get().song.lock().unwrap().set_sample_rate(spec.freq as f32);
                            let (t, r) = mpsc::channel();
                            let audio_cb = AudioCB { q: song.clone(), rx: r };
                            audio_cb
                        }).unwrap());

                        started = true;

                        Self::resume(audio_output);
                    }
                }
            }
        });

    }
}

// #[no_mangle]
// extern fn Modplayer() -> *mut c_void {
//     // App::new()
// }

// #[no_mangle]
// extern fn Modplayer_Init(app_ptr: *mut c_void) {
//     let leaked_pointer = app_ptr as * mut App;
//     let self_ = unsafe { &mut *leaked_pointer };
//     self_.run()
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


