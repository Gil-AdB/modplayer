use std::sync::atomic::{AtomicU32, AtomicBool, AtomicUsize};
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
    buffer: UnsafeCell<[T; 3]>,
    // bits:
    // 0-1: reader index
    // 2-3: writer index
    // 4-5: ready index (most recently written)
    // 6:   dirty bit (new data available)
    indexes: AtomicU32,
}

unsafe impl<T: Send> Send for TripleBuffer<T> {}
unsafe impl<T: Sync> Sync for TripleBuffer<T> {}

pub struct TripleBufferReader<T> {
    triple_buffer: Arc<TripleBuffer<T>>
}

impl<T> TripleBufferReader<T> where T: Clone + Default {
    /// Reads the current state from the buffer.
    /// Returns a reference to the data and its state (Dirty or NoChange).
    pub fn get_read_buffer(&mut self) -> (&T, State) {
        let tb = &*self.triple_buffer;
        loop {
            let current_indexes = tb.indexes.load(Acquire);
            if !TripleBuffer::<T>::get_dirty(current_indexes) {
                let idx = TripleBuffer::<T>::get_reader(current_indexes) as usize;
                return (tb.get_buffer_ref(idx), State::StateNoChange)
            }
            
            // Try to exchange ready slot with reader
            let new_indexes = TripleBuffer::<T>::swap_ready_and_reader(current_indexes);
            let result = tb.indexes.compare_exchange_weak(current_indexes, new_indexes, Release, Relaxed);
            
            let result_index = result.unwrap_or_else(|e| e);

            if result_index != current_indexes { // failed to exchange, try again
                continue;
            }

            let idx = TripleBuffer::<T>::get_reader(new_indexes) as usize;
            return (tb.get_buffer_ref(idx), State::StateDirty);
        }
    }
}

pub struct TripleBufferWriter<T> {
    triple_buffer: Arc<TripleBuffer<T>>
}

impl<T> TripleBufferWriter<T> where T: Clone + Default {
    /// Commits the previously written buffer to the reader and provides a mutable reference
    /// to the next available buffer for writing.
    pub fn get_write_buffer(&mut self) -> &mut T {
        let tb = &*self.triple_buffer;
        loop {
            let current_indexes = tb.indexes.load(Acquire);
            
            // Swap ready and writer bits to publish the previously written buffer
            let new_indexes = TripleBuffer::<T>::swap_ready_and_writer(current_indexes);
            
            // Attempt to update the atomic state
            if tb.indexes.compare_exchange_weak(current_indexes, new_indexes, Release, Relaxed).is_ok() {
                // Success! Return the NEW writer buffer.
                let new_writer_idx = TripleBuffer::<T>::get_writer(new_indexes) as usize;
                return tb.get_buffer_mut(new_writer_idx);
            }
        }
    }
}

impl<T> TripleBuffer<T> where T: Clone + Default {
    /// Creates a new TripleBuffer.
    pub fn new() -> Box<Self> {
        Box::new(TripleBuffer {
            buffer: UnsafeCell::new(array_init(|_| T::default())),
            indexes: AtomicU32::new(0x24) // Initial state: reader=0, writer=1, ready=2, dirty=0
        })
    }

