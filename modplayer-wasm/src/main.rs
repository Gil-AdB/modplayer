// Based on sample source from https://gitlab.com/ThibaultLemaire/rust-sdl-canvas-wasm provided by Thibault Lemaire

#[macro_use]
extern crate lazy_static;

extern crate sdl2;
extern crate xmplayer;

mod emscripten_boilerplate;
mod leak;

use emscripten_boilerplate::{setup_mainloop};
use sdl2::audio::{AudioCallback, AudioSpecDesired, AudioDevice};
use xmplayer::song::{PlaybackCmd, PlayData};
use xmplayer::song_state::{SongState, SongHandle};
use std::sync::{mpsc, Arc};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Mutex;
use sdl2::{EventPump};
use std::ffi::c_void;
use crate::emscripten_boilerplate::{emscripten_run_script, term_writeln};
use std::ffi::CString;
use std::collections::VecDeque;
use std::time::{SystemTime, Duration};
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use xmplayer::triple_buffer::TripleBufferReader;
use xmplayer::triple_buffer::State::StateNoChange;
use xmplayer::instrument::Instrument;
use display::display::Display;
use display::ViewPort;
use std::ops::DerefMut;
use xmplayer::producer_consumer_queue::{AUDIO_BUF_FRAMES};

pub enum PlayerCmd {
    Stop,
    NewSong
}


// mpsc is just too much hassle. I tried.
lazy_static!(
    static ref CMDS: Mutex<VecDeque<PlayerCmd>> = Mutex::new(VecDeque::new());
    static ref PLAYBACK_CMDS: Mutex<VecDeque<PlaybackCmd>> = Mutex::new(VecDeque::new());
);

struct AudioCB {
    q: SongHandle,
}

impl AudioCallback for AudioCB {
    type Channel = f32;

    fn callback(&mut self, out: &mut [f32]) {
        let mut song = self.q.get_mut().song.lock().unwrap();
        let (tx, mut rx): (Sender<PlaybackCmd>, Receiver<PlaybackCmd>) = mpsc::channel();

        // Oh, Well...
        let mut cmds = PLAYBACK_CMDS.lock().unwrap();
        while cmds.len() > 0 {
            let cmd = cmds.pop_front().unwrap();
            let _ = tx.send(cmd);
        }


        song.get_next_tick(out, &mut rx);
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
        let audio_boxed = unsafe { Box::from_raw(audio)};
        audio_boxed.close_and_get_callback();
    }

    fn handle_display(&mut self, triple_buffer_reader: &mut TripleBufferReader<PlayData>, instruments: &Vec<Instrument>) {
        let (play_data, state) = triple_buffer_reader.read();
        if StateNoChange == state { return; }
        if play_data.tick != self.song_tick || play_data.row != self.song_row {

            let view_port = ViewPort {
                x1: 0,
                y1: 0,
                width: 200,
                height: 35
            };


            unsafe { term_writeln(CString::new(Display::move_to(1, 1)).unwrap().as_ptr()); }

            Display::display(play_data, instruments, view_port, &mut|str| {
                unsafe { term_writeln(CString::new(str).unwrap().as_ptr()); }
            });
            self.song_row = play_data.row;
            self.song_tick = play_data.tick;
        }
    }

    fn run(&self) {
        let sdl_context = sdl2::init().unwrap();
        let audio = sdl_context.audio().unwrap();

        // Must init video subsytem in order for keyboard input to work
        let video_subsystem = sdl_context.video().unwrap();
        let window = video_subsystem
            .window("Mod Player", 0, 0)
            .build()
            .unwrap();
        let _canvas = window.into_canvas().build().unwrap();

        let event_pump = leak!(sdl_context.event_pump().unwrap());

        let fps = -1; // call the function as fast as the browser wants to render (typically 60fps)
        let simulate_infinite_loop = 1; // call the function repeatedly

        let leaked_self = leak!(self);

        let mut audio_output: *mut c_void = 0 as *mut c_void;
        let mut triple_buffer_reader: Option<Arc<Mutex<TripleBufferReader<PlayData>>>> = None;
        let mut instruments: Vec<Instrument> = vec![];

        setup_mainloop(fps, simulate_infinite_loop, leaked_self, move |self_| unsafe {
            let leaked_pointer = leaked_self as *mut Self;
            let self_ = &mut *leaked_pointer;

            if triple_buffer_reader.is_some() {
                self_.handle_display(&mut triple_buffer_reader.as_ref().unwrap().lock().unwrap().deref_mut(), &instruments);
            }

            let mut cmds = CMDS.lock().unwrap();
            while cmds.len() > 0 {
                let cmd = cmds.pop_front();
                match cmd.unwrap() {
                    PlayerCmd::Stop => {
                        dbg!("Stop");
                        App::stop_audio(&mut audio_output, &mut triple_buffer_reader);
                    }
                    PlayerCmd::NewSong => {
                        dbg!("Start");

                        App::stop_audio(&mut audio_output, &mut triple_buffer_reader);

                        let desired_spec = AudioSpecDesired {
                            freq: Some(48000 as i32),
                            channels: Some(2),
                            samples: Some(AUDIO_BUF_FRAMES as u16)
                        };

                        let mut song = SongState::new("/file".to_string());
                        instruments = song.get_mut().song.lock().unwrap().get_instruments();

                        triple_buffer_reader =  Option::from(song.get_mut().get_triple_buffer_reader());
                        audio_output = leak!(audio.open_playback(None, &desired_spec, |spec| {
                            song.get_mut().song.lock().unwrap().set_sample_rate(spec.freq as f32);
                            let audio_cb = AudioCB { q: song.clone()};
                            audio_cb
                        }).unwrap());

                        Self::resume(audio_output);
                    }
                }
            }
            let leaked_event_pump = event_pump as *mut EventPump;
            let event_pump = &mut *leaked_event_pump;
            if handle_input(event_pump) {self_.stop();}
        });

    }

