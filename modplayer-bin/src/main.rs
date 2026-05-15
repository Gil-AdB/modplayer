use xmplayer::song::{PlaybackCmd, UserData};
use xmplayer::module_reader::print_module;
use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::io::{stdout, Write};

use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::event::{
    KeyCode, KeyEventKind, KeyModifiers, MediaKeyCode,
    KeyboardEnhancementFlags, PushKeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
};
use xmplayer::song_state::{SongState, SongHandle};
use xmplayer::AudioConsumer;

#[cfg(feature="sdl2-feature")] mod sdl2_audio;
#[cfg(feature="sdl2-feature")] use sdl2_audio::AudioOutput;
#[cfg(feature="portaudio-feature")] mod portaudio_audio;
#[cfg(feature="portaudio-feature")] use portaudio_audio::AudioOutput;
use crossterm::cursor::{Hide, MoveTo, Show};
use display::display::{Display, TargetPlatform};
use display::{ViewPort, Grid};

mod settings;
use settings::Settings;

#[cfg(target_os = "macos")]
mod media_keys_macos;
#[cfg(target_os = "macos")]
use media_keys_macos::{MediaKey, MediaKeysHandle};

// On non-macOS, stub the handle as a unit so the call sites compile.
// The actual integration is OS-specific; future Linux (MPRIS via souvlaki
// with use_dbus feature) / Windows (SMTC) backends would slot in here.
#[cfg(not(target_os = "macos"))]
pub struct MediaKeysHandle;
#[cfg(not(target_os = "macos"))]
impl MediaKeysHandle {
    pub fn pump(&self) {}
    pub fn try_recv(&self) -> Option<MediaKey> { None }
    pub fn set_song(&self, _: &str, _: f32) {}
    pub fn set_playing(&self, _: bool) {}
    pub fn set_progress(&self, _: f32) {}
}
#[cfg(not(target_os = "macos"))]
#[derive(Debug, Clone, Copy)]
pub enum MediaKey { Toggle, Stop, Next, Previous, SeekToSeconds(f32) }

fn main() {
    if env::args().len() < 2 { return; }

    let cli_args: Vec<String> = env::args().skip(1).collect();
    let dump_mode = cli_args.iter().any(|a| a == "--dump");
    let non_flag: Vec<&String> = cli_args.iter().filter(|a| !a.starts_with("--")).collect();
    if non_flag.is_empty() { return; }

    // Heuristic: treat extra args as a playlist only if every one of them is
    // an existing file. If any extra isn't a file, fall back to the legacy
    // print_module debug shorthand (`modplayer file.s3m 5 10` → debug-print
    // patterns 5 and 10 of the first file). This keeps backward compat for
    // the existing single-file flow while making `modplayer a.s3m b.xm c.it`
    // do the obvious thing.
    let all_files = non_flag.iter().all(|a| Path::new(a.as_str()).is_file());

    if dump_mode {
        // Dump mode is a developer affordance; only the first file is dumped.
        let path = non_flag[0].clone();
        match SongState::new(&path) {
            Ok((mut song, consumer)) => run_dump(&mut song, consumer),
            Err(e) => { dbg!(e); }
        }
        return;
    }

    if !all_files && non_flag.len() > 1 {
        // Legacy print_module path — first arg is the file, rest are pattern
        // indices to debug-print.
        let path = non_flag[0].clone();
        let extras: Vec<String> = non_flag[1..].iter().map(|s| s.to_string()).collect();
        match SongState::new(&path) {
            Ok((song, _consumer)) => print_module(&song, extras.into_iter()),
            Err(e) => { dbg!(e); }
        }
        return;
    }

    let playlist: Vec<PathBuf> = non_flag.iter().map(|s| PathBuf::from(s.as_str())).collect();
    run_playlist(playlist);
}

