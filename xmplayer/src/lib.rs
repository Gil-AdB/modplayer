#![feature(generators, generator_trait)]
#![feature(vec_drain_as_slice)]
#![feature(slice_fill)]
// #![feature(in_band_lifetimes)]


pub mod io_helpers;
pub mod xm_reader;
pub mod envelope;
pub mod instrument;
pub mod channel_state;
pub mod pattern;
pub mod producer_consumer_queue;
pub mod song;


#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
