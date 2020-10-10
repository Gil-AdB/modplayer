extern crate sdl2;
extern crate xmplayer;

mod emscripten_boilerplate;

use emscripten_boilerplate::{setup_mainloop, emscripten_cancel_main_loop};
use sdl2::pixels::Color;
use sdl2::rect::Rect;
use sdl2::audio::{AudioCallback, AudioSpecDesired};
use xmplayer::producer_consumer_queue::{AUDIO_BUF_SIZE, ProducerConsumerQueue, AUDIO_BUF_FRAMES};
use xmplayer::producer_consumer_queue::{PCQHolder};
use xmplayer::song::{Song, PlaybackCmd, PlayData, CallbackState};
use xmplayer::module_reader::{SongData, read_module, print_module};
use xmplayer::song_state::{SongState, SongHandle};
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};


struct AudioCB {
    q: SongHandle
}

impl AudioCallback for AudioCB {
    type Channel = f32;

    fn callback(&mut self, out: &mut [f32]) {
        let (tx, mut rx): (Sender<PlaybackCmd>, Receiver<PlaybackCmd>) = mpsc::channel();
        let mut song = self.q.get().song.lock().unwrap();

        song.get_next_tick(out, &mut rx);
    }
}


pub fn main() {

    let sdl_context = sdl2::init().unwrap();

    let audio = sdl_context.audio().unwrap();

    let desired_spec = AudioSpecDesired {
        freq: Some(44100 as i32),
        channels: Some(2),
        samples: Some(1024 as u16)
    };

    println!("1");

    let mut song = SongState::new("/modplayer-wasm/src/static/thraddash.mod".to_string());
    println!("2");

    // let handle = song.get().start(44100.0, |data| {
    //     // Display::display(data, 0);
    // });
    println!("3");


    let mut audio_cb = AudioCB{ q: song.clone()};

    // for (i, s) in audio_cb.sine.iter_mut().enumerate() {
    //     *s = f32::sin((i as f32 / BUF_SIZE as f32) * std::f32::consts::PI * 2.0) * 0.25;
    // }

    let audio_output = audio.open_playback(None, &desired_spec, |spec| {
        audio_cb
    }).unwrap();

    audio_output.resume();

    let video_subsystem = sdl_context.video().unwrap();
    let window = video_subsystem
        .window("canvas", 255, 255)
        .build()
        .unwrap();
    let mut canvas = window.into_canvas().build().unwrap();

    let fps = -1; // call the function as fast as the browser wants to render (typically 60fps)
    let simulate_infinite_loop = 1; // call the function repeatedly
    let mut iteration = 0;

    setup_mainloop(fps, simulate_infinite_loop, move || unsafe {
        // example: draw a moving rectangle

        // red background
        canvas.set_draw_color(Color::RGB(255, 255, 0));
        canvas.clear();


        println!("{}", iteration);

        if iteration == 300 {
            emscripten_cancel_main_loop();
        }

        // moving blue rectangle
        iteration = iteration + 1;
        canvas.set_draw_color(Color::RGB(255, 0, 255));
        let rect = Rect::new(iteration & 0xFF, 50, 50, 50);

        println!("{:?}", rect);


        let res = canvas.fill_rect(rect);
        println!("{:?}", res);
    });


    audio_output.close_and_get_callback();

}

