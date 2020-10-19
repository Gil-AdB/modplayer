
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::{Arc, mpsc, Mutex};
use std::sync::atomic::{AtomicPtr, Ordering, AtomicBool};
use std::sync::mpsc::{Receiver, Sender};

use crossbeam::thread;
use getch::Getch;

use xmplayer::song::{Song, PlaybackCmd, PlayData, CallbackState};
use xmplayer::module_reader::{SongData, read_module, print_module};
use std::env;
use std::time::{Duration, SystemTime};
use std::thread::{sleep, spawn, JoinHandle};
use std::io::{stdout, Write};
use xmplayer::triple_buffer::{TripleBuffer, TripleBufferReader};
use xmplayer::triple_buffer::State::StateNoChange;

use crossterm::terminal::ClearType::All;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use std::borrow::BorrowMut;
use std::marker::PhantomData;
use std::rc::Rc;
use crossterm::event::{Event, KeyEvent, KeyCode};
use crossterm::event::Event::Key;
use xmplayer::song::PlaybackCmd::Quit;
use xmplayer::song_state::{SongState, SongHandle};
use std::error::Error;

#[cfg(feature="sdl2-feature")] mod sdl2_audio;
#[cfg(feature="sdl2-feature")] use sdl2_audio::AudioOutput;
#[cfg(feature="portaudio-feature")] mod portaudio_audio;
#[cfg(feature="portaudio-feature")] use portaudio_audio::AudioOutput;
use xmplayer::instrument::Instrument;
use crossterm::cursor::MoveToNextLine;
use crossterm::terminal::{Clear, ClearType};
use display::display::Display;
use display::ViewPort;

fn main() {
    if env::args().len() < 2 {return;}

	dbg!(env::args());

    let path = env::args().nth(1).unwrap();
    //let file = File::open(path).expect("failed to open the file");

   // let data = read_module(path.as_str()).unwrap();

    let mut song = SongState::new(path);
    if env::args().len() > 2 {
        print_module(&song, env::args().skip(2));
    } else {
        run(&mut song);
    }
}

struct TerminalModeSetter {
}

impl TerminalModeSetter {
    fn new() -> Self {
        if let Err(_e) = crossterm::execute!(stdout(), EnterAlternateScreen) {}
        crossterm::terminal::enable_raw_mode();
        TerminalModeSetter {}
    }
}

impl Drop for TerminalModeSetter {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}


fn run(song_data: &mut SongHandle) {
    const CHANNELS: i32 = 2;
    const SAMPLE_RATE: f32 = 48_000.0;

    let _mode_setter = TerminalModeSetter::new();

    let mut audio = AudioOutput::new(song_data, SAMPLE_RATE);

    let handle = song_data.get_mut().start(|data, instruments| {

        let mut view_port = ViewPort {
            x1: 0,
            y1: 0,
            x2: 1,
            y2: 1
        };

        if let Ok(size) = crossterm::terminal::size() {view_port.x2 = (size.0 + 1) as usize; view_port.y2 = (size.1 + 1) as usize; }

        dbg!(&view_port);
        Display::display(data, instruments, view_port, &mut|str| {
            write!(stdout(), "{}", str);
            if let Err(_e) = crossterm::execute!(stdout(), Clear(ClearType::UntilNewLine)) {}
            if let Err(_e) = crossterm::execute!(stdout(), MoveToNextLine(1)) {}
        });
    });

    audio.start_audio_output();
    mainloop(song_data.get_mut());

    song_data.get_mut().close(handle);
    audio.close();
    if let Err(_e) = crossterm::execute!(stdout(), LeaveAlternateScreen) {}
}

fn is_num (ch: char) -> bool {
    ch >= '0' && ch <= '9'
}


fn mainloop(song_data: &mut SongState) -> std::result::Result<bool, crossterm::ErrorKind> {
    let mut last_time = SystemTime::now();
    let mut last_char= '\0';

    loop {
        if song_data.is_stopped() {return Ok(true);}

        // let input = tokio::time::timeout(Duration::from_secs(1), getter.getch()).await;
        let input;
        if let Err(_e) = crossterm::terminal::enable_raw_mode() {}
        if crossterm::event::poll(Duration::from_millis(100)).is_ok() {
            // It's guaranteed that the `read()` won't block when the `poll()`
            // function returns `true`
            match crossterm::event::read() {
                Ok(crossterm::event::Event::Key(event)) => input = event,
                _ => {
                    continue;
                }
            }
        } else {
            continue;
        }

        if SystemTime::now() > last_time + Duration::from_secs(1) {
            last_char = '\0';
        }

        if let KeyCode::Esc = input.code {
            let tx = song_data.get_sender();
            let _ = tx.send(PlaybackCmd::Quit);
            break;
        }

        if let KeyCode::Char(ch) = input.code {
            let tx = &mut song_data.get_sender();
            if ch == 'q' {
                let _ = tx.send(PlaybackCmd::Quit);
                break;
            }
            if is_num(ch) {
                if is_num(last_char) {
                    let channel_number = (last_char as u8 - '0' as u8)  * 10 + (ch as u8 - '0' as u8);
                    if channel_number > 0  && channel_number <= 32 {
                        let _ = tx.send(PlaybackCmd::ChannelToggle(channel_number - 1));
                    }
                    last_char = '\0';
                } else {
                    last_char = ch;
                }
            }
            if ch == '+' {
                let _ = tx.send(PlaybackCmd::IncSpeed);
            }
            if ch == '-' {
                let _ = tx.send(PlaybackCmd::DecSpeed);
            }
            if ch == '.' {
                let _ = tx.send(PlaybackCmd::IncBPM);
            }
            if ch == ',' {
                let _ = tx.send(PlaybackCmd::DecBPM);
            }
            if ch == ' ' {
                let _ = tx.send(PlaybackCmd::PauseToggle);
            }
            if ch == 'n' {
                let _ = tx.send(PlaybackCmd::Next);
            }
            if ch == '/' {
                let _ = tx.send(PlaybackCmd::LoopPattern);
            }
            if ch == 'p' {
                let _ = tx.send(PlaybackCmd::Prev);
            }
            if ch == 'r' {
                let _ = tx.send(PlaybackCmd::Restart);
            }
            if ch == 'a' {
                let _ = tx.send(PlaybackCmd::AmigaTable);
            }
            if ch == 'l' {
                let _ = tx.send(PlaybackCmd::LinearTable);
            }
            if ch == 'f' {
                let _ = tx.send(PlaybackCmd::FilterToggle);
            }
            if ch == 'd' {
                let _ = tx.send(PlaybackCmd::DisplayToggle);
            }
        }
        last_time = SystemTime::now();
    }
    Ok(true)
}
