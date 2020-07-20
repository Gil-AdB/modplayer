use std::sync::atomic::{AtomicU32, AtomicPtr, Ordering};
use std::sync::atomic::Ordering::{Acquire, Release};
use std::io::Read;
use std::sync::Arc;
use ::array_init::array_init;

pub trait Init {
    fn new() -> Self;
}

pub struct TripleBuffer<T> {
    buffer:     [T;3],
    // bits:
    // 0-1:     reader
    // 2-3:     writer
    // 4-5:     ready
    indexes:    AtomicU32,
}

pub struct TripleBufferReader<T> where T: Clone + Init {
    triple_buffer: Arc<AtomicPtr<TripleBuffer<T>>>
}

impl <T> TripleBufferReader<T> where T: Clone + Init {
    pub fn get(&mut self) -> &mut TripleBuffer<T> {
        unsafe{&mut *self.triple_buffer.load(Ordering::Acquire)}
    }

    pub fn read(&mut self) -> &T {
        let tb = self.get();
        loop {
            let current_indexes = tb.indexes.load(Acquire);
            // try to exchange ready slot with reader
            let new_indexes = TripleBuffer::<T>::swap_ready_and_reader(current_indexes);

            let result = tb.indexes.compare_and_swap(current_indexes, new_indexes, Release);

            if result != current_indexes { // failed to exchange, try again
                continue;
            }

            return &tb.buffer[TripleBuffer::<T>::get_reader(new_indexes) as usize];
        }
    }

}

pub struct TripleBufferWriter<T> where T: Clone + Init {
    triple_buffer: Arc<AtomicPtr<TripleBuffer<T>>>
}

impl <T> TripleBufferWriter<T> where T: Clone + Init {
    pub fn get(&mut self) -> &mut TripleBuffer<T> {
        unsafe{&mut *self.triple_buffer.load(Ordering::Acquire)}
    }

    pub fn write(&mut self, val: &T) {
        let tb = self.get();
        loop {
            let current_indexes = tb.indexes.load(Acquire);
            // try to exchange ready slot with writer
            let new_indexes = TripleBuffer::<T>::swap_ready_and_writer(current_indexes);

            let result = tb.indexes.compare_and_swap(current_indexes, new_indexes, Release);

            if result != current_indexes { // failed to exchange, try again
                continue;
            }

            tb.buffer[TripleBuffer::<T>::get_writer(new_indexes) as usize] = val.clone();
            return;
        }
    }
}

impl<T> TripleBuffer<T> where T: Clone + Init {

    pub fn new() -> (TripleBufferReader<T>, TripleBufferWriter<T>) {
        let triple_buffer =
            Box::new(
                TripleBuffer {
                    buffer: array_init(|_| T::new()),//[; 3],
                    indexes: AtomicU32::from(0x24)
                });
        let tpp = Box::into_raw(triple_buffer) as *mut TripleBuffer<T>;
        return (
            TripleBufferReader {
                triple_buffer: Arc::new(AtomicPtr::new(tpp))
            },
            TripleBufferWriter {
                triple_buffer: Arc::new(AtomicPtr::new(tpp))
            }
        )
    }

    fn get_reader(indexes: u32) -> u32 {
        indexes & 0x03
    }

    fn get_writer(indexes: u32) -> u32 {
        (indexes & 0xC) >> 2
    }

    fn get_ready(indexes: u32) -> u32 {
        (indexes & 0x30) >> 4
    }

    fn set_reader(indexes: u32, val: u32) -> u32 {
        indexes & !0x03 | val & 0x3
    }

    fn set_writer(indexes: u32, val: u32) -> u32 {
        indexes & !0x0C | ((val & 0x3) << 2)
    }

    fn set_ready(indexes: u32, val: u32) -> u32 {
        indexes & !0x30 | ((val & 0x3) << 4)
    }

    fn swap_ready_and_writer(indexes: u32) -> u32 {
        let writer = Self::get_writer(indexes);
        let ready = Self::get_ready(indexes);
        indexes & !0x3C | (writer << 4) | ready << 2
    }

    fn swap_ready_and_reader(indexes: u32) -> u32 {
        let reader = Self::get_reader(indexes);
        let ready = Self::get_ready(indexes);
        indexes & !0x33 | (reader << 4) | ready
    }
}

