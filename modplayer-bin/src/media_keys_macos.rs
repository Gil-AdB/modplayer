//! macOS Now Playing / media-key integration via `MPRemoteCommandCenter`.
//!
//! Goal: receive OS-level media key events (Play / Pause / Stop / Next /
//! Previous) regardless of which app has focus, and surface the current
//! song in the Control Center "Now Playing" widget. souvlaki wraps the
//! `MediaPlayer.framework` Objective-C API for us; this module owns the
//! `MediaControls` handle and bridges its events back into the player's
//! existing `PlaybackCmd` flow.
//!
//! ## Why pump CFRunLoop instead of starting a side thread
//!
//! `MPRemoteCommandCenter` blocks are dispatched on the *main thread's*
//! CFRunLoop by default. Our terminal player's main thread runs a
//! crossterm event poll, not a CFRunLoop — so callbacks would queue
//! forever waiting for a loop that never drains. The classic fix is to
//! split work: NSApplication on main, app logic on a worker. That's
//! invasive.
//!
//! Instead we call `CFRunLoopRunInMode(default, 0, returnAfterSource)`
//! from inside the existing 10 ms event poll. That drains any queued
//! media-key blocks immediately and returns, so the main thread stays
//! responsible for terminal input as before.

use std::sync::{mpsc, Mutex};

use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, PlatformConfig,
};

use core_foundation::base::Boolean;
use core_foundation::runloop::CFRunLoopRunInMode;
use core_foundation::string::CFStringRef;

unsafe extern "C" {
    static kCFRunLoopDefaultMode: CFStringRef;
}

/// Subset of `souvlaki::MediaControlEvent` mapped to actions the player
/// already knows how to do.
#[derive(Debug, Clone, Copy)]
pub enum MediaKey {
    /// Play/Pause toggle. We collapse standalone Play and Pause into
    /// Toggle since the player has a single PauseToggle command.
    Toggle,
    /// Hard stop. Mapped to "quit the current song".
    Stop,
    /// Next track / fast-forward.
    Next,
    /// Previous track / rewind.
    Previous,
}

pub struct MediaKeysHandle {
    /// Behind a mutex because souvlaki's MediaControls is `!Sync` and we
    /// touch it from both the main loop (set_song_title / set_playing)
    /// and the OS callback thread (when MediaControls drops it).
    controls: Mutex<MediaControls>,
    rx: mpsc::Receiver<MediaKey>,
}

/// Initialize the media controls, register key handlers, and return a
/// handle that polls for events. Returns `Err` if the OS refuses to
/// register the player.
pub fn init(display_name: &'static str) -> Result<MediaKeysHandle, String> {
    let config = PlatformConfig {
        dbus_name: "modplayer",
        display_name,
        hwnd: None,
    };
    let mut controls = MediaControls::new(config).map_err(|e| format!("{:?}", e))?;
    let (tx, rx) = mpsc::sync_channel::<MediaKey>(32);

    controls
        .attach(move |evt| {
            let mapped = match evt {
                MediaControlEvent::Toggle
                | MediaControlEvent::Play
                | MediaControlEvent::Pause => Some(MediaKey::Toggle),
                MediaControlEvent::Stop => Some(MediaKey::Stop),
                MediaControlEvent::Next => Some(MediaKey::Next),
                MediaControlEvent::Previous => Some(MediaKey::Previous),
                _ => None,
            };
            if let Some(k) = mapped {
                // Best-effort: if the receiver is gone or the buffer is
                // full, drop the event rather than panic the OS callback.
                let _ = tx.try_send(k);
            }
        })
        .map_err(|e| format!("{:?}", e))?;

    // Start in a paused state — the caller flips this to Playing once
    // the first song is loaded.
    let _ = controls.set_playback(MediaPlayback::Paused { progress: None });

    Ok(MediaKeysHandle {
        controls: Mutex::new(controls),
        rx,
    })
}

impl MediaKeysHandle {
    /// Drain queued OS callbacks. Call once per main-loop iteration.
    /// Runs the macOS CFRunLoop briefly in non-blocking mode so any
    /// `addTargetWithHandler:` blocks the framework queued for the main
    /// thread actually fire and end up in our channel.
    pub fn pump(&self) {
        unsafe {
            // mode: kCFRunLoopDefaultMode
            // seconds: 0.0 → return immediately if no work
            // returnAfterSourceHandled: true → process one source, then
            // return; avoids monopolizing the main thread if many events
            // queued at once.
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.0, true as Boolean);
        }
    }

    /// Non-blocking receive of the next pending media-key event, if any.
    pub fn try_recv(&self) -> Option<MediaKey> {
        self.rx.try_recv().ok()
    }

    /// Update the Now Playing entry with the current song title. Empty
    /// / whitespace-only titles fall back to "modplayer".
    pub fn set_song_title(&self, title: &str) {
        let trimmed = title.trim();
        let display = if trimmed.is_empty() { "modplayer" } else { trimmed };
        if let Ok(mut controls) = self.controls.lock() {
            let _ = controls.set_metadata(MediaMetadata {
                title: Some(display),
                artist: None,
                album: None,
                duration: None,
                cover_url: None,
            });
        }
    }

    /// Update the system's playing/paused state. Drives the Control
    /// Center play/pause icon and tells macOS this app currently has
    /// audio output, which biases media-key routing toward us.
    pub fn set_playing(&self, playing: bool) {
        if let Ok(mut controls) = self.controls.lock() {
            let state = if playing {
                MediaPlayback::Playing { progress: None }
            } else {
                MediaPlayback::Paused { progress: None }
            };
            let _ = controls.set_playback(state);
        }
    }
}