    fn stop_audio(audio_output: &mut *mut c_void, triple_buffer_reader: &mut Option<Arc<Mutex<TripleBufferReader<PlayData>>>>) {
        if *audio_output != 0 as *mut c_void {
            *triple_buffer_reader = None;
            Self::close_audio(*audio_output);
            *audio_output = 0 as *mut c_void;
        }
    }
}

fn handle_input(event_pump: &mut EventPump) -> bool {
    let mut last_time = SystemTime::now();
    let mut last_char = '\0';

    let mut tx = PLAYBACK_CMDS.lock().unwrap();

    // let input = tokio::time::timeout(Duration::from_secs(1), getter.getch()).await;
    for input in  event_pump.poll_iter() {
        if SystemTime::now() > last_time + Duration::from_secs(1) {
            last_char = '\0';
        }

        match input {
            Event::Quit { .. } => {
                let _ = tx.push_back(PlaybackCmd::Quit);
                return true;
            }
            Event::KeyUp { keycode, ..} => {
                if !keycode.is_some() {
                    continue;
                }
                let key = keycode.unwrap();
                match key {
                    Keycode::Escape | Keycode::Q => {
                        let _ = tx.push_back(PlaybackCmd::Quit);
                        return true;
                    }
                    Keycode::Num0 | Keycode::Num1 | Keycode::Num2 | Keycode::Num3 |
                    Keycode::Num4 | Keycode::Num5 | Keycode::Num6 | Keycode::Num7 |
                    Keycode::Num8 | Keycode::Num9 => {
                        let ch = ((key as i32 - Keycode::Num0 as i32) as u8 + '0' as u8) as char; // FIXME: Blachhh
                        if last_char != '\0' {
                            let channel_number = (last_char as u8 - '0' as u8) * 10 + (ch as u8 - '0' as u8);
                            if channel_number > 0 && channel_number <= 32 {
                                let _ = tx.push_back(PlaybackCmd::ChannelToggle(channel_number - 1));
                            }
                            last_char = '\0';
                        } else {
                            last_char = ch;
                        }
                    }
                    Keycode::Plus => {
                        let _ = tx.push_back(PlaybackCmd::IncSpeed);
                    }

                    Keycode::Minus => {
                        let _ = tx.push_back(PlaybackCmd::DecSpeed);
                    }
                    Keycode::Period => {
                        let _ = tx.push_back(PlaybackCmd::IncBPM);
                    }
                    Keycode::Comma => {
                        let _ = tx.push_back(PlaybackCmd::DecBPM);
                    }
                    Keycode::Space => {
                        let _ = tx.push_back(PlaybackCmd::PauseToggle);
                    }
                    Keycode::N => {
                        let _ = tx.push_back(PlaybackCmd::Next);
                    }
                    Keycode::Slash => {
                        let _ = tx.push_back(PlaybackCmd::LoopPattern);
                    }
                    Keycode::P => {
                        let _ = tx.push_back(PlaybackCmd::Prev);
                    }
                    Keycode::R => {
                        let _ = tx.push_back(PlaybackCmd::Restart);
                    }
                    Keycode::A => {
                        let _ = tx.push_back(PlaybackCmd::AmigaTable);
                    }
                    Keycode::L => {
                        let _ = tx.push_back(PlaybackCmd::LinearTable);
                    }
                    Keycode::F => {
                        let _ = tx.push_back(PlaybackCmd::FilterToggle);
                    }
                    Keycode::D => {
                        let _ = tx.push_back(PlaybackCmd::DisplayToggle);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    last_time = SystemTime::now();
    return false;
}

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


