#[macro_use]
extern crate lazy_static;
pub use simple_error::SimpleError as ExternalSimpleError;
pub use simple_error::SimpleResult as ExternalSimpleResult;

#[derive(Debug, Clone)]
pub struct SimpleError {
    message: String,
}

impl SimpleError {
    pub fn new(message: &str) -> Self {
        Self { message: message.to_string() }
    }
}

impl std::fmt::Display for SimpleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for SimpleError {}

impl From<std::io::Error> for SimpleError {
    fn from(e: std::io::Error) -> Self {
        Self { message: e.to_string() }
    }
}

impl From<&str> for SimpleError {
    fn from(s: &str) -> Self {
        Self { message: s.to_string() }
    }
}

impl From<String> for SimpleError {
    fn from(s: String) -> Self {
        Self { message: s }
    }
}

impl From<ExternalSimpleError> for SimpleError {
    fn from(e: ExternalSimpleError) -> Self {
        Self { message: e.to_string() }
    }
}

pub type SimpleResult<T> = Result<T, SimpleError>;

pub const AUDIO_BUF_FRAMES: usize   = 512;
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
mod it_fidelity_tests;
#[cfg(test)]
mod it_mapping_tests;
pub mod test_utils;


#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
