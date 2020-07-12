use std::sync::{Arc, Condvar, Mutex};
use std::sync::atomic::{AtomicPtr, Ordering, AtomicBool};
use std::sync::atomic::Ordering::{Acquire, Release};

pub const AUDIO_BUF_FRAMES: usize = 1024;
pub const AUDIO_BUF_SIZE: usize = AUDIO_BUF_FRAMES *2;
pub const AUDIO_NUM_BUFFERS: usize = 3;

struct Semaphore {
    condvar: Arc<(Mutex<usize>, Condvar)>,
}

impl Semaphore {
    fn new(initial: usize) -> Semaphore {
        return Semaphore {
            condvar: Arc::new((Mutex::new(initial), Condvar::new())),
        };
    }

    fn signal(&mut self) {
        let (lock, cvar) = &*self.condvar;
        let mut count = lock.lock().unwrap();
        *count += 1;
        cvar.notify_one();
    }

    fn wait(&mut self) {
        let (lock, cvar) = &*self.condvar;
        let mut count = lock.lock().unwrap();
        while *count == 0 {
            count = cvar.wait(count).unwrap();
        }
        *count -= 1;
    }

}

pub struct ProducerConsumerQueue {
    full_count:         Semaphore,
    empty_count:        Semaphore,
    buf:                [[f32; AUDIO_BUF_SIZE]; AUDIO_NUM_BUFFERS],
    front:              usize,
    back:               usize,
    stopped:            AtomicBool,
}

#[derive(Clone)]
pub struct PCQHolder {
    q: Arc<AtomicPtr<ProducerConsumerQueue>>,
}

impl PCQHolder {
    pub fn get(&mut self) -> &mut ProducerConsumerQueue {
        unsafe{&mut *self.q.load(Ordering::Acquire)}
    }
}


impl ProducerConsumerQueue {
    pub fn new() -> PCQHolder {
        let q = Box::new(ProducerConsumerQueue {
            full_count: Semaphore::new(0),
            empty_count: Semaphore::new(AUDIO_NUM_BUFFERS - 1),
            // consumer: Arc::new((Mutex::new(false), Default::default())),
            buf: [[0.0f32; AUDIO_BUF_SIZE]; AUDIO_NUM_BUFFERS],
            front: 0,
            back: 0,
            stopped: AtomicBool::from(false),
        });
        PCQHolder{q: Arc::new(AtomicPtr::new(Box::into_raw(q) as *mut ProducerConsumerQueue))}
    }

    pub fn produce<F: FnMut(&mut[f32; AUDIO_BUF_SIZE]) -> bool>(&mut self, mut f: F) -> bool {
        loop {
            self.empty_count.wait();
            let my_buf = &mut self.buf[self.front];
            self.front = (self.front + 1) % AUDIO_NUM_BUFFERS;
            if !f(my_buf) { self.stopped.store(true, Release);self.full_count.signal(); return false; }
            self.full_count.signal()
        }
    }

    pub fn consume<F: FnMut(&[f32; AUDIO_BUF_SIZE])>(&mut self, mut f: F) -> bool {
        self.full_count.wait();
        if self.stopped.load(Acquire) == true {
            return false;
        }

        let my_buf = &self.buf[self.back];
        self.back = (self.back + 1) % AUDIO_NUM_BUFFERS;
        f(my_buf);
        self.empty_count.signal();
        true
    }
}
