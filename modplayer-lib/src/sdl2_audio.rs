#[macro_use]

use xmplayer::song_state::SongHandle;
use xmplayer::producer_consumer_queue::AUDIO_BUF_SIZE;
use core::option::Option::{Some, None};

use sdl2::{Error, AudioSubsystem, audio::{AudioSpecDesired, AudioCallback, AudioDevice}};


use std::thread::JoinHandle;

struct AudioCB {
   q: SongHandle
}

impl AudioCallback for AudioCB {
   type Channel = f32;

   fn callback(&mut self, out: &mut [f32]) {
       if out.len() != AUDIO_BUF_SIZE {panic!("unexpected frame size: {}", out.len());}

       self.q.get_mut().get_queue().get().consume(|buf: &[f32]| { out.clone_from_slice(buf); });
   }
}

type ErrorType = Error;

pub(crate) struct AudioOutput {
    sdl_context: sdl2::Sdl,
    audio: AudioSubsystem,
    // desired_spec: AudioSpecDesired,
    audio_output: AudioDevice<AudioCB>,
    play_thread: Option<JoinHandle<()>>,
    display_thread: Option<JoinHandle<()>>,
}

impl AudioOutput {
    pub fn new(song_handle: SongHandle, sample_rate: f32) -> Self {
        let sdl_context = sdl2::init().unwrap();
        let audio = sdl_context.audio().unwrap();
        let desired_spec = AudioSpecDesired {
            freq: Some(sample_rate as i32),
            channels: Some(2),
            samples: Some((AUDIO_BUF_SIZE / 2) as u16)
        };

        let audio_output = audio.open_playback(None, &desired_spec, |_spec| {
            //song_handle.get_mut().song.lock().unwrap().set_sample_rate(spec.freq as f32);
            let cb = AudioCB{ q: song_handle.clone()};
            cb
        }).unwrap();

        Self {
            sdl_context,
            audio,
            // desired_spec,
            audio_output,
            play_thread: None,
            display_thread: None,
        }
    }

    pub fn start_audio_output(&mut self) {
        let h = self.audio_output.lock().q.get_mut().start(|_data, _instruments| {});
        self.play_thread = h.0;
        self.display_thread = h.1;

        self.audio_output.resume();
    }

    pub fn set_order(&mut self, order: u32) {
        self.audio_output.lock().q.get_mut().set_order(order);
    }


    pub fn close(&mut self) {
        self.audio_output.lock().q.get_mut().close();
        if self.play_thread.is_some() {
            self.play_thread.take().map(JoinHandle::join);
        }
        if self.display_thread.is_some() {
            self.display_thread.take().map(JoinHandle::join);
        }
        //self.audio_output.close_and_get_callback();
    }

}


