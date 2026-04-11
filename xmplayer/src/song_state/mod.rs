#[macro_use]

mod leak;

use core::sync::atomic::{AtomicBool, Ordering};
use crate::song::{PlayData, Song, PlaybackCmd, CallbackState};
use crate::module_reader::{SongData, read_module};
use shared_sync_primitives::{ProducerConsumerQueue};
use std::sync::{mpsc, Mutex, Arc};
use core::option::Option::None;
use core::option::Option;
use std::thread::{spawn, sleep, JoinHandle};
use core::time::Duration;
use crate::song::PlaybackCmd::Quit;
use shared_sync_primitives::{TripleBufferReader, TripleBuffer, State::StateNoChange};
use std::sync::mpsc::{Sender, Receiver};
use std::ops::{DerefMut};
use crate::instrument::Instrument;
use crate::{SimpleResult};
use crate::song::InterleavedBufferAdaptar;
use crate::{AUDIO_BUF_SIZE, NUM_AUDIO_CHUNKS, AudioConsumer, AudioProducer};

use std::cell::UnsafeCell;

// We implement Send and Sync to mimic the previous AtomicPtr behavior.
// Users must ensure actual usage avoids data races.
unsafe impl<T: Send> Send for StructHolder<T> {}
unsafe impl<T: Sync> Sync for StructHolder<T> {}

pub(crate) struct StructHolder<T> {
    t: Arc<UnsafeCell<T>>,
}

impl<T> Clone for StructHolder<T> {
    fn clone(&self) -> Self {
        Self { t: self.t.clone() }
    }
}

impl <T> StructHolder<T> {
    pub(crate) fn new(arg: Box<T>) -> Self {
        Self { t: Arc::new(UnsafeCell::new(*arg)) }
    }

    pub(crate) fn get(&self) -> &T {
        unsafe { &*self.t.get() }
    }
}

#[derive(Clone)]
pub struct SongHandle(pub(crate) StructHolder<SongState>);

impl std::ops::Deref for SongHandle {
    type Target = SongState;
    fn deref(&self) -> &Self::Target {
        self.0.get()
    }
}

pub struct SongState {
    pub stopped:                        Arc<AtomicBool>,
    triple_buffer_reader:               Arc<Mutex<TripleBufferReader<PlayData>>>,
    pub song_data:                      SongData,
    pub song:                           Arc<Mutex<Song>>,
    tx:                                 Sender<PlaybackCmd>,
    rx:                                 Arc<Mutex<Receiver<PlaybackCmd>>>,
    q:                                  AudioProducer,
    display_cb:                         Mutex<Option<fn (&PlayData, &Vec<Instrument>)>>,
}

impl SongState {

    pub fn new(path: &str) -> SimpleResult<(SongHandle, AudioConsumer)> {
        let song_data = read_module(path)?;

        let triple_buffer = TripleBuffer::<PlayData>::new();
        let (triple_buffer_reader, triple_buffer_writer) = triple_buffer.split();
        let song = Arc::new(Mutex::new(Song::new(&song_data, triple_buffer_writer, 48000.0)));
        let (tx, rx): (Sender<PlaybackCmd>, Receiver<PlaybackCmd>) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::from(false));

        let (producer, consumer) = ProducerConsumerQueue::<f32, AUDIO_BUF_SIZE, NUM_AUDIO_CHUNKS>::new();

        let sh = SongHandle(StructHolder::new( Box::new( Self {
            stopped,
            triple_buffer_reader: Arc::new(Mutex::new(triple_buffer_reader)),
            song_data,
            song,
            tx,
            rx: Arc::new(Mutex::new(rx)),
            q: producer,
            display_cb: Mutex::new(None),
        })));

        Ok((sh, consumer))
    }

    pub fn set_order(&self, order: u32) {
        if let Ok(_) = self.tx.send(PlaybackCmd::SetPosition(order)) {}
    }

    fn callback(&self) {
        let mut song = self.song.lock().unwrap();
        let mut rx = self.rx.lock().unwrap();
        while let Some(mut buf) = self.q.acquire_buffer() {
            let mut adaptar = InterleavedBufferAdaptar{buf: &mut *buf};
            if let CallbackState::Complete = song.get_next_tick(&mut adaptar, rx.deref_mut()) { break; }
        }
        self.stopped.store(true, Ordering::Release);
        self.q.stop();
    }

    // fn callback_planar(&mut self) {
    //     let mut song = self.song.lock().unwrap();
    //     let mut rx = self.rx.lock().unwrap();
    //     self.q.get().produce(|buf: &mut [f32]| -> bool {
    //         let adaptar = PlanarBufferAdaptar::new(buf);
    //         if let CallbackState::Complete = song.get_next_tick(adaptar, rx.deref_mut()) { return false; }
    //         true
    //     });
    //     self.stopped.store(true, Ordering::Release);
    // }


    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
}


