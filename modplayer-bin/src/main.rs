use xmplayer::song::{PlaybackCmd, UserData};
use xmplayer::module_reader::print_module;
use std::env;
use std::time::{Duration, SystemTime};
use std::io::{stdout, Write};

use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::event::{KeyCode, KeyModifiers};
use xmplayer::song_state::{SongState, SongHandle};
use xmplayer::AudioConsumer;

#[cfg(feature="sdl2-feature")] mod sdl2_audio;
#[cfg(feature="sdl2-feature")] use sdl2_audio::AudioOutput;
#[cfg(feature="portaudio-feature")] mod portaudio_audio;
#[cfg(feature="portaudio-feature")] use portaudio_audio::AudioOutput;
use crossterm::cursor::{Hide, MoveTo, Show};
use display::display::{Display, TargetPlatform};
use display::{ViewPort, Grid};

fn main() {
    if env::args().len() < 2 {return;}

	let _ = dbg!(env::args());

    let path = env::args().nth(1).unwrap();
    //let file = File::open(path).expect("failed to open the file");

   // let data = read_module(path.as_str()).unwrap();

    let (mut song, consumer) = match SongState::new(&path) {
        Ok(s) => {s}
        Err(e) => {dbg!(e);return;}
    };

    if env::args().any(|arg| arg == "--dump") {
        run_dump(&mut song, consumer);
    } else if env::args().len() > 2 {
        print_module(&song, env::args().skip(2));
    } else {
        run(&mut song, consumer);
    }
}

fn run_dump(song_data: &mut SongHandle, mut consumer: AudioConsumer) {
    println!("Starting dump of {}...", song_data.get_song_data().name);
    let _handle = song_data.start(|_, _, _, _| {});

    let mut last_tick = 999;
    let mut last_row = 999;
    let mut last_pos = 999;

    loop {
        if song_data.is_stopped() { break; }
        
        consumer.drain();

        let state = song_data.get_state();
        if state.tick != last_tick || state.row != last_row || state.song_position != last_pos {
            println!("{}", state.dump.to_string());
            let _ = std::io::stdout().flush();
            last_tick = state.tick;
            last_row = state.row;
            last_pos = state.song_position;
        }

        if state.song_position >= song_data.get_song_data().song_length as usize {
            println!("Reached end of song.");
            break;
        }

        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

struct TerminalModeSetter {
}

impl TerminalModeSetter {
    fn new() -> Self {
        // Install panic hook to restore terminal on crash
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = crossterm::execute!(stdout(), Show, LeaveAlternateScreen);
            default_hook(info);
        }));

        if let Err(_e) = crossterm::execute!(stdout(), EnterAlternateScreen) {}
        let _ = crossterm::terminal::enable_raw_mode();
        TerminalModeSetter {}
    }
}

impl Drop for TerminalModeSetter {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(stdout(), Show, LeaveAlternateScreen);
    }
}


fn run(song_data: &mut SongHandle, consumer: AudioConsumer) {
    const _CHANNELS: i32 = 2;
    const SAMPLE_RATE: f32 = 48_000.0;

    let _mode_setter = TerminalModeSetter::new();

    let mut audio = AudioOutput::new(consumer, SAMPLE_RATE);

    let handle = song_data.start(|data, instruments, patterns, order| {

        let mut view_port = ViewPort {
            x1: 0,
            y1: 0,
            width: 120, // Increased width for full view
            height: 40
        };

        if let UserData::ISize(x) = data.user_data.get("x").unwrap_or(&UserData::ISize(0)) {
            if let UserData::ISize(y) = data.user_data.get("y").unwrap_or(&UserData::ISize(0)) {
                if let UserData::USize(height) = data.user_data.get("height").unwrap_or(&UserData::USize(0)) {
                    if let UserData::USize(width) = data.user_data.get("width").unwrap_or(&UserData::USize(0)) {
                        view_port.x1 = *x;
                        view_port.y1 = *y;
                        view_port.width = *width;
                        view_port.height = *height;
                    }
                }
            }
        }

        let mut grid = Grid::new(view_port.width, view_port.height);
        Display::render(&mut grid, data, instruments, patterns, order, view_port.width, view_port.height, data.view_mode, data.theme_id, view_port.x1, view_port.y1, TargetPlatform::Native);
        
        if let Err(_e) = crossterm::execute!(stdout(), Hide, MoveTo(0, 0)) {}
        print!("{}", grid.to_ansi());
        let _ = stdout().flush();
        if let Err(_e) = crossterm::execute!(stdout(), Show) {}
    });

    audio.start_audio_output();
    mainloop(song_data);

    song_data.close();
    if handle.0.is_some() {
        handle.0.unwrap().join().unwrap();
    }
    if handle.1.is_some() {
        handle.1.unwrap().join().unwrap();
    }

    audio.close();
    if let Err(_e) = crossterm::execute!(stdout(), LeaveAlternateScreen) {}
}

fn is_num (ch: char) -> bool {
    ch >= '0' && ch <= '9'
}


fn mainloop(song_data: &SongState) {

    if let Ok(size) = crossterm::terminal::size() {
        let tx = song_data.get_sender();
        let _ = tx.send(PlaybackCmd::SetUserData("width".to_string(), UserData::USize((size.0) as usize)));
        let _ = tx.send(PlaybackCmd::SetUserData("height".to_string(), UserData::USize((size.1) as usize)));
        let _ = tx.send(PlaybackCmd::SetUserData("x".to_string(), UserData::ISize(0)));
        let _ = tx.send(PlaybackCmd::SetUserData("y".to_string(), UserData::ISize(0)));
    }

    let mut last_time = SystemTime::now();
    let mut last_char= '\0';

    if let Err(_e) = crossterm::terminal::enable_raw_mode() {}
    loop {
        if song_data.is_stopped() {break;}
        // let input = tokio::time::timeout(Duration::from_secs(1), getter.getch()).await;
        if crossterm::event::poll(Duration::from_millis(10)).is_ok() {
            // It's guaranteed that the `read()` won't block when the `poll()`
            // function returns `true`
            match crossterm::event::read() {
                Ok(crossterm::event::Event::Key(event)) => {
                    let tx = song_data.get_sender();
                    match event.code {
                        KeyCode::Backspace => {}
                        KeyCode::Enter => {}
                        KeyCode::Left => {
                            let _ = tx.send(PlaybackCmd::SeekBackward10s);
                        }
                        KeyCode::Right => {
                            let _ = tx.send(PlaybackCmd::SeekForward10s);
                        }
                        KeyCode::Up => {
                            let _ = tx.send(PlaybackCmd::ModifyUserDataSubISize("y".to_string(), 1));
                        }
                        KeyCode::Down => {
                            let _ = tx.send(PlaybackCmd::ModifyUserDataAddISize("y".to_string(), 1));
                        }
                        // KeyCode::Null => {}
                        KeyCode::Esc => {
                            let tx = song_data.get_sender();
                            let _ = tx.send(PlaybackCmd::Quit);
                            break;
                        }
                        // KeyCode::Home => {}
                        // KeyCode::End => {}
                        // KeyCode::PageUp => {}
                        // KeyCode::PageDown => {}
                        // KeyCode::Tab => {}
                        // KeyCode::BackTab => {}
                        // KeyCode::Delete => {}
                        // KeyCode::Insert => {}
                        KeyCode::F(num) => {
                            let tx = &mut song_data.get_sender();
                            match num {
                                1 => {
                                    let _ = tx.send(PlaybackCmd::SetViewMode(0));
                                }
                                2 => {
                                    let _ = tx.send(PlaybackCmd::SetViewMode(1));
                                }
                                3 => {
                                    let _ = tx.send(PlaybackCmd::SetViewMode(2));
                                }
                                4 => {
                                    let _ = tx.send(PlaybackCmd::SetViewMode(3));
                                }
                                _ => {}
                            }

                        }
                        KeyCode::Char(ch) => {
                            let tx = &mut song_data.get_sender();
                            match ch {
                                'q' => {
                                    let _ = tx.send(PlaybackCmd::Quit);
                                    break;
                                }
                                'c' => {
                                    let _ = tx.send(PlaybackCmd::ModifyUserDataAddUSize("view_mode".to_string(), 1));
                                }
                                '0'..='9' => {
                                    let ch_idx = if ch == '0' { 9 } else { ch as u8 - '1' as u8 };
                                    if event.modifiers.contains(KeyModifiers::SHIFT) {
                                        let _ = tx.send(PlaybackCmd::ChannelSolo(ch_idx));
                                    } else {
                                        let _ = tx.send(PlaybackCmd::ChannelToggle(ch_idx));
                                    }
                                }
                                'a' => {
                                    let _ = tx.send(PlaybackCmd::ChannelUnmuteAll);
                                }
                                'm' => {
                                    let _ = tx.send(PlaybackCmd::ChannelMuteAll);
                                }
                                '+' => {
                                    let _ = tx.send(PlaybackCmd::IncSpeed);
                                }
                                '-' => {
                                    let _ = tx.send(PlaybackCmd::DecSpeed);
                                }
                                '.' => {
                                    let _ = tx.send(PlaybackCmd::IncBPM);
                                }
                                ',' => {
                                    let _ = tx.send(PlaybackCmd::DecBPM);
                                }
                                ' ' => {
                                    let _ = tx.send(PlaybackCmd::PauseToggle);
                                }
                                'n' => {
                                    let _ = tx.send(PlaybackCmd::Next);
                                }
                                '/' => {
                                    let _ = tx.send(PlaybackCmd::LoopPattern);
                                }
                                'r' => {
                                    let _ = tx.send(PlaybackCmd::Restart);
                                }
                                'A' => {
                                    let _ = tx.send(PlaybackCmd::AmigaTable);
                                }
                                'l' => {
                                    let _ = tx.send(PlaybackCmd::LinearTable);
                                }
                                'f' => {
                                    let _ = tx.send(PlaybackCmd::FilterToggle);
                                }
                                'd' => {
                                    let _ = tx.send(PlaybackCmd::DisplayToggle);
                                }
                                't' | 'T' => {
                                    let _ = tx.send(PlaybackCmd::CycleTheme);
                                }
                                'v' | 'V' => {
                                    let _ = tx.send(PlaybackCmd::ToggleVisualizerMode);
                                }
                                's' | 'S' => {
                                    let _ = tx.send(PlaybackCmd::ToggleScopes);
                                }
                                'p' | 'P' => {
                                    let _ = tx.send(PlaybackCmd::Prev);
                                }
                                '[' => {
                                    let _ = tx.send(PlaybackCmd::ModifyUserDataSubISize("x".to_string(), 1));
                                }
                                ']' => {
                                    let _ = tx.send(PlaybackCmd::ModifyUserDataAddISize("x".to_string(), 1));
                                }
                                '(' => {
                                    let _ = tx.send(PlaybackCmd::DecLatency);
                                }
                                ')' => {
                                    let _ = tx.send(PlaybackCmd::IncLatency);
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                },
                Ok(crossterm::event::Event::Resize(x, y)) => {
                    let tx = song_data.get_sender();
                    let _ = tx.send(PlaybackCmd::SetUserData("width".to_string(), UserData::USize(x as usize)));
                    let _ = tx.send(PlaybackCmd::SetUserData("height".to_string(), UserData::USize(y as usize)));
                },
                _ => {
                    continue;
                }
            }
        } else {
            continue;
        }
    }
}