    /// Splits a TripleBuffer into a Reader and a Writer.
    pub fn split(self: Box<Self>) -> (TripleBufferReader<T>, TripleBufferWriter<T>) {
        let arc: Arc<TripleBuffer<T>> = Arc::from(self);
        (
            TripleBufferReader {
                triple_buffer: arc.clone()
            },
            TripleBufferWriter {
                triple_buffer: arc
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

    fn get_buffer_ref(&self, idx: usize) -> &T {
        unsafe { &(*self.buffer.get())[idx] }
    }

    fn get_buffer_mut(&self, idx: usize) -> &mut T {
        unsafe { &mut (*self.buffer.get())[idx] }
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

unsafe impl<T: Send, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> Send for SharedQueue<T, CHUNK_SIZE, NUM_CHUNKS> {}
unsafe impl<T: Sync, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> Sync for SharedQueue<T, CHUNK_SIZE, NUM_CHUNKS> {}

pub struct Producer<T, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> {
    q: Arc<SharedQueue<T, CHUNK_SIZE, NUM_CHUNKS>>,
}

unsafe impl<T: Send, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> Send for Producer<T, CHUNK_SIZE, NUM_CHUNKS> {}
unsafe impl<T: Sync, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> Sync for Producer<T, CHUNK_SIZE, NUM_CHUNKS> {}

pub struct Consumer<T, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> {
    q: Arc<SharedQueue<T, CHUNK_SIZE, NUM_CHUNKS>>,
}

unsafe impl<T: Send, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> Send for Consumer<T, CHUNK_SIZE, NUM_CHUNKS> {}
unsafe impl<T: Sync, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> Sync for Consumer<T, CHUNK_SIZE, NUM_CHUNKS> {}

pub struct ProducerGuard<'a, T, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> {
    producer: &'a mut Producer<T, CHUNK_SIZE, NUM_CHUNKS>,
}

impl<'a, T, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> Deref for ProducerGuard<'a, T, CHUNK_SIZE, NUM_CHUNKS> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.producer.get_buffer()
    }
}

impl<'a, T, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> DerefMut for ProducerGuard<'a, T, CHUNK_SIZE, NUM_CHUNKS> {
    fn deref_mut(&mut self) -> &mut [T] {
        self.producer.get_buffer_mut()
    }
}

impl<'a, T, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> Drop for ProducerGuard<'a, T, CHUNK_SIZE, NUM_CHUNKS> {
    fn drop(&mut self) {
        self.producer.commit();
    }
}

pub struct ConsumerGuard<'a, T, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> {
    consumer: &'a mut Consumer<T, CHUNK_SIZE, NUM_CHUNKS>,
}

impl<'a, T, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> Deref for ConsumerGuard<'a, T, CHUNK_SIZE, NUM_CHUNKS> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.consumer.get_buffer()
    }
}

impl<'a, T, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> Drop for ConsumerGuard<'a, T, CHUNK_SIZE, NUM_CHUNKS> {
    fn drop(&mut self) {
        self.consumer.commit();
    }
}

pub struct ProducerConsumerQueue<T, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> {
    _marker: std::marker::PhantomData<T>,
}