fn run_dump(song_data: &mut SongHandle, consumer: AudioConsumer) {
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
            let _ = crossterm::execute!(stdout(), PopKeyboardEnhancementFlags);
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = crossterm::execute!(stdout(), Show, LeaveAlternateScreen);
            default_hook(info);
        }));

        if let Err(_e) = crossterm::execute!(stdout(), EnterAlternateScreen) {}
        let _ = crossterm::terminal::enable_raw_mode();
        // Request kitty keyboard protocol — the DISAMBIGUATE_ESCAPE_CODES
        // bit alone unlocks the media-key path. We deliberately do NOT
        // ask for REPORT_EVENT_TYPES / REPORT_ALTERNATE_KEYS: with iTerm
        // 3.5 those caused a CPU storm (iTerm reportedly pegged at 150%)
        // — likely because every key turns into a press+release event
        // pair, multiplying terminal-side encoding work and our event
        // loop churn. Press-only is enough for our needs; the kind
        // filter below stays defensive in case future terminals send
        // releases anyway.
        let _ = crossterm::execute!(
            stdout(),
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES,
            ),
        );
        TerminalModeSetter {}
    }
}

impl Drop for TerminalModeSetter {
    fn drop(&mut self) {
        let _ = crossterm::execute!(stdout(), PopKeyboardEnhancementFlags);
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(stdout(), Show, LeaveAlternateScreen);
    }
}


/// What ended the inner mainloop — drives playlist navigation.
enum LoopExit {
    /// User pressed q / Esc; tear everything down and stop.
    Quit,
    /// Song reached natural end; advance to next playlist item.
    SongEnded,
    /// User pressed `>`; jump to next playlist item.
    NextSong,
    /// User pressed `<`; jump to previous playlist item.
    PrevSong,
}

fn run_playlist(items: Vec<PathBuf>) {
    let _mode_setter = TerminalModeSetter::new();

    // Initialize OS-level media keys + Now Playing once per process so
    // the system sees us as a continuous audio app across the whole
    // playlist (not per-song registration churn). On non-macOS this is
    // a no-op stub.
    #[cfg(target_os = "macos")]
    let media_keys: Option<MediaKeysHandle> = match media_keys_macos::init("modplayer") {
        Ok(h) => Some(h),
        Err(e) => { eprintln!("media-keys init failed: {}", e); None }
    };
    #[cfg(not(target_os = "macos"))]
    let media_keys: Option<MediaKeysHandle> = None;

    // Load once at session start; carry the latest UI state forward
    // between tracks so toggling theme on track 1 sticks for track 2.
    // Save on every track end (not just the final one) so a kill -9 mid-
    // playlist keeps the user's preferences.
    let mut settings = Settings::load();
    let mut idx: usize = 0;

    while idx < items.len() {
        let path = items[idx].to_string_lossy().to_string();
        // Surface the path to stderr so a stuck song can be identified
        // in scrollback even after the UI has redrawn over the header.
        eprintln!("[{}/{}] loading: {}", idx + 1, items.len(), path);
        let (mut song_data, consumer) = match SongState::new(&path) {
            Ok(s) => s,
            Err(e) => { eprintln!("  load failed: {:?}", e); idx += 1; continue; }
        };

        // Push the new song's title to Control Center + flip to Playing.
        // Use the IT/XM/etc internal song name if present, otherwise the
        // file name. The OS uses this as the routing hint for media keys.
        if let Some(mk) = media_keys.as_ref() {
            let (title, duration_secs) = {
                let song = song_data.get_song().lock().unwrap();
                let internal = song.name.trim().to_string();
                let title = if internal.is_empty() {
                    Path::new(&path).file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("modplayer")
                        .to_string()
                } else {
                    internal
                };
                (title, song.total_duration_ms / 1000.0)
            };
            mk.set_song(&title, duration_secs);
            mk.set_playing(true);
        }

        let exit = run_one(&mut song_data, consumer, &settings, media_keys.as_ref());

        // Refresh `settings` from the song's final UI state and persist.
        {
            let song = song_data.get_song().lock().unwrap();
            settings = Settings {
                theme_id: song.theme_id,
                filter: song.filter,
                view_mode: song.view_mode,
                visualizer_enabled: song.visualizer_enabled,
                visualizer_mode: song.visualizer_mode,
            };
        }
        settings.save();

        match exit {
            LoopExit::Quit => break,
            LoopExit::SongEnded | LoopExit::NextSong => { idx = idx.saturating_add(1); }
            LoopExit::PrevSong => { idx = idx.saturating_sub(1); }
        }
    }
}

