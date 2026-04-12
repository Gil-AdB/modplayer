// #[macro_use]

// extern crate lazy_static;

extern crate sdl2;
extern crate xmplayer;

mod leak;

#[cfg(feature="sdl2-feature")] mod sdl2_audio;
#[cfg(feature="sdl2-feature")] use sdl2_audio::AudioOutput;
#[cfg(feature="portaudio-feature")] mod portaudio_audio;
#[cfg(feature="portaudio-feature")] use portaudio_audio::AudioOutput;

use sdl2::audio::{AudioCallback};
use xmplayer::song::{PlaybackCmd, CallbackState, InterleavedBufferAdaptar};
use xmplayer::song_state::{SongState, SongHandle};
use std::sync::{mpsc};
use std::sync::mpsc::{Receiver, Sender};
use std::ffi::{c_void, CStr};
use std::os::raw::c_char;
use xmplayer::SimpleResult;


pub enum PlayerCmd {
    Stop,
    NewSong(String)
}

#[allow(dead_code)]
struct AudioCB {
    q: SongHandle,
}

impl AudioCallback for AudioCB {
    type Channel = f32;

    fn callback(&mut self, out: &mut [f32]) {
        let mut song = self.q.get_song().lock().unwrap();
        let (_tx, mut rx): (Sender<PlaybackCmd>, Receiver<PlaybackCmd>) = mpsc::channel();
        let mut adaptar = InterleavedBufferAdaptar{buf: out};

        if let CallbackState::Complete = song.get_next_tick(&mut adaptar, &mut rx) {
            self.q.stop();
            // App::stop();
        }
    }
}

#[allow(dead_code)]
struct App {
    song_row:       usize,
    song_tick:      u32,
    audio_output:   AudioOutput,
    song_handle:    SongHandle,
    play_thread: Option<std::thread::JoinHandle<()>>,
    display_thread: Option<std::thread::JoinHandle<()>>,
}

impl App {
    fn new(path: String) -> SimpleResult<*mut c_void> {

        dbg!("start");
        let (song, consumer) = SongState::new(&path)?;
        Ok(leak!(Self {
            song_row: 0,
            song_tick: 2000,
            audio_output: AudioOutput::new(consumer, 48000.0),
            song_handle: song,
            play_thread: None,
            display_thread: None,
        }))
    }

    pub(crate) fn start(&mut self) {
        let h = self.song_handle.start(|_data, _instruments, _patterns, _order| {});
        self.play_thread = h.0;
        self.display_thread = h.1;
        self.audio_output.start_audio_output();
    }

    pub(crate) fn set_order(&mut self, order: u32) {
        self.song_handle.set_order(order);
    }

    fn close_audio(&mut self) {
        self.audio_output.close();
        self.song_handle.close();
        if self.play_thread.is_some() {
            self.play_thread.take().map(std::thread::JoinHandle::join);
        }
        if self.display_thread.is_some() {
            self.display_thread.take().map(std::thread::JoinHandle::join);
        }
    }
}


#[unsafe(no_mangle)]
extern "C" fn Modplayer_Stop(app_ptr: *mut c_void) {
    if app_ptr == 0 as *mut c_void {return;}
    let leaked_pointer = app_ptr as *mut App;
    let self_ = unsafe { &mut *leaked_pointer };
    self_.close_audio();
    let _ = unsafe { Box::from_raw(self_) };
}

#[unsafe(no_mangle)]
extern "C" fn Modplayer_Start(app_ptr: *mut c_void) {
    dbg!("Modplayer_Start");
    if app_ptr == 0 as *mut c_void {return;}
    let leaked_pointer = app_ptr as *mut App;
    let self_ = unsafe { &mut *leaked_pointer };
    self_.start();
}

#[unsafe(no_mangle)]
extern "C" fn Modplayer_SetOrder(app_ptr: *mut c_void, order: u32) {
    dbg!("Modplayer_SetOrder");
    if app_ptr == 0 as *mut c_void {return;}
    let leaked_pointer = app_ptr as *mut App;
    let self_ = unsafe { &mut *leaked_pointer };
    self_.set_order(order);
}

#[unsafe(no_mangle)]
extern "C" fn Modplayer_Create(path: *const c_char) -> *mut c_void {
         match App::new(unsafe { CStr::from_ptr(path) }.to_str().unwrap().to_string()) {
         Ok(app) => {app}
         Err(_) => {0 as * mut c_void}
     }
}


