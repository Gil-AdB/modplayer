mod console;
mod leak;

use std::cmp::max;
use wasm_bindgen::prelude::*;
use xmplayer::song::{PlaybackCmd, PlayData, CallbackState, Song};
extern crate console_error_panic_hook;
use xmplayer::song_state::{SongHandle};
use std::sync::{mpsc, Arc};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Mutex;

use std::ffi::c_void;
use xmplayer::instrument::Instrument;
use display::{Display, display::TargetPlatform};
use xmplayer::module_reader::{open_module, Patterns};
use shared_sync_primitives::{TripleBufferReader, TripleBuffer};
use xmplayer::song::PlanarBufferAdaptar;

use wasm_bindgen::JsCast;



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
//
// pub enum PlayerCmd {
//     Stop,
//     NewSong(String)
// }

// // mpsc is just too much hassle. I tried.
// lazy_static!(
//     static ref CMDS: Mutex<VecDeque<PlayerCmd>> = Mutex::new(VecDeque::new());
//     static ref PLAYBACK_CMDS: Mutex<VecDeque<PlaybackCmd>> = Mutex::new(VecDeque::new());
// );

#[allow(dead_code)]
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
    instruments:                        Vec<Instrument>,
    patterns:                           Vec<Patterns>,
    order:                              Vec<u8>,
    scroll_offset:                      isize,
    scroll_offset_x:                    isize,
    grid:                               display::grid::Grid,
    downsampled_scopes:                 Vec<f32>,
}

use js_sys::{Array, JsString};
use display::RGB;

#[wasm_bindgen(module = "/export.js")]
extern "C" {
    pub fn term_writeln(str: String);
    pub fn term_writeln_with_background(str: String, background: RGB);
}

#[wasm_bindgen]
impl SongJs {
    pub fn new(sample_rate:f32, data: &[u8]) -> Self {
        console_error_panic_hook::set_once();
        let data = open_module(data).unwrap();
        let triple_buffer = TripleBuffer::<PlayData>::new();
        let (triple_buffer_reader, triple_buffer_writer) = triple_buffer.split();
        let song = Song::new(&data, triple_buffer_writer, sample_rate);
        let (tx, rx): (Sender<PlaybackCmd>, Receiver<PlaybackCmd>) = mpsc::channel();
        let instruments = song.get_instruments();
        let patterns = song.get_patterns();
        let order = song.get_order();

        let width = 200;
        let height = 50;

        Self {
            song,
            triple_buffer_reader: Arc::new(Mutex::new(triple_buffer_reader)),
            song_row: 0,
            song_tick: 2000,
            tx,
            rx,
            last_time: Instant::now(),
            last_char: '\0',
            instruments,
            patterns,
            order,
            scroll_offset: 0,
            scroll_offset_x: 0,
            grid: display::grid::Grid::new(width, height),
            downsampled_scopes: vec![0.0f32; 64 * 128], // Support up to 64 channels
        }
    }

    pub fn toggle_panning(&mut self) {
    }

    pub fn display(&mut self, view_mode: u32, theme_id: u32) {
        let tbr = self.triple_buffer_reader.lock().unwrap();
        let (play_data, _state) = tbr.get_read_buffer();
        
        // Copy dimensions to avoid borrow conflicts in Display::render
        let width = self.grid.width;
        let height = self.grid.height;
        
        Display::render(&mut self.grid, play_data, &self.instruments, &self.patterns, &self.order, width, height, view_mode, theme_id, self.scroll_offset_x, self.scroll_offset, TargetPlatform::WASM);
        
        self.song_row = play_data.row;
        self.song_tick = play_data.tick;

        // Perform 4x Downsampling (512 -> 128) for all channels
        let num_channels = play_data.channel_status.len().min(64);
        for ch in 0..num_channels {
            let dst_offset = ch * 128;
            if play_data.channel_status[ch].on {
                let src = &play_data.channel_status[ch].oscilloscope;
                for i in 0..128 {
                    // Simple decimation (pick every 4th sample) - fast and sufficient for UI
                    self.downsampled_scopes[dst_offset + i] = src[i * 4];
                }
            } else {
                for i in 0..128 {
                    self.downsampled_scopes[dst_offset + i] = 0.0;
                }
            }
        }
    }

    pub fn get_grid_ptr(&self) -> *const u8 {
        // Binary format is [c, fr, fg, fb, br, bg, bb] per cell
        // We'll use a custom optimized binary representation
        // Actually to_binary is okay, but we return its ptr
        // Wait: we need to persist the binary Vec too if we want a stable pointer.
        // Let's just return the pointer to the cells themselves if JS can handle it.
        // The display::grid::Cell is [char, RGB, RGB].
        // Let's just create a persisted binary buffer in SongJs.
        self.grid.cells.as_ptr() as *const u8
    }

    pub fn get_grid_size(&self) -> usize {
        self.grid.cells.len() * std::mem::size_of::<display::grid::Cell>()
    }

    pub fn get_scopes_ptr(&self) -> *const f32 {
        self.downsampled_scopes.as_ptr()
    }

    pub fn get_scopes_len(&self) -> usize {
        self.downsampled_scopes.len()
    }

    // Lightweight Metadata Getters
    pub fn get_row(&self) -> usize { self.song_row }
    pub fn get_tick(&self) -> u32 { self.song_tick }
    
    pub fn get_bpm(&self) -> u32 {
        let tbr = self.triple_buffer_reader.lock().unwrap();
        let (play_data, _) = tbr.get_read_buffer();
        play_data.bpm
    }

    pub fn get_speed(&self) -> u32 {
        let tbr = self.triple_buffer_reader.lock().unwrap();
        let (play_data, _) = tbr.get_read_buffer();
        play_data.speed
    }

    pub fn get_pos(&self) -> u32 {
        let tbr = self.triple_buffer_reader.lock().unwrap();
        let (play_data, _) = tbr.get_read_buffer();
        play_data.song_position as u32
    }

    pub fn get_play_data(&self) -> JsValue {
        // Kept for backward compatibility if needed, but deprecated
        let tbr = self.triple_buffer_reader.lock().unwrap();
        let (play_data, _state) = tbr.get_read_buffer();
        serde_wasm_bindgen::to_value(play_data).unwrap()
    }

    pub fn scroll(&mut self, delta: isize) {
        self.scroll_offset = max(0, self.scroll_offset + delta);
    }

    pub fn scroll_x(&mut self, delta: isize) {
        self.scroll_offset_x = max(0, self.scroll_offset_x + delta);
    }

    pub fn get_channel_count(&self) -> usize {
        self.song.get_channel_count()
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
                "F1" => { let _ = tx.send(PlaybackCmd::SetViewMode(0)); }
                "F2" => { let _ = tx.send(PlaybackCmd::SetViewMode(1)); }
                "F3" => { let _ = tx.send(PlaybackCmd::SetViewMode(2)); }
                "F4" => { let _ = tx.send(PlaybackCmd::SetViewMode(3)); }
                "T" => {
                    let _ = tx.send(PlaybackCmd::CycleTheme);
                }
                "S" => {
                    let _ = tx.send(PlaybackCmd::ToggleScopes);
                }
                "v" | "V" => {
                    let _ = tx.send(PlaybackCmd::ToggleVisualizerMode);
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

#[allow(dead_code)]
struct App {
    song_row:       usize,
    song_tick:      u32,
}

impl App {
    #[allow(dead_code)]
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

    #[allow(dead_code)]
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