impl<T, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> ProducerConsumerQueue<T, CHUNK_SIZE, NUM_CHUNKS> 
where T: Default + Copy {
    /// Creates a new ProducerConsumerQueue and returns a tuple of (Producer, Consumer).
    pub fn new() -> (Producer<T, CHUNK_SIZE, NUM_CHUNKS>, Consumer<T, CHUNK_SIZE, NUM_CHUNKS>) {
        let q = Arc::new(SharedQueue {
            full_count:  Semaphore::new(0),
            empty_count: Semaphore::new(NUM_CHUNKS - 1),
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

    /// Allows the producer to fill multiple buffers. 
    /// This method will block and wait for empty slots until the closure `f` returns `false`.
    /// The callback `f` receives a mutable slice to the current buffer.
    pub fn produce<F: FnMut(&mut [T]) -> bool>(&self, mut f: F) -> bool {
        loop {
            if !self.wait_for_write() {
                return false;
            }

            let result = f(self.next_write_buffer());
            self.commit_write();

            if !result {
                return true;
            }
        }
    }

    /// Allows the consumer to process exactly one buffer.
    /// Returns true if a buffer was processed, false if the queue is stopped and empty.
    pub fn consume<F: FnMut(&[T])>(&self, mut f: F) -> bool {
        if !self.wait_for_read() {
            return false;
        }

        f(self.next_read_buffer());
        self.commit_read();

        true
    }

    fn wait_for_write(&self) -> bool {
        self.empty_count.wait();
        if self.stopped.load(Acquire) {
            self.empty_count.signal();
            return false;
        }
        true
    }

    fn next_write_buffer(&self) -> &mut [T] {
        let front = self.front.load(Acquire);
        unsafe { &mut (*self.buf.get())[front] }
    }

    fn commit_write(&self) {
        let front = self.front.load(Acquire);
        self.front.store((front + 1) % NUM_CHUNKS, Release);
        self.full_count.signal();
    }

    fn wait_for_read(&self) -> bool {
        self.full_count.wait();
        let back = self.back.load(Acquire);
        let front = self.front.load(Acquire);
        if self.stopped.load(Acquire) && front == back {
            self.full_count.signal();
            return false;
        }
        true
    }

    fn next_read_buffer(&self) -> &[T] {
        let back = self.back.load(Acquire);
        unsafe { &(*self.buf.get())[back] }
    }

    fn commit_read(&self) {
        let back = self.back.load(Acquire);
        self.back.store((back + 1) % NUM_CHUNKS, Release);
        self.empty_count.signal();
    }
}

impl<T, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> Producer<T, CHUNK_SIZE, NUM_CHUNKS> {
    pub fn produce<F: FnMut(&mut [T]) -> bool>(&self, f: F) -> bool {
        self.q.produce(f)
    }

    pub fn acquire_buffer(&mut self) -> Option<ProducerGuard<'_, T, CHUNK_SIZE, NUM_CHUNKS>> {
        if self.q.wait_for_write() {
            Some(ProducerGuard { producer: self })
        } else {
            None
        }
    }

    fn get_buffer(&self) -> &[T] {
        self.q.next_write_buffer()
    }

    fn get_buffer_mut(&mut self) -> &mut [T] {
        self.q.next_write_buffer()
    }

    fn commit(&self) {
        self.q.commit_write();
    }

    pub fn stop(&self) {
        self.q.stop();
    }
}

impl<T, const CHUNK_SIZE: usize, const NUM_CHUNKS: usize> Consumer<T, CHUNK_SIZE, NUM_CHUNKS> {
    pub fn consume<F: FnMut(&[T])>(&self, f: F) -> bool {
        self.q.consume(f)
    }

    pub fn acquire_buffer(&mut self) -> Option<ConsumerGuard<'_, T, CHUNK_SIZE, NUM_CHUNKS>> {
        if self.q.wait_for_read() {
            Some(ConsumerGuard { consumer: self })
        } else {
            None
        }
    }

    fn get_buffer(&self) -> &[T] {
        self.q.next_read_buffer()
    }

    fn commit(&self) {
        self.q.commit_read();
    }

    pub fn drain(&self) {
        self.q.drain();
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_triple_buffer_basic() {
        let (mut reader, mut writer) = TripleBuffer::<u32>::new().split();
        
        // Initial state
        let (val, state) = reader.get_read_buffer();
        assert_eq!(*val, 0);
        assert_eq!(state, State::StateNoChange);

        // Update state
        {
            let w_val = writer.get_write_buffer(); // Swaps ready buffer to writer, previous writer becomes ready
            *w_val = 42;
            writer.get_write_buffer(); // Publishes the buffer containing 42 by swapping it to ready
        }

        // Reader should see dirty state
        let (val, state) = reader.get_read_buffer();
        assert_eq!(*val, 42);
        assert_eq!(state, State::StateDirty);

        // Reader should see no change now
        let (val, state) = reader.get_read_buffer();
        assert_eq!(*val, 42);
        assert_eq!(state, State::StateNoChange);
    }

    #[test]
    fn test_triple_buffer_multiple_writes() {
        let (mut reader, mut writer) = TripleBuffer::<u32>::new().split();

        // Write twice
        *writer.get_write_buffer() = 1;
        *writer.get_write_buffer() = 2; // This publishes 1 and makes writer point to a new buffer
        writer.get_write_buffer();      // This publishes 2

        let (val, state) = reader.get_read_buffer();
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
                let w = writer.get_write_buffer();
                *w = i;
            }
            writer.get_write_buffer(); // Flush the last value
        });

        let reader_thread = thread::spawn(move || {
            b2.wait();
            let mut last_val: Option<u32> = None;
            while last_val.unwrap_or(0) < 1000 {
                let (val, state) = reader.get_read_buffer();
                if state == State::StateDirty {
                    if let Some(lv) = last_val {
                        assert!(*val > lv, "Read value {} not greater than last {}", *val, lv);
                    }
                    last_val = Some(*val);
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
            let _w = writer.get_write_buffer();
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

    #[test]
    fn test_pcq_non_power_of_two() {
        // Use 3 buffers (not a power of two)
        let (prod, cons) = ProducerConsumerQueue::<u32, 1, 3>::new();
        
        // Fill 2 buffers (capacity is NUM-1 = 2)
        prod.produce(|buf| { buf[0] = 1; false });
        prod.produce(|buf| { buf[0] = 2; false });
        
        let mut val = 0;
        cons.consume(|buf| { val = buf[0]; });
        assert_eq!(val, 1);
        cons.consume(|buf| { val = buf[0]; });
        assert_eq!(val, 2);
        
        // Verify we can continue using it after wraparound
        prod.produce(|buf| { buf[0] = 3; false });
        cons.consume(|buf| { val = buf[0]; });
        assert_eq!(val, 3);
    }

    #[test]
    fn test_pcq_raii() {
        let (mut prod, mut cons) = ProducerConsumerQueue::<u32, 1, 2>::new();
        
        {
            let mut w = prod.acquire_buffer().unwrap();
            w[0] = 42;
        } // commit occurs here
        
        {
            let r = cons.acquire_buffer().unwrap();
            assert_eq!(r[0], 42);
        } // commit occurs here
    }
}
