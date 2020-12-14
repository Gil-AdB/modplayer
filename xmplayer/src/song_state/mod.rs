#[macro_use]

mod leak;

use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use crate::song::{PlayData, Song, PlaybackCmd, CallbackState};
use crate::module_reader::{SongData, read_module};
use crate::producer_consumer_queue::{PCQHolder, ProducerConsumerQueue};
use std::sync::{mpsc, Mutex, Arc, MutexGuard};
use core::option::Option::None;
use core::option::Option;
use std::thread::{spawn, sleep, JoinHandle};
use core::time::Duration;
use crate::triple_buffer::State::StateNoChange;
use crate::song::PlaybackCmd::Quit;
use crate::triple_buffer::{TripleBufferReader, TripleBuffer};
use std::sync::mpsc::{Sender, Receiver};
use std::ops::{DerefMut, Deref};
use crate::instrument::Instrument;
use simple_error::{SimpleResult};

#[derive(Clone)]
pub struct StructHolder<T> {
    t: Arc<AtomicPtr<T>>,
}

impl <T> StructHolder<T> {
    pub fn new(arg: Box<T>) -> Self {
        Self { t: Arc::new(AtomicPtr::new(Box::into_raw(arg))) }
    }

    pub fn get_mut(&mut self) -> &mut T {
        unsafe { &mut *self.t.load(Ordering::Acquire) }
    }

    pub fn get(&self) -> &T {
        unsafe { &*self.t.load(Ordering::Acquire) }
    }
}

#[derive(Clone)]
pub struct SongState {
    pub stopped:                        Arc<AtomicBool>,
    triple_buffer_reader:               Arc<Mutex<TripleBufferReader<PlayData>>>,
    pub song_data:                      SongData,
    pub song:                           Arc<Mutex<Song>>,
    tx:                                 Sender<PlaybackCmd>,
    rx:                                 Arc<Mutex<Receiver<PlaybackCmd>>>,
    q:                                  PCQHolder,
    display_cb:                         Option<fn (&PlayData, &Vec<Instrument>)>,

    self_ref:                           Option<StructHolder<SongState>>,

}

pub type SongHandle = StructHolder<SongState>;

impl SongState {

    pub fn new(path: String) -> SimpleResult<SongHandle> {
        let song_data = read_module(path.as_str())?;

        let triple_buffer = TripleBuffer::<PlayData>::new();
        let (triple_buffer_reader, triple_buffer_writer) = triple_buffer.split();
        let song = Arc::new(Mutex::new(Song::new(&song_data, triple_buffer_writer, 48000.0)));
        let (tx, rx): (Sender<PlaybackCmd>, Receiver<PlaybackCmd>) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::from(false));

        let mut sh = StructHolder::new( Box::new( Self {
            stopped,
            triple_buffer_reader: Arc::new(Mutex::new(triple_buffer_reader)),
            song_data,
            song,
            tx,
            rx: Arc::new(Mutex::new(rx)),
            q: ProducerConsumerQueue::new(),
            display_cb: None,
            self_ref: None
        }));

        sh.get_mut().self_ref = Option::from(sh.clone());
        Ok(sh)
    }

    pub fn set_order(&mut self, order: u32) {
        if let Ok(_) = self.tx.send(PlaybackCmd::SetPosition(order)) {}
    }

    fn callback(&mut self) {
        let mut song = self.song.lock().unwrap();
        let mut rx = self.rx.lock().unwrap();
        self.q.get().produce(|buf: &mut [f32]| -> bool {
            if let CallbackState::Complete = song.get_next_tick(buf, rx.deref_mut()) { return false; }
            true
        });
        self.stopped.store(true, Ordering::Release);
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    fn clone(&mut self) -> SongHandle {
        self.self_ref.as_mut().unwrap().clone()
    }

    pub fn start(&mut self, display_cb: fn (&PlayData, &Vec<Instrument>)) -> (Option<JoinHandle<()>>, Option<JoinHandle<()>>) {

        self.display_cb = Option::from(display_cb);

        let mut s1 = self.clone();
        let play_thread = Option::from(spawn(move || Self::callback(s1.get_mut())));
        let mut display_thread: Option<JoinHandle<()>> = None;

        let mut s2 = self.clone();

        if self.display_cb.is_some() {
            display_thread = Option::from(spawn(move || {
                let s = s2.get_mut();
                let tb_guard = s.triple_buffer_reader.clone();
                let mut triple_buffer_reader = tb_guard.lock().unwrap().get();
                //         let mut triple_buffer_reader = triple_buffer_reader.lock().unwrap();

                let mut song_row = 0;
                let mut song_tick = 2000;
                loop {
                    if s.is_stopped() {
                        break;
                    }
                    sleep(Duration::from_millis(30));
                    let (play_data, state) = triple_buffer_reader.read();
                    if StateNoChange == state { continue; }
                    if play_data.tick != song_tick || play_data.row != song_row {
                        (s.display_cb.unwrap())(play_data, &s.song_data.instruments);
                        song_row = play_data.row;
                        song_tick = play_data.tick;
                    }
                }
            }));
        }
        (play_thread, display_thread)
    }

    pub fn get_queue(&mut self) -> PCQHolder {
        return self.q.clone();
    }

    pub fn get_sender(&mut self) -> &mut Sender<PlaybackCmd> {
        return &mut self.tx;
    }

    pub fn get_triple_buffer_reader(&self) -> Arc<Mutex<TripleBufferReader<PlayData>>> {
        return self.triple_buffer_reader.clone();
    }

    pub fn close(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.tx.send(Quit).unwrap();
        self.q.get().quit();
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