fn run_one(song_data: &mut SongHandle, consumer: AudioConsumer, settings: &Settings, media_keys: Option<&MediaKeysHandle>) -> LoopExit {
    const _CHANNELS: i32 = 2;
    const SAMPLE_RATE: f32 = 48_000.0;

    let mut audio = AudioOutput::new(consumer, SAMPLE_RATE);

    // Apply persisted UI preferences before the audio thread starts.
    // Direct field mutation (rather than send-cycle-N-times) is safe here
    // because the audio thread isn't running yet — `song_data.start()` is
    // what spawns it. Grab `channel_count` while we're holding the lock,
    // because once the audio thread starts it holds the song mutex for
    // the entire song lifetime (see SongState::callback) and any later
    // attempt to `.lock()` would deadlock the keyboard handler.
    let channel_count;
    {
        let mut song = song_data.get_song().lock().unwrap();
        song.theme_id = settings.theme_id;
        song.filter = settings.filter;
        song.view_mode = settings.view_mode;
        song.visualizer_enabled = settings.visualizer_enabled;
        song.visualizer_mode = settings.visualizer_mode;
        channel_count = song.get_channel_count();
    }

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
    let exit = mainloop(song_data, channel_count, media_keys);

    song_data.close();
    if handle.0.is_some() {
        handle.0.unwrap().join().unwrap();
    }
    if handle.1.is_some() {
        handle.1.unwrap().join().unwrap();
    }

    audio.close();
    exit
}

/// Command-palette + channel-cursor input modes.
///
/// `Command(buf)` is `:`-prefixed. While in it, all printable keys go
/// into `buf` instead of dispatching; Enter executes, Esc cancels. The
/// buffer is mirrored to the audio-thread display via `cmdline_buf` /
/// `cmdline_show` user_data.
///
/// `ChannelCursor(idx)` is `g`-entered. Arrow keys move the highlight
/// across channel rows; m / s / u / a act on the highlighted channel;
/// Esc exits. The cursor is mirrored to the display via `channel_cursor`
/// user_data (1-indexed; 0 = no cursor).
enum InputMode {
    Normal,
    Command(String),
    ChannelCursor(usize),
}

/// Push the current command-line buffer into UserData so the display
/// thread renders the status line. `show=false` hides it.
fn sync_cmdline(tx: &std::sync::mpsc::Sender<PlaybackCmd>, show: bool, buf: &str) {
    let _ = tx.send(PlaybackCmd::SetUserData(
        "cmdline_show".to_string(),
        UserData::USize(if show { 1 } else { 0 }),
    ));
    let _ = tx.send(PlaybackCmd::SetUserData(
        "cmdline_buf".to_string(),
        UserData::String(buf.to_string()),
    ));
}

/// Push the channel-cursor index (1-based) so the display can highlight
/// the active row. `idx=None` clears the cursor.
fn sync_channel_cursor(tx: &std::sync::mpsc::Sender<PlaybackCmd>, idx: Option<usize>) {
    let value = match idx { Some(i) => i + 1, None => 0 };
    let _ = tx.send(PlaybackCmd::SetUserData(
        "channel_cursor".to_string(),
        UserData::USize(value),
    ));
}

/// Parse and dispatch a `:`-command. Returns Some(LoopExit) if the command
/// requires the mainloop to return (quit / next song / prev song);
/// otherwise None. Channel indices in commands are 1-based for human use
/// and converted to 0-based PlaybackCmd args here.
fn execute_command(buf: &str, tx: &std::sync::mpsc::Sender<PlaybackCmd>) -> Option<LoopExit> {
    let toks: Vec<&str> = buf.split_whitespace().collect();
    if toks.is_empty() { return None; }
    match toks[0] {
        "q" | "quit" => {
            let _ = tx.send(PlaybackCmd::Quit);
            return Some(LoopExit::Quit);
        }
        "next" => return Some(LoopExit::NextSong),
        "prev" => return Some(LoopExit::PrevSong),
        "ch" if toks.len() == 3 => {
            // :ch <N> <m|s|u>   — N is 1-based.
            let Ok(n) = toks[1].parse::<u32>() else { return None; };
            if n == 0 { return None; }
            let idx = (n - 1) as u8;
            match toks[2] {
                "m" | "mute"  => { let _ = tx.send(PlaybackCmd::ChannelToggle(idx)); }
                "s" | "solo"  => { let _ = tx.send(PlaybackCmd::ChannelSolo(idx)); }
                "u" | "unmute" => {
                    // No "unmute single channel" command; toggle works if
                    // currently muted, no-op otherwise. Accept as alias.
                    let _ = tx.send(PlaybackCmd::ChannelToggle(idx));
                }
                _ => {}
            }
        }
        "mute" if toks.get(1) == Some(&"all") => { let _ = tx.send(PlaybackCmd::ChannelMuteAll); }
        "unmute" if toks.get(1) == Some(&"all") => { let _ = tx.send(PlaybackCmd::ChannelUnmuteAll); }
        "goto" if toks.len() == 2 => {
            // Accept hex (`0x14`, `14h`) and decimal.
            let s = toks[1];
            let parsed = if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                u32::from_str_radix(rest, 16).ok()
            } else if let Some(rest) = s.strip_suffix('h').or_else(|| s.strip_suffix('H')) {
                u32::from_str_radix(rest, 16).ok()
            } else {
                s.parse::<u32>().ok()
            };
            if let Some(n) = parsed {
                let _ = tx.send(PlaybackCmd::SetPosition(n));
            }
        }
        _ => {}
    }
    None
}

