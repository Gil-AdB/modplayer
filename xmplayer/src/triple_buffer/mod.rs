use std::sync::atomic::{AtomicU32, AtomicPtr, Ordering};
use std::sync::atomic::Ordering::{Acquire, Release};
use std::sync::Arc;
use ::array_init::array_init;
use crate::triple_buffer::State::{StateNoChange, StateDirty};

pub trait Init {
    fn new() -> Self;
}

const READER:u32        = 0x03;
const WRITER:u32        = 0x0C;
const WRITER_SHIFT:u32  = 2;
const READY:u32         = 0x30;
const READY_SHIFT:u32   = 4;
const DIRTY:u32         = 0x40;

pub struct TripleBuffer<T> {
    buffer:     [T;3],
    // bits:
    // 0-1:     reader
    // 2-3:     writer
    // 4-5:     ready
    // 6:       dirty
    indexes:    AtomicU32,
}

#[derive(PartialEq, Eq)]
pub enum State {
    StateNoChange,
    StateDirty
}

pub struct TripleBufferReader<T> where T: Clone + Init {
    triple_buffer: Arc<AtomicPtr<TripleBuffer<T>>>
}

impl <T> TripleBufferReader<T> where T: Clone + Init {
    pub fn get(&mut self) -> &mut TripleBuffer<T> {
        unsafe{&mut *self.triple_buffer.load(Ordering::Acquire)}
    }

    // read() -> (&T, dirty?)
    pub fn read(&mut self) -> (&T, State) {
        let tb = self.get();
        loop {
            let current_indexes = tb.indexes.load(Acquire);
            if !TripleBuffer::<T>::get_dirty(current_indexes) {
                return (&tb.buffer[TripleBuffer::<T>::get_reader(current_indexes) as usize], StateNoChange)
            }
            // try to exchange ready slot with reader
            let new_indexes = TripleBuffer::<T>::swap_ready_and_reader(current_indexes);

            let result = tb.indexes.compare_and_swap(current_indexes, new_indexes, Release);

            if result != current_indexes { // failed to exchange, try again
                continue;
            }

            return (&tb.buffer[TripleBuffer::<T>::get_reader(new_indexes) as usize], StateDirty);
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

    pub fn write(&mut self) -> &mut T {
        let tb = self.get();
        loop {
            let current_indexes = tb.indexes.load(Acquire);
            // try to exchange ready slot with writer
            let new_indexes = TripleBuffer::<T>::swap_ready_and_writer(current_indexes);

            let result = tb.indexes.compare_and_swap(current_indexes, new_indexes, Release);

            if result != current_indexes { // failed to exchange, try again
                continue;
            }

            return &mut tb.buffer[TripleBuffer::<T>::get_writer(new_indexes) as usize];
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
        indexes & READER
    }

    fn get_writer(indexes: u32) -> u32 {
        (indexes & WRITER) >> WRITER_SHIFT
    }

    fn get_ready(indexes: u32) -> u32 {
        (indexes & READY) >> READY_SHIFT
    }

    fn get_dirty(indexes: u32) -> bool {
        (indexes & DIRTY) == DIRTY
    }

    // fn set_reader(indexes: u32, val: u32) -> u32 {
    //     indexes & !READER | val & 0x3
    // }
    //
    // fn set_writer(indexes: u32, val: u32) -> u32 {
    //     indexes & !WRITER | ((val & 0x3) << WRITER_SHIFT)
    // }
    //
    // fn set_ready(indexes: u32, val: u32) -> u32 {
    //     indexes & !READY | ((val & 0x3) << READY_SHIFT)
    // }

    fn swap_ready_and_writer(indexes: u32) -> u32 {
        let new_ready = Self::get_writer(indexes);
        let new_writer = Self::get_ready(indexes);
        indexes & !(READY | WRITER) | (new_writer << WRITER_SHIFT) | new_ready << READY_SHIFT | DIRTY
    }

    fn swap_ready_and_reader(indexes: u32) -> u32 {
        let new_ready = Self::get_reader(indexes);
        let new_reader = Self::get_ready(indexes);
        indexes & !(READER | READY | DIRTY) | (new_ready << READY_SHIFT) | new_reader
    }
}

// #[test]
// fn Test() {
//     let (mut triple_buffer_reader, mut triple_buffer_writer) = triple_buffer::<u32>::new();
//
//     let idx = 0;
//     assert!(triple_buffer_reader.read() == (&idx, StateNoChange))
//
//     //
//     // assert!(song.)
// }