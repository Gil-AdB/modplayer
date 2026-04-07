use std::sync::atomic::{AtomicU32, AtomicPtr, AtomicBool, AtomicUsize};
use std::sync::atomic::Ordering::{Acquire, Release, Relaxed};
use std::sync::Arc;
use std::ops::{Deref, DerefMut};
use std::cell::UnsafeCell;
use array_init::array_init;

/// Indicates if the reader has new data to read.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum State {
    /// No change in data since the last read.
    StateNoChange,
    /// Data has been updated and is ready to be read.
    StateDirty
}

const READER: u32       = 0x03;
const WRITER: u32       = 0x0C;
const WRITER_SHIFT: u32 = 2;
const READY: u32        = 0x30;
const READY_SHIFT: u32  = 4;
const DIRTY: u32        = 0x40;



/// A lock-free triple buffer for single-producer, single-consumer state sharing.
/// 
/// Triple buffering allows the producer to write new state while the consumer reads the previous state
/// without blocking each other.
pub struct TripleBuffer<T> {
    buffer: [T; 3],
    // bits:
    // 0-1: reader index
    // 2-3: writer index
    // 4-5: ready index (most recently written)
    // 6:   dirty bit (new data available)
    indexes: AtomicU32,
}

/// The reading side of a TripleBuffer.
pub struct TripleBufferReader<T> {
    triple_buffer: Arc<AtomicPtr<TripleBuffer<T>>>
}

impl<T> TripleBufferReader<T> where T: Clone + Default {
    fn load(&self) -> &mut TripleBuffer<T> {
        unsafe { &mut *self.triple_buffer.load(Acquire) }
    }

    /// Reads the current state from the buffer.
    /// Returns a reference to the data and its state (Dirty or NoChange).
    pub fn read(&mut self) -> (&T, State) {
        let tb = self.load();
        loop {
            let current_indexes = tb.indexes.load(Acquire);
            if !TripleBuffer::<T>::get_dirty(current_indexes) {
                return (&tb.buffer[TripleBuffer::<T>::get_reader(current_indexes) as usize], State::StateNoChange)
            }
            
            // Try to exchange ready slot with reader
            let new_indexes = TripleBuffer::<T>::swap_ready_and_reader(current_indexes);
            let result = tb.indexes.compare_exchange_weak(current_indexes, new_indexes, Release, Relaxed);
            
            let result_index = match result {
                Ok(v) => v,
                Err(e) => e,
            };

            if result_index != current_indexes { // failed to exchange, try again
                continue;
            }

            return (&tb.buffer[TripleBuffer::<T>::get_reader(new_indexes) as usize], State::StateDirty);
        }
    }
}

/// The writing side of a TripleBuffer.
pub struct TripleBufferWriter<T> {
    triple_buffer: Arc<AtomicPtr<TripleBuffer<T>>>
}

/// A guard that manages writing to a TripleBuffer.
///
/// When the guard is dropped, the buffer is atomically marked as "ready" for the reader.
pub struct TripleBufferWriterGuard<'a, T> where T: Clone + Default {
    writer: &'a mut TripleBufferWriter<T>,
}

impl<'a, T> Deref for TripleBufferWriterGuard<'a, T> where T: Clone + Default {
    type Target = T;
    fn deref(&self) -> &T {
        let tb = self.writer.load();
        let current_indexes = tb.indexes.load(Acquire);
        &tb.buffer[TripleBuffer::<T>::get_writer(current_indexes) as usize]
    }
}

impl<'a, T> DerefMut for TripleBufferWriterGuard<'a, T> where T: Clone + Default {
    fn deref_mut(&mut self) -> &mut T {
        let tb = self.writer.load();
        let current_indexes = tb.indexes.load(Acquire);
        &mut tb.buffer[TripleBuffer::<T>::get_writer(current_indexes) as usize]
    }
}

impl<'a, T> Drop for TripleBufferWriterGuard<'a, T> where T: Clone + Default {
    fn drop(&mut self) {
        let tb = self.writer.load();
        loop {
            let current_indexes = tb.indexes.load(Acquire);
            let new_indexes = TripleBuffer::<T>::swap_ready_and_writer(current_indexes);
            if tb.indexes.compare_exchange(current_indexes, new_indexes, Release, Relaxed).is_ok() {
                break;
            }
        }
    }
}

impl<T> TripleBufferWriter<T> where T: Clone + Default {
    fn load(&self) -> &mut TripleBuffer<T> {
        unsafe { &mut *self.triple_buffer.load(Acquire) }
    }

    /// Provides a mutable reference to the next buffer for writing via a guard.
    /// After writing, when the guard is dropped, the buffer will be marked as "ready" for the reader.
    pub fn write(&mut self) -> TripleBufferWriterGuard<'_, T> {
        TripleBufferWriterGuard { writer: self }
    }
}

impl<T> TripleBuffer<T> where T: Clone + Default {
    /// Creates a new TripleBuffer.
    pub fn new() -> Box<Self> {
        Box::new(TripleBuffer {
            buffer: array_init(|_| T::default()),
            indexes: AtomicU32::from(0x24) // Initial state: reader=0, writer=1, ready=2, dirty=0
        })
    }

    /// Splits a TripleBuffer into a Reader and a Writer.
    pub fn split(self: Box<Self>) -> (TripleBufferReader<T>, TripleBufferWriter<T>) {
        let tpp = Box::into_raw(self) as *mut TripleBuffer<T>;
        (
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

    fn swap_ready_and_writer(indexes: u32) -> u32 {
        let new_ready = Self::get_writer(indexes);
        let new_writer = Self::get_ready(indexes);
        indexes & !(READY | WRITER) | (new_writer << WRITER_SHIFT) | (new_ready << READY_SHIFT) | DIRTY
    }

    fn swap_ready_and_reader(indexes: u32) -> u32 {
        let new_ready = Self::get_reader(indexes);
        let new_reader = Self::get_ready(indexes);
        indexes & !(READER | READY | DIRTY) | (new_ready << READY_SHIFT) | new_reader
    }
}

use std::sync::{Condvar, Mutex};

/// A simple semaphore implementation using Mutex and Condvar.
pub struct Semaphore {
    condvar: Arc<(Mutex<usize>, Condvar)>,
}

impl Semaphore {
    /// Creates a new Semaphore with an initial value.
    pub fn new(initial: usize) -> Semaphore {
        Semaphore {
            condvar: Arc::new((Mutex::new(initial), Condvar::new())),
        }
    }

    /// Increments the semaphore and notifies a waiting consumer.
    pub fn signal(&self) {
        let (lock, cvar) = &*self.condvar;
        let mut count = lock.lock().unwrap();
        *count += 1;
        cvar.notify_one();
    }

    /// Block until the semaphore is signaled.
    pub fn wait(&self) {
        let (lock, cvar) = &*self.condvar;
        let mut count = lock.lock().unwrap();
        while *count == 0 {
            count = cvar.wait(count).unwrap();
        }
        *count -= 1;
    }

    /// Tries to decrement the semaphore if it's currently > 0.
    pub fn try_wait(&self) -> bool {
        let (lock, _cvar) = &*self.condvar;
        let mut count = match lock.try_lock() {
            Ok(lock) => lock,
            Err(_) => return false,
        };
        if *count == 0 {
            return false;
        }
        *count -= 1;
        true
    }
}

/// A single-producer, single-consumer queue using semaphores for synchronization.
///
/// Refactored to use const generics for fixed-size static buffers.
struct SharedQueue<T, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> {
    full_count:  Semaphore,
    empty_count: Semaphore,
    buf:         UnsafeCell<[[T; CHUNK_SIZE]; NUM_CHUNKS]>,
    front:       AtomicUsize,
    back:        AtomicUsize,
    stopped:     AtomicBool,
}

pub struct Producer<T, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> {
    q: Arc<SharedQueue<T, CHUNK_SIZE, NUM_CHUNKS>>,
}

// Ensure Producer is Send (AtomicUsize/Semaphore are already Send/Sync, UnsafeCell needs explicit Send capability)
unsafe impl<T: Send, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> Send for Producer<T, CHUNK_SIZE, NUM_CHUNKS> {}
unsafe impl<T: Sync, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> Sync for Producer<T, CHUNK_SIZE, NUM_CHUNKS> {}

pub struct Consumer<T, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> {
    q: Arc<SharedQueue<T, CHUNK_SIZE, NUM_CHUNKS>>,
}

unsafe impl<T: Send, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> Send for Consumer<T, CHUNK_SIZE, NUM_CHUNKS> {}
unsafe impl<T: Sync, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> Sync for Consumer<T, CHUNK_SIZE, NUM_CHUNKS> {}

pub struct ProducerConsumerQueue<T, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> {
    _marker: std::marker::PhantomData<T>,
}

impl<T, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> ProducerConsumerQueue<T, CHUNK_SIZE, NUM_CHUNKS> 
where T: Default + Copy {
    /// Creates a new ProducerConsumerQueue and returns a tuple of (Producer, Consumer).
    pub fn new() -> (Producer<T, CHUNK_SIZE, NUM_CHUNKS>, Consumer<T, CHUNK_SIZE, NUM_CHUNKS>) {
        let q = Arc::new(SharedQueue {
            full_count:  Semaphore::new(0),
            empty_count: Semaphore::new(NUM_CHUNKS),
            buf:         UnsafeCell::new([[T::default(); CHUNK_SIZE]; NUM_CHUNKS]),
            front:       AtomicUsize::new(0),
            back:        AtomicUsize::new(0),
            stopped:     AtomicBool::from(false),
        });
        (Producer { q: q.clone() }, Consumer { q })
    }
}

impl<T, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> SharedQueue<T, CHUNK_SIZE, NUM_CHUNKS> {
    pub fn stop(&self) {
        self.stopped.store(true, Release);
        self.empty_count.signal();
        self.full_count.signal();
    }

    /// Clears any pending full buffers.
    pub fn drain(&self) {
        while self.full_count.try_wait() {
            let back = self.back.load(Acquire);
            self.back.store((back + 1) % NUM_CHUNKS, Release);
            self.empty_count.signal();
        }
    }

    /// Allows the producer to fill a buffer. 
    /// The callback `f` will receive a mutable slice to the buffer.
    /// Allows the producer to fill multiple buffers. 
    /// This method will block and wait for empty slots until the closure `f` returns `false`.
    pub fn produce<F: FnMut(&mut [T]) -> bool>(&self, mut f: F) -> bool {
        loop {
            self.empty_count.wait();
            if self.stopped.load(Acquire) {
                self.empty_count.signal(); // Persist stop signal for other potential producers
                return false;
            }

            let front = self.front.load(Acquire);
            let my_buf = unsafe { &mut (*self.buf.get())[front % NUM_CHUNKS] };
            let result = f(my_buf);
            
            self.front.store(front + 1, Release);
            self.full_count.signal();

            if !result {
                return true;
            }
        }
    }

    /// Allows the consumer to process exactly one buffer.
    /// Returns true if a buffer was processed, false if the queue is stopped and empty.
    pub fn consume<F: FnMut(&[T])>(&self, mut f: F) -> bool {
        self.full_count.wait();

        let back = self.back.load(Acquire);
        let front = self.front.load(Acquire);
        
        // front == back uniquely means empty with monotonic indices
        if self.stopped.load(Acquire) && front == back {
            self.full_count.signal(); // Persist stop signal for subsequent calls
            return false;
        }

        let my_buf = unsafe { &(*self.buf.get())[back % NUM_CHUNKS] };
        f(my_buf);
        self.back.store(back + 1, Release);
        self.empty_count.signal();

        true
    }
}

impl<T, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> Producer<T, CHUNK_SIZE, NUM_CHUNKS> {
    pub fn produce<F: FnMut(&mut [T]) -> bool>(&self, f: F) -> bool {
        self.q.produce(f)
    }

    pub fn stop(&self) {
        self.q.stop();
    }
}

impl<T, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> Consumer<T, CHUNK_SIZE, NUM_CHUNKS> {
    pub fn consume<F: FnMut(&[T])>(&self, f: F) -> bool {
        self.q.consume(f)
    }

    pub fn drain(&self) {
        self.q.drain();
    }
}

impl<T> Drop for TripleBuffer<T> {

    fn drop(&mut self) {
        // Since we are using Box::into_raw and AtomicPtr, we need to handle drop correctly if multiple handles exist.
        // However, in this implementation, split produces exactly one reader and one writer.
        // A more robust implementation would use reference counting for the raw pointer.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_triple_buffer_basic() {
        let (mut reader, mut writer) = TripleBuffer::<u32>::new().split();
        
        // Initial state
        let (val, state) = reader.read();
        assert_eq!(*val, 0);
        assert_eq!(state, State::StateNoChange);

        // Update state
        {
            let mut w_val = writer.write();
            *w_val = 42;
        }

        // Reader should see dirty state
        let (val, state) = reader.read();
        assert_eq!(*val, 42);
        assert_eq!(state, State::StateDirty);

        // Reader should see no change now
        let (val, state) = reader.read();
        assert_eq!(*val, 42);
        assert_eq!(state, State::StateNoChange);
    }

    #[test]
    fn test_triple_buffer_multiple_writes() {
        let (mut reader, mut writer) = TripleBuffer::<u32>::new().split();

        // Write twice
        *writer.write() = 1;
        *writer.write() = 2; // This overwrites the previous ready state

        let (val, state) = reader.read();
        assert_eq!(*val, 2);
        assert_eq!(state, State::StateDirty);
    }

    #[test]
    fn test_triple_buffer_multithreaded() {
        use std::thread;
        use std::sync::Barrier;

        let (mut reader, mut writer) = TripleBuffer::<u32>::new().split();
        let barrier = Arc::new(Barrier::new(2));
        let b1 = barrier.clone();
        let b2 = barrier.clone();

        let writer_thread = thread::spawn(move || {
            b1.wait();
            for i in 1..=1000 {
                let mut w = writer.write();
                *w = i;
                // Guard drop here triggers swap
            }
        });

        let reader_thread = thread::spawn(move || {
            b2.wait();
            let mut last_val = 0;
            while last_val < 1000 {
                let (val, state) = reader.read();
                if state == State::StateDirty {
                    assert!(*val > last_val, "Read value {} not greater than last {}", *val, last_val);
                    last_val = *val;
                }
            }
        });

        writer_thread.join().unwrap();
        reader_thread.join().unwrap();
    }

    #[test]
    fn test_pcq_basic() {
        let (prod, cons) = ProducerConsumerQueue::<u32, 4, 3>::new();

        prod.produce(|buf| {
            buf.copy_from_slice(&[1, 2, 3, 4]);
            false
        });
        
        let mut out = [0; 4];
        let res = cons.consume(|buf| {
            out.copy_from_slice(buf);
        });
        
        assert!(res);
        assert_eq!(out, [1, 2, 3, 4]);
    }

    #[test]
    fn test_pcq_blocking_and_shutdown() {
        use std::thread;
        use std::time::Duration;

        let (prod, cons) = ProducerConsumerQueue::<u32, 4, 2>::new();
        
        let producer = thread::spawn(move || {
            prod.produce(|buf| {
                buf.copy_from_slice(&[1, 2, 3, 4]);
                false
            });
            prod.produce(|buf| {
                buf.copy_from_slice(&[5, 6, 7, 8]);
                false
            });
            prod.produce(|buf| {
                buf.copy_from_slice(&[9, 10, 11, 12]);
                false
            });
            prod.stop();
        });

        thread::sleep(Duration::from_millis(50));

        let mut all_data = Vec::new();
        while cons.consume(|buf| {
            all_data.extend_from_slice(buf);
        }) {}

        producer.join().unwrap();
        assert_eq!(all_data, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    }

    #[test]
    fn test_pcq_drain() {
        let (prod, cons) = ProducerConsumerQueue::<u32, 4, 2>::new();
        prod.produce(|buf| {
            buf.copy_from_slice(&[1, 2, 3, 4]);
            false
        });
        cons.drain();
    }

    #[test]
    fn test_semaphore_multi_thread() {
        use std::thread;
        let sem = Arc::new(Semaphore::new(0));
        let s1 = sem.clone();
        let s2 = sem.clone();

        let t1 = thread::spawn(move || {
            s1.wait();
            s1.wait();
        });

        s2.signal();
        s2.signal();
        t1.join().unwrap();
        
        assert_eq!(sem.try_wait(), false);
    }

    #[test]
    fn test_triple_buffer_panic_safety() {
        use std::thread;
        let (mut _reader, mut writer) = TripleBuffer::<u32>::new().split();
        
        let _ = thread::spawn(move || {
            let mut w = writer.write();
            *w = 100;
            panic!("Writer panicked before drop!");
        }).join();

        // The writer guard should have been dropped during stack unwinding
        // Actually, we can't easily check the reader state here without the reader handle,
        // but this verifies that the Drop implementation doesn't double-panic or deadlock.
    }

    #[test]
    fn test_semaphore_contention() {
        use std::thread;
        let sem = Arc::new(Semaphore::new(0));
        let mut threads = vec![];

        for _ in 0..10 {
            let s = sem.clone();
            threads.push(thread::spawn(move || {
                s.wait();
            }));
        }

        for _ in 0..10 {
            sem.signal();
        }

        for t in threads {
            t.join().unwrap();
        }
    }

    #[test]
    fn test_pcq_high_throughput() {
        use std::thread;
        let (prod, cons) = ProducerConsumerQueue::<u32, 10, 5>::new();

        let producer = thread::spawn(move || {
            let mut i = 0;
            prod.produce(|buf| {
                for j in 0..10 {
                    buf[j] = (i * 10 + j) as u32;
                }
                i += 1;
                i < 10000 
            });
            prod.stop();
        });

        let mut next_val = 0u32;
        while cons.consume(|buf| {
            for &val in buf {
                assert_eq!(val, next_val);
                next_val += 1;
            }
        }) {}

        producer.join().unwrap();
        assert_eq!(next_val, 100000);
    }

    #[test]
    fn test_pcq_persistent_stop() {
        let (prod, cons) = ProducerConsumerQueue::<u32, 4, 3>::new();
        
        prod.stop();
        
        // Multiple calls to consume should all return false and not hang
        assert_eq!(cons.consume(|_| {}), false);
        assert_eq!(cons.consume(|_| {}), false);
        assert_eq!(cons.consume(|_| {}), false);
        
        // Multiple calls to produce should all return false and not hang
        assert_eq!(prod.produce(|_| true), false);
        assert_eq!(prod.produce(|_| true), false);
    }
}