fn mainloop(song_data: &SongState, channel_count: usize, media_keys: Option<&MediaKeysHandle>) -> LoopExit {

    if let Ok(size) = crossterm::terminal::size() {
        let tx = song_data.get_sender();
        let _ = tx.send(PlaybackCmd::SetUserData("width".to_string(), UserData::USize((size.0) as usize)));
        let _ = tx.send(PlaybackCmd::SetUserData("height".to_string(), UserData::USize((size.1) as usize)));
        let _ = tx.send(PlaybackCmd::SetUserData("x".to_string(), UserData::ISize(0)));
        let _ = tx.send(PlaybackCmd::SetUserData("y".to_string(), UserData::ISize(0)));
    }

    let mut mode = InputMode::Normal;

    if let Err(_e) = crossterm::terminal::enable_raw_mode() {}
    // Local pause-state mirror. Updated by every code path that sends
    // PlaybackCmd::PauseToggle so Control Center's play/pause icon stays
    // in sync. Uses RefCell because the borrow checker would otherwise
    // refuse to capture `&mut paused` in the closure while the main
    // loop already holds `&mut` to its surrounding state.
    let paused = std::cell::RefCell::new(false);
    // Helper: toggle pause + sync the OS media controls. Call this for
    // every PauseToggle origin (keyboard space, media-key Toggle, future
    // command-palette / scripted toggles).
    let toggle_pause = |tx: &std::sync::mpsc::Sender<PlaybackCmd>| {
        let _ = tx.send(PlaybackCmd::PauseToggle);
        let mut p = paused.borrow_mut();
        *p = !*p;
        if let Some(mk) = media_keys {
            mk.set_playing(!*p);
        }
    };
    loop {
        // Natural song-end → advance to next playlist item.
        if song_data.is_stopped() { return LoopExit::SongEnded; }

        // Drain any pending macOS media-key callbacks: pump the CFRunLoop
        // briefly (non-blocking) so souvlaki's registered handlers fire,
        // then read whatever ended up in its channel and translate to the
        // player's commands. This is the *whole* reason we don't need a
        // separate NSApplication run loop — once per outer iteration is
        // enough latency for a key press to feel instant (<10 ms typical).
        if let Some(mk) = media_keys {
            mk.pump();
            while let Some(key) = mk.try_recv() {
                let tx = song_data.get_sender();
                match key {
                    MediaKey::Toggle => {
                        toggle_pause(&tx);
                    }
                    MediaKey::Stop => {
                        let _ = tx.send(PlaybackCmd::Quit);
                        return LoopExit::Quit;
                    }
                    MediaKey::Next => {
                        let _ = tx.send(PlaybackCmd::Quit);
                        return LoopExit::NextSong;
                    }
                    MediaKey::Previous => {
                        let _ = tx.send(PlaybackCmd::Quit);
                        return LoopExit::PrevSong;
                    }
                    MediaKey::SeekToSeconds(t) => {
                        let _ = tx.send(PlaybackCmd::SeekToSeconds(t));
                    }
                }
            }

            // Push the current position to Control Center periodically.
            // 500 ms is responsive without thrashing the system pasteboard
            // — macOS interpolates between updates so the thumb stays
            // smooth. Lock window kept short: just read total_samples and
            // drop the song mutex before the FFI call.
            const PROGRESS_INTERVAL_MS: u64 = 500;
            use std::time::Instant;
            // RefCell-free: keep the last push time on the stack via a
            // mutable local. The outer `loop` already owns `paused`, so
            // adding one more local is consistent.
            // Note: we can't easily put a static here because mainloop
            // is called once per song and we want fresh state per song.
            let now_ms = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()) as u64;
            // Use a process-local-once cell as the "last push" cursor.
            // We rely on the 10-ms poll cadence to keep the modulo
            // check coarse — checking every iteration is cheap.
            thread_local! {
                static LAST_PROGRESS_MS: std::cell::Cell<u64> = std::cell::Cell::new(0);
            }
            let should_push = LAST_PROGRESS_MS.with(|c| {
                let last = c.get();
                if now_ms.saturating_sub(last) >= PROGRESS_INTERVAL_MS {
                    c.set(now_ms);
                    true
                } else {
                    false
                }
            });
            if should_push {
                // try_lock instead of lock: the audio thread holds the
                // song mutex during seek/fast-forward, which can run for
                // hundreds of ms on long backward seeks. Blocking here
                // would deadlock the keyboard event poll on the main
                // thread — user reports "seek hangs the player
                // interaction". macOS interpolates between progress
                // updates so missing one tick is invisible; we'll push
                // the next one when the lock is free.
                if let Ok(song) = song_data.get_song().try_lock() {
                    let elapsed = song.total_samples as f32 / song.rate;
                    drop(song);
                    mk.set_progress(elapsed);
                }
            }
            let _ = now_ms;
            let _ = Instant::now();
        }

        // `event::poll` returns Ok(true) when an event is ready and Ok(false)
        // on timeout. The previous `.is_ok()` check entered the `read()` arm
        // on *both* — and `read()` blocks indefinitely waiting for input.
        // That meant a song-end with no key press left the mainloop stuck
        // here forever; the next is_stopped check never ran. Match on
        // `Ok(true)` so timeouts loop back and re-check is_stopped.
        if matches!(crossterm::event::poll(Duration::from_millis(10)), Ok(true)) {
            // It's guaranteed that the `read()` won't block when the `poll()`
            // function returns `true`
            match crossterm::event::read() {
                Ok(crossterm::event::Event::Key(event)) => {
                    // With REPORT_EVENT_TYPES on under the kitty keyboard
                    // protocol, terminals deliver Press / Release / Repeat
                    // events. Filter to Press only so a single tap of any
                    // key doesn't double-fire its command. KeyEventKind
                    // is implicitly Press on terminals that don't support
                    // the protocol, so this is a no-op there.
                    if event.kind != KeyEventKind::Press {
                        continue;
                    }
                    let tx = song_data.get_sender();

                    // Command-palette mode short-circuits all normal keymapping.
                    if let InputMode::Command(ref buf) = mode {
                        match event.code {
                            KeyCode::Esc => {
                                sync_cmdline(&tx, false, "");
                                mode = InputMode::Normal;
                            }
                            KeyCode::Enter => {
                                let cmd = buf.clone();
                                sync_cmdline(&tx, false, "");
                                mode = InputMode::Normal;
                                if let Some(exit) = execute_command(&cmd, &tx) {
                                    return exit;
                                }
                            }
                            KeyCode::Backspace => {
                                let mut new_buf = buf.clone();
                                new_buf.pop();
                                sync_cmdline(&tx, true, &new_buf);
                                mode = InputMode::Command(new_buf);
                            }
                            KeyCode::Char(c) => {
                                let mut new_buf = buf.clone();
                                new_buf.push(c);
                                sync_cmdline(&tx, true, &new_buf);
                                mode = InputMode::Command(new_buf);
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // Channel-cursor mode also short-circuits normal keys.
                    // Up/Down move by 1 (matches the channel rows being
                    // stacked vertically); Left/Right do the same so users
                    // who reach for either pair land on the right thing.
                    if let InputMode::ChannelCursor(idx) = mode {
                        let max_idx = channel_count.saturating_sub(1);
                        match event.code {
                            KeyCode::Esc => {
                                sync_channel_cursor(&tx, None);
                                mode = InputMode::Normal;
                            }
                            KeyCode::Up | KeyCode::Left => {
                                let new_idx = idx.saturating_sub(1);
                                sync_channel_cursor(&tx, Some(new_idx));
                                mode = InputMode::ChannelCursor(new_idx);
                            }
                            KeyCode::Down | KeyCode::Right => {
                                let new_idx = (idx + 1).min(max_idx);
                                sync_channel_cursor(&tx, Some(new_idx));
                                mode = InputMode::ChannelCursor(new_idx);
                            }
                            KeyCode::Char('m') => {
                                let _ = tx.send(PlaybackCmd::ChannelToggle(idx as u8));
                            }
                            KeyCode::Char('s') => {
                                let _ = tx.send(PlaybackCmd::ChannelSolo(idx as u8));
                            }
                            KeyCode::Char('u') => {
                                // No "unmute single" command; toggle works
                                // when the channel is muted, no-op otherwise.
                                let _ = tx.send(PlaybackCmd::ChannelToggle(idx as u8));
                            }
                            KeyCode::Char('a') => {
                                let _ = tx.send(PlaybackCmd::ChannelUnmuteAll);
                            }
                            _ => {}
                        }
                        continue;
                    }
                    // System media keys: routed through the kitty keyboard
                    // protocol enabled in `TerminalModeSetter`. On supported
                    // terminals (kitty/WezTerm/alacritty/foot/ghostty),
                    // pressing the OS Play/Pause/Stop/Next/Prev keys
                    // delivers a `KeyCode::Media(...)` event we can map to
                    // existing playback commands. Other terminals just
                    // don't get these events, no harm done.
                    if let KeyCode::Media(mk) = event.code {
                        match mk {
                            MediaKeyCode::PlayPause
                            | MediaKeyCode::Play
                            | MediaKeyCode::Pause => {
                                toggle_pause(&tx);
                            }
                            MediaKeyCode::Stop => {
                                let _ = tx.send(PlaybackCmd::Quit);
                                return LoopExit::Quit;
                            }
                            MediaKeyCode::TrackNext | MediaKeyCode::FastForward => {
                                let _ = tx.send(PlaybackCmd::Quit);
                                return LoopExit::NextSong;
                            }
                            MediaKeyCode::TrackPrevious | MediaKeyCode::Rewind => {
                                let _ = tx.send(PlaybackCmd::Quit);
                                return LoopExit::PrevSong;
                            }
                            _ => {}
                        }
                        continue;
                    }

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
                            return LoopExit::Quit;
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
                                    return LoopExit::Quit;
                                }
                                ':' => {
                                    // Enter command-palette input mode. The
                                    // outer `if let InputMode::Command...`
                                    // branch will handle subsequent keys.
                                    sync_cmdline(&tx, true, "");
                                    mode = InputMode::Command(String::new());
                                }
                                'g' => {
                                    // Enter channel-cursor mode at channel 0.
                                    sync_channel_cursor(&tx, Some(0));
                                    mode = InputMode::ChannelCursor(0);
                                }
                                '>' => {
                                    // Next playlist track (Shift-period).
                                    let _ = tx.send(PlaybackCmd::Quit);
                                    return LoopExit::NextSong;
                                }
                                '<' => {
                                    // Previous playlist track (Shift-comma).
                                    let _ = tx.send(PlaybackCmd::Quit);
                                    return LoopExit::PrevSong;
                                }
                                'c' => {
                                    let _ = tx.send(PlaybackCmd::ModifyUserDataAddUSize("view_mode".to_string(), 1));
                                }
                                '0'..='9' => {
                                    // Bare digit: channels 1-10 (0 = ch10).
                                    // Alt+digit:  channels 11-20 for files with >10 channels.
                                    // Shift+digit: solo (mutes everything else).
                                    let base = if ch == '0' { 9 } else { ch as u8 - b'1' };
                                    let alt = event.modifiers.contains(KeyModifiers::ALT);
                                    let shift = event.modifiers.contains(KeyModifiers::SHIFT);
                                    let ch_idx = base + if alt { 10 } else { 0 };
                                    if shift {
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
                                    toggle_pause(&tx);
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
