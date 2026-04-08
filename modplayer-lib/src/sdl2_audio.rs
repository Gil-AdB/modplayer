#[macro_use]

use xmplayer::{AUDIO_BUF_SIZE, NUM_AUDIO_CHUNKS, AudioConsumer};
use core::option::Option::{Some, None};

use sdl2::{Error, AudioSubsystem, audio::{AudioSpecDesired, AudioCallback, AudioDevice}};

struct AudioCB {
   q: AudioConsumer
}

impl AudioCallback for AudioCB {
   type Channel = f32;

   fn callback(&mut self, out: &mut [f32]) {
       if out.len() != AUDIO_BUF_SIZE {panic!("unexpected frame size: {}", out.len());}

       self.q.consume(|buf: &[f32]| { out.clone_from_slice(buf); });
   }
}

type ErrorType = Error;

pub(crate) struct AudioOutput {
    sdl_context: sdl2::Sdl,
    audio: AudioSubsystem,
    // desired_spec: AudioSpecDesired,
    audio_output: AudioDevice<AudioCB>,
}

impl AudioOutput {
    pub fn new(consumer: AudioConsumer, sample_rate: f32) -> Self {
        let sdl_context = sdl2::init().unwrap();
        let audio = sdl_context.audio().unwrap();
        let desired_spec = AudioSpecDesired {
            freq: Some(sample_rate as i32),
            channels: Some(2),
            samples: Some((AUDIO_BUF_SIZE / 2) as u16)
        };

        let audio_output = audio.open_playback(None, &desired_spec, |_spec| {
            AudioCB{ q: consumer }
        }).unwrap();

        Self {
            sdl_context,
            audio,
            // desired_spec,
            audio_output,
        }
    }

    pub fn start_audio_output(&mut self) {
        self.audio_output.resume();
    }

    pub fn close(&mut self) {
        self.audio_output.pause();
    }
}


