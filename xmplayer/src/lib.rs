#[macro_use]
extern crate lazy_static;
pub extern crate simple_error;

pub const AUDIO_BUF_FRAMES: usize   = 1024;
pub const AUDIO_BUF_SIZE: usize     = AUDIO_BUF_FRAMES * 2;
pub const NUM_AUDIO_CHUNKS: usize   = 3;

pub type AudioConsumer = shared_sync_primitives::Consumer<f32, AUDIO_BUF_SIZE, NUM_AUDIO_CHUNKS>;
pub type AudioProducer = shared_sync_primitives::Producer<f32, AUDIO_BUF_SIZE, NUM_AUDIO_CHUNKS>;

pub mod module_reader;
pub mod envelope;
pub mod instrument;
pub mod channel_state;
pub mod pattern;
pub mod song;
pub mod tables;
pub mod song_state;


#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
