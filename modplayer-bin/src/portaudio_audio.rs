extern crate portaudio;
use portaudio as pa;
type ErrorType = pa::Error;
use portaudio::{NonBlocking, Output};
use portaudio::stream::OutputSettings;
use xmplayer::song_state::SongHandle;
use xmplayer::producer_consumer_queue::{AUDIO_BUF_FRAMES, AUDIO_BUF_SIZE};


pub(crate) struct AudioOutput {
    stream: pa::Stream<NonBlocking, Output<f32>>,
}

impl AudioOutput {
    pub fn new(song_handle: &mut SongHandle, sample_rate: f32) -> Self {
        // let pa_result: Result<pa::PortAudio, pa::Error> = pa::PortAudio::new();
        // let _pa = match pa_result {
        //     Ok(p) => p,
        //     Err(e) => return Err(e),
        // };
        const CHANNELS: i32 = 2;
        // const NUM_SECONDS: i32 = 500;

        let pa = pa::PortAudio::new().unwrap();
        let settings =
            pa.default_output_stream_settings(CHANNELS, sample_rate as f64, AUDIO_BUF_FRAMES as u32).unwrap();

        let mut qclone = song_handle.get_mut().get_queue();

        // This routine will be called by the PortAudio engine when audio is needed. It may called at
        // interrupt level on some machines so don't do anything that could mess up the system like
        // dynamic resource allocation or IO.
        let callback = move |pa::OutputStreamCallbackArgs { buffer, frames, .. }| {
            if frames != AUDIO_BUF_FRAMES { panic!("unexpected frame size: {}", frames); }

            if !qclone.get().consume(|buf: &[f32; AUDIO_BUF_SIZE]| { buffer.clone_from_slice(buf); }) {
                pa::Complete
            } else {
                pa::Continue
            }
        };

        let mut stream = pa.open_non_blocking_stream(settings, callback).unwrap();

        Self {
            stream
        }
    }

    pub fn start_audio_output(&mut self) {
        self.stream.start().unwrap();
    }

    pub fn close(&mut self) {
        self.stream.stop().unwrap();
        self.stream.close().unwrap();
    }
}
