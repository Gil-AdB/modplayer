// #![feature(generators, generator_trait)]
// #![feature(slice_fill)]
// #![feature(const_fn)]
// #![feature(seek_convenience)]
// #![feature(box_syntax)]
// #![feature(in_band_lifetimes)]
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
