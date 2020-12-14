#[macro_use]

// extern crate lazy_static;

extern crate sdl2;
extern crate xmplayer;

mod leak;

#[cfg(feature="sdl2-feature")] mod sdl2_audio;
#[cfg(feature="sdl2-feature")] use sdl2_audio::AudioOutput;
#[cfg(feature="portaudio-feature")] mod portaudio_audio;
#[cfg(feature="portaudio-feature")] use portaudio_audio::AudioOutput;

use sdl2::audio::{AudioCallback, AudioSpecDesired, AudioDevice};
use xmplayer::song::{PlaybackCmd, PlayData, CallbackState};
use xmplayer::song_state::{SongState, SongHandle, StructHolder};
use std::sync::{mpsc, Arc};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Mutex;
use sdl2::{EventPump};
use std::ffi::{c_void, CStr};
use std::collections::VecDeque;
use std::time::{SystemTime, Duration};
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use xmplayer::triple_buffer::TripleBufferReader;
use xmplayer::triple_buffer::State::StateNoChange;
use xmplayer::instrument::Instrument;
use std::ops::DerefMut;
use xmplayer::producer_consumer_queue::{AUDIO_BUF_FRAMES};
use std::sync::atomic::Ordering;
use std::os::raw::c_char;
use simple_error::{SimpleResult, SimpleError};
use std::thread::JoinHandle;

pub enum PlayerCmd {
    Stop,
    NewSong(String)
}

struct AudioCB {
    q: SongHandle,
}

impl AudioCallback for AudioCB {
    type Channel = f32;

    fn callback(&mut self, out: &mut [f32]) {
        let song_state = self.q.get_mut();
        let mut song = song_state.song.lock().unwrap();
        let (tx, mut rx): (Sender<PlaybackCmd>, Receiver<PlaybackCmd>) = mpsc::channel();

        if let CallbackState::Complete = song.get_next_tick(out, &mut rx) {
            song_state.stopped.store(true, Ordering::Release);
            // App::stop();
        }
    }
}

struct App {
    song_row:       usize,
    song_tick:      u32,
    audio_output:   AudioOutput,
}

impl App {
    fn new(path: String) -> SimpleResult<*mut c_void> {

        // let (tx, mut rx): (Sender<PlayerCmd>, Receiver<PlayerCmd>) = mpsc::channel();
        dbg!("start");
        let mut song = SongState::new(path)?;
        Ok(leak!(Self {
            // tx: Box::new(tx),
            // rx: Box::new(rx)
            song_row: 0,
            song_tick: 2000,
            audio_output: AudioOutput::new(song, 48000.0),
        }))
    }

    pub(crate) fn start(&mut self) {
        self.audio_output.start_audio_output();
    }
    pub(crate) fn set_order(&mut self, order: u32) {
        self.audio_output.set_order(order);
    }

    fn close_audio(&mut self) {
        self.audio_output.close();
    }
}


#[no_mangle]
extern fn Modplayer_Stop(app_ptr: *mut c_void) {
    if app_ptr == 0 as *mut c_void {return;}
    let leaked_pointer = app_ptr as *mut App;
    let self_ = unsafe { &mut *leaked_pointer };
    self_.close_audio();
    unsafe { Box::from_raw(self_); }
}

#[no_mangle]
extern fn Modplayer_Start(app_ptr: *mut c_void) {
    dbg!("Modplayer_Start");
    if app_ptr == 0 as *mut c_void {return;}
    let leaked_pointer = app_ptr as *mut App;
    let self_ = unsafe { &mut *leaked_pointer };
    self_.start();
}

#[no_mangle]
extern fn Modplayer_SetOrder(app_ptr: *mut c_void, order: u32) {
    dbg!("Modplayer_SetOrder");
    if app_ptr == 0 as *mut c_void {return;}
    let leaked_pointer = app_ptr as *mut App;
    let self_ = unsafe { &mut *leaked_pointer };
    self_.set_order(order);
}

#[no_mangle]
extern fn Modplayer_Create(path: *const c_char) -> *mut c_void {
         match App::new(unsafe { CStr::from_ptr(path) }.to_str().unwrap().to_string()) {
         Ok(app) => {app}
         Err(_) => {0 as * mut c_void}
     }
}


