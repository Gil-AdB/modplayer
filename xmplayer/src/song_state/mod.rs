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

pub(crate) struct StructHolder<T> {
    t: Arc<T>,
}

impl<T> Clone for StructHolder<T> {
    fn clone(&self) -> Self {
        Self { t: self.t.clone() }
    }
}

// We implement Send and Sync to mimic the previous AtomicPtr behavior.
// Users must ensure actual usage avoids data races.
unsafe impl<T: Send> Send for StructHolder<T> {}
unsafe impl<T: Sync> Sync for StructHolder<T> {}

impl <T> StructHolder<T> {
    pub(crate) fn new(arg: Box<T>) -> Self {
        Self { t: Arc::from(arg) }
    }

    pub(crate) fn get(&self) -> &T {
        &self.t
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
    pub(crate) stopped:              Arc<AtomicBool>,
    pub(crate) triple_buffer_reader: TripleBufferReader<PlayData>,
    pub(crate) song_data:            SongData,
    pub(crate) song:                 Arc<Mutex<Song>>,
    pub(crate) tx:                   Sender<PlaybackCmd>,
    pub(crate) rx:                   Arc<Mutex<Receiver<PlaybackCmd>>>,
    pub(crate) q:                    AudioProducer,
    pub(crate) display_cb:           Mutex<Option<fn (&PlayData, &Vec<Instrument>, &Vec<crate::module_reader::Patterns>, &Vec<u8>)>>,
}

impl SongState {

    pub fn new(path: &str) -> SimpleResult<(SongHandle, AudioConsumer)> {
        let song_data = read_module(path)?;

        let triple_buffer = TripleBuffer::<PlayData>::new_with_signal();
        let (triple_buffer_reader, triple_buffer_writer) = triple_buffer.split();
        let song = Arc::new(Mutex::new(Song::new(&song_data, triple_buffer_writer, 48000.0)));
        let (tx, rx): (Sender<PlaybackCmd>, Receiver<PlaybackCmd>) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::from(false));

        let (producer, consumer) = ProducerConsumerQueue::<f32, AUDIO_BUF_SIZE, NUM_AUDIO_CHUNKS>::new();

        let sh = SongHandle(StructHolder::new( Box::new( Self {
            stopped,
            triple_buffer_reader,
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
        loop {
            if !song.handle_commands(rx.deref_mut()) { break; }
            if self.is_stopped() { break; }

            if let Some(mut buf) = self.q.try_acquire_buffer() {
                let mut adaptar = InterleavedBufferAdaptar{buf: &mut *buf};
                if let CallbackState::Complete = song.get_next_tick(&mut adaptar, rx.deref_mut()) { break; }
            } else {
                // If we couldn't acquire a buffer, sleep a bit to avoid busy waiting
                sleep(Duration::from_millis(10));
            }
        }
        self.stopped.store(true, Ordering::Release);
        self.triple_buffer_reader.wake_reader();
        self.q.stop();
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
}


impl SongHandle {
    pub fn start(&self, display_cb: fn (&PlayData, &Vec<Instrument>, &Vec<crate::module_reader::Patterns>, &Vec<u8>)) -> (Option<JoinHandle<()>>, Option<JoinHandle<()>>) {
        {
            let mut cb = self.display_cb.lock().unwrap();
            *cb = Option::from(display_cb);
        }

        let s1 = self.clone();
        let play_thread = Option::from(spawn(move || s1.callback()));

        let s2 = self.clone();
        let display_thread = Option::from(spawn(move || {
            let s = &s2;
            loop {
                if s.is_stopped() {
                    break;
                }
                s.triple_buffer_reader.wait();
                let (play_data, state) = s.triple_buffer_reader.get_read_buffer();
                if StateNoChange == state { continue; }
                let cb_guard = s.display_cb.lock().unwrap();
                if let Some(cb) = *cb_guard {
                    (cb)(play_data, &s.song_data.instruments, &s.song_data.patterns, &s.song_data.pattern_order);
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

    #[cfg(test)]
    pub fn get_song_data(&self) -> &SongData {
        &self.song_data
    }

    /// Returns the underlying Song struct. Only intended for testing purposes.
    pub fn get_song(&self) -> &Arc<Mutex<Song>> {
        &self.song
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.triple_buffer_reader.wake_reader();
    }

    pub fn close(&self) {
        self.stopped.store(true, Ordering::Release);
        let _ = self.tx.send(Quit);
        self.q.stop();
        self.triple_buffer_reader.wake_reader();
    }
}

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
        
        let triple_buffer = TripleBuffer::<PlayData>::new_with_signal();
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
                triple_buffer_reader,
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