impl SongHandle {
    pub fn start(&self, display_cb: fn (&PlayData, &Vec<Instrument>)) -> (Option<JoinHandle<()>>, Option<JoinHandle<()>>) {
        {
            let mut cb = self.display_cb.lock().unwrap();
            *cb = Option::from(display_cb);
        }

        let s1 = self.clone();
        let play_thread = Option::from(spawn(move || s1.callback()));

        let s2 = self.clone();
        let display_thread = Option::from(spawn(move || {
            let s = &s2;
            let tb_guard = s.triple_buffer_reader.clone();
            let mut triple_buffer_reader = tb_guard.lock().unwrap();

            let mut song_row = 0;
            let mut song_tick = 2000;
            loop {
                if s.is_stopped() {
                    break;
                }
                sleep(Duration::from_millis(30));
                let (play_data, state) = triple_buffer_reader.get_read_buffer();
                if StateNoChange == state { continue; }
                if play_data.tick != song_tick || play_data.row != song_row {
                    let cb_guard = s.display_cb.lock().unwrap();
                    if let Some(cb) = *cb_guard {
                        (cb)(play_data, &s.song_data.instruments);
                    }
                    song_row = play_data.row;
                    song_tick = play_data.tick;
                }
            }
        }));
        (play_thread, display_thread)
    }
}

impl SongState {

    pub fn get_sender(&self) -> Sender<PlaybackCmd> {
        return self.tx.clone();
    }

    pub fn get_triple_buffer_reader(&self) -> Arc<Mutex<TripleBufferReader<PlayData>>> {
        return self.triple_buffer_reader.clone();
    }

    pub fn close(&self) {
        self.stopped.store(true, Ordering::Release);
        let _ = self.tx.send(Quit);
        self.q.stop();
        // if handle.0.is_some() {
        //     handle.0.unwrap().join().unwrap();
        // }
        // if handle.1.is_some() {
        //     handle.1.unwrap().join().unwrap();
        // }
    }
}

// pub struct SongHandleLockGuard<'a>{
//     song_state: &'a mut SongState,
//     mutex_guard: MutexGuard<'a, u32>,
//     _nosend: PhantomData<*mut ()>
// }
//
// impl<'a> Deref for SongHandleLockGuard<'a> {
//     type Target = SongState;
//     fn deref(&self) -> &SongState { (*self.song_state).as_ref() }
// }
//
// impl<'a> DerefMut for SongHandleLockGuard<'a> {
//     fn deref_mut(&mut self) -> &mut SongState { (*self.song_state).as_mut() }
// }
//
// impl<'a> Drop for SongHandleLockGuard<'a> {
//     fn drop(&mut self) {
//         mem::drop(self.mutex_guard);
//     }
// }
//
// #[derive(Clone)]
// pub struct SongHandle {
//     song_state: *mut c_void,
//     mutex: Mutex<u32>,
// }
//
// impl SongHandle {
//     pub fn new(path: String) -> Self {
//         Self { song_state: leak!(SongState::new(path)), mutex: Mutex::new(0) }
//     }
//
//     pub fn lock(&mut self) -> SongHandleLockGuard {
//         let guard = self.mutex.lock().unwrap();
//         SongHandleLockGuard{ song_state: self.song_state as &mut SongState, mutex_guard: guard, _nosend: Default::default() }
//     }
// }

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct DropTracker(Arc<AtomicUsize>);
    impl Drop for DropTracker {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct LeakTestState {
        _tracker: DropTracker,
    }

    #[test]
    fn test_song_handle_drop() {
        let counter = Arc::new(AtomicUsize::new(0));
        {
            let tracker = DropTracker(counter.clone());
            let _sh = StructHolder::new(Box::new(LeakTestState { _tracker: tracker }));
            assert_eq!(counter.load(Ordering::SeqCst), 0);
        }
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_song_state_leak() {
        // We use a dummy file path or a mocked file loader once we have one, 
        // but for now, we just test the StructHolder drop logic with SongState.
        let counter = Arc::new(AtomicUsize::new(0));
        
        let triple_buffer = TripleBuffer::<PlayData>::new();
        let (triple_buffer_reader, triple_buffer_writer) = triple_buffer.split();
        let song_data = SongData::default();
        let song = Arc::new(Mutex::new(Song::new(&song_data, triple_buffer_writer, 48000.0)));
        let (tx, rx) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::from(false));
        let (producer, _consumer) = ProducerConsumerQueue::<f32, AUDIO_BUF_SIZE, NUM_AUDIO_CHUNKS>::new();

        struct SongStateWithTracker {
            _ss: SongState,
            _tracker: DropTracker,
        }

        {
            let tracker = DropTracker(counter.clone());
            let ss = SongState {
                stopped,
                triple_buffer_reader: Arc::new(Mutex::new(triple_buffer_reader)),
                song_data,
                song,
                tx,
                rx: Arc::new(Mutex::new(rx)),
                q: producer,
                display_cb: Mutex::new(None),
            };
            let _sh = StructHolder::new(Box::new(SongStateWithTracker { _ss: ss, _tracker: tracker }));
            assert_eq!(counter.load(Ordering::SeqCst), 0);
        }
        
        // If there is no cycle, counter should be 1
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}