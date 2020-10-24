#[macro_use]
extern crate lazy_static;
extern crate simple_error;

pub mod io_helpers;
pub mod module_reader;
pub mod envelope;
pub mod instrument;
pub mod channel_state;
pub mod pattern;
pub mod producer_consumer_queue;
pub mod song;
pub mod tables;
pub mod triple_buffer;
pub mod song_state;


#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
