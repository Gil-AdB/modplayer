use xmplayer::AudioConsumer;

pub(crate) struct AudioOutput {
    pub(crate) consumer: AudioConsumer,
}

impl AudioOutput {
    pub fn new(consumer: AudioConsumer, _sample_rate: f32) -> Self {
        Self { consumer }
    }

    pub fn start_audio_output(&mut self) {}

    pub fn close(&mut self) {}
}
