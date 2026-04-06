# modplayer

A cross-platform module music player written in Rust, with native CLI and WebAssembly (WASM) browser backends.

## Overview

Modplayer plays classic tracker music files (MOD, S3M, XM, STM) with a real-time channel visualization display. It aims for accuracy by replicating FastTracker 2 behavior, including known FT2 quirks and bugs for compatibility.

## Supported Formats

| Format | Status | Notes |
|--------|--------|-------|
| **XM** (FastTracker 2) | ✅ Fully supported | Primary format, most tested |
| **MOD** (ProTracker) | ✅ Supported | Classic Amiga module format |
| **S3M** (Scream Tracker 3) | ✅ Supported | |
| **STM** (Scream Tracker 2) | ✅ Supported | |
| **IT** (Impulse Tracker) | 🚧 WIP | Header parsing and instrument reading started; not yet playable |

## Architecture

The project is organized as a Cargo workspace with the following crates:

```
modplayer/
├── xmplayer/            # Core library — format parsing, playback engine
├── display/             # Terminal display module (ANSI escape codes)
├── modplayer-bin/       # Native CLI player (SDL2 or PortAudio audio output)
├── modplayer-wasm/      # WebAssembly browser player (wasm-bindgen + Web Audio API)
├── modplayer-lib/       # Static library build (C FFI, not actively used)
├── modplayer-bindgen/   # Older wasm-bindgen approach (legacy, superseded by modplayer-wasm)
├── modplayer-emscripten/# Emscripten-based build (legacy, superseded by modplayer-wasm)
├── vendor/              # Vendored dependencies (termios for Termux/Android)
└── tests/               # Integration tests (placeholder)
```

### Core Library (`xmplayer`)

The core playback engine, independent of any audio backend or display:

- **`module_reader/`** — Format-specific parsers: `xm.rs`, `module.rs` (MOD), `s3m.rs`, `stm.rs`, `it.rs`. Autodetects format via `open_module()`.
- **`song/`** — Main playback engine. Manages tick processing, effect handling, sample mixing, and buffer filling. Implements all XM/MOD/S3M effects (arpeggio, portamento, vibrato, tremolo, volume slides, etc.).
- **`channel_state/`** — Per-channel state: notes, envelopes, vibrato/tremolo oscillators, portamento, panning, volume.
- **`instrument/`** — Instrument, sample, and envelope data structures. Sample data is unpacked to f32.
- **`envelope/`** — Envelope point interpolation (volume, panning envelopes).
- **`tables/`** — Precomputed frequency tables (linear and Amiga period tables), panning tables. Bit-exact to FT2 tables.
- **`song_state/`** — Thread-safe song handle for the native player. Manages playback/display threads and producer-consumer audio queue.
- **`triple_buffer/`** — Lock-free triple buffer for passing display data from the audio thread to the display thread.
- **`producer_consumer_queue/`** — Audio buffer queue for the native backend.

### Native CLI Player (`modplayer-bin`)

Terminal-based player with real-time channel visualization:

- **Audio backends**: SDL2 (default) or PortAudio (selectable via Cargo features)
- **Display**: ANSI terminal with crossterm, showing channel volumes, notes, envelopes, pattern view
- **Controls**: Keyboard input for play/pause, pattern navigation, channel muting, BPM/speed adjustment, filter toggle

### WebAssembly Player (`modplayer-wasm`)

Browser-based player using wasm-bindgen:

- **Audio**: Web Audio API `ScriptProcessorNode` (deprecated but functional)
- **Display**: Canvas-based terminal emulator using the `wglt` library
- **UI**: File picker, drag-and-drop, play/pause/prev/next controls
- **Build**: wasm-pack → webpack dev server (npm)

## Build Requirements

### Native Build

- **Rust nightly** (uses `#![feature(seek_stream_len)]`)
- **SDL2 development libraries** (bundled by default via `sdl2/bundled` feature)
- Or **PortAudio** (optional, via `portaudio-feature`)

```bash
# Build and run the native player (SDL2 backend, default)
cargo build --release -p modplayer-bin
cargo run --release -p modplayer-bin -- <module_file.xm>
```

### WASM Build

- **Rust nightly** with `wasm32-unknown-unknown` target
- **wasm-pack** (v0.13+)
- **Node.js / npm**

```bash
# 1. Build the WASM package
cd modplayer-wasm
wasm-pack build --target web

# 2. Install npm dependencies and start the dev server
cd www
npm install
npm start
```

The dev server runs on `http://localhost:8080`. Drop module files onto the canvas or use the file picker.

> **Note**: The WASM build and npm setup currently require multiple manual steps. See the Roadmap below for planned improvements.

## Keyboard Controls

### Native Player
| Key | Action |
|-----|--------|
| `q` / `Esc` | Quit |
| `Space` | Pause/Resume |
| `n` / `p` | Next/Previous pattern |
| `r` | Restart current pattern |
| `+` / `-` | Increase/Decrease speed |
| `.` / `,` | Increase/Decrease BPM |
| `01`-`32` | Toggle channel mute (type two-digit number) |
| `f` | Toggle interpolation filter |
| `d` | Toggle display |
| `a` / `l` | Switch to Amiga/Linear frequency tables |
| `/` | Toggle pattern loop |
| `F1`/`F2`/`F3` | Slow down / Reset / Speed up playback rate |
| Arrow keys | Scroll display viewport |

### WASM Player
Same keyboard shortcuts apply when the browser canvas is focused.

## Audio Mixing

The mixer supports:
- **Linear interpolation** — toggled via the `f` key (enabled by default). Interpolates between adjacent sample points for smoother playback.
- **No interpolation** — nearest-neighbor (raw sample value)

Sample output is mixed per-channel with panning (using FT2-accurate panning tables) and volume envelopes (volume, panning, auto-vibrato).

### Volume Calculation
```
FinalVol = (FadeOutVol/65536) × (EnvelopeVol/16384) × (GlobalVol/64) × (ChannelVol/64)
```

## Known Issues & Limitations

1. **`#![feature(seek_stream_len)]`** — Requires Rust nightly. The IT reader uses `stream_len()` which is a nightly-only feature.
2. **`cargo-features = ["named-profiles"]`** — The workspace `Cargo.toml` uses `cargo-features` for named profiles, which may cause warnings on newer Rust versions where this feature is stabilized.
3. **`edition = "2024"`** — Cargo.toml files reference edition `2024` which may not be available on all nightly toolchains. May need adjustment.
4. **WASM display rendering** — The `wglt` terminal canvas can have issues with character alignment, color rendering, and pattern view layout.
5. **Deprecated Web Audio API** — Uses `createScriptProcessor` (deprecated). Should migrate to `AudioWorklet`.
6. **IT format** — Only header and partial instrument/sample parsing is implemented. Returns Error before completing. Pattern reading, playback effects, and the different NNA/DCT behavior are not yet implemented.
7. **Cubic interpolation** — `SplineData` struct exists in `channel_state` but the cubic interpolation method is commented out. Only linear and no-filter modes work.
8. **Package versions** — The npm `package.json` has a mix of old and current webpack versions, and references `webpack-dev-server` in both `dependencies` and `devDependencies`.
9. **Legacy crates** — `modplayer-bindgen` and `modplayer-emscripten` are not in the workspace and appear to be legacy/unused. (`modplayer-lib` is actively used by the Revival project via C FFI.)
10. **Tests** — The integration test file (`tests/tests.rs`) references a hardcoded file and won't compile as-is.

## Format Reference Documentation

Reference specs are in the `docs/` folder:

| File | Format | Description |
|------|--------|-------------|
| `xm.txt`, `XM_file_format.pdf` | XM | FastTracker 2 format specification |
| `xm_errata.txt` | XM | Known quirks and corrections |
| `FT2.DOC` | XM | FastTracker 2 documentation |
| `mod.txt` | MOD | ProTracker format specification |
| `s3m.txt` | S3M | Scream Tracker 3 format specification |
| `ITTECH.TXT`, `IT.TXT` | IT | Impulse Tracker format specification |
| `mtm.txt`, `mtm-efx.txt` | MTM | MultiTracker format specification |

### Reference Implementations (BSD-3-Clause)
- **[ft2-clone](https://github.com/8bitbubsy/ft2-clone)** — Definitive FastTracker 2 clone by 8bitbubsy
- **[libopenmpt](https://github.com/OpenMPT/openmpt)** — Module player library with comprehensive test suites

## Roadmap

### Phase 1: Build System Modernization
- Remove use of `#![feature(seek_stream_len)]` (replace with a stable alternative)
- Remove `cargo-features = ["named-profiles"]` (stabilized since Rust 1.57)
- Fix/verify `edition` settings across all crates
- Add `modplayer-lib` to workspace members
- Ensure `cargo build` works cleanly for all targets (native, WASM, C library)

### Phase 2: WASM Build Streamlining
- Create a Makefile that orchestrates `wasm-pack build` → `npm install` → `npm start`
- Update webpack configuration and npm dependencies to current versions
- Consider migrating from `ScriptProcessorNode` to `AudioWorklet` for better performance

### Phase 3: WASM Display Fixes
- Fix character alignment and row rendering in the `wglt` canvas terminal
- Fix pattern view display issues (row counting, boundary conditions)
- Ensure proper screen clearing between songs

### Phase 4: Cubic Interpolation
- Implement the commented-out cubic spline interpolation in the sample mixer
- Wire it up as a selectable filter option (none / linear / cubic)
- Add UI controls to switch between interpolation modes

### Phase 5: Testing Framework
- Unit tests for format parsers (verify headers, instruments, sample counts)
- Unit tests for effect logic (volume slides, envelopes, frequency calculations)
- Golden file comparison (render to buffer, compare against saved reference output)
- Property/invariant tests (volume bounds, song completion, silence on pause)
- Regression tests with OpenMPT test modules (BSD-3-Clause)

### Phase 6: IT Format Support (Major)
- Complete IT sample reading (compressed samples, stereo samples)
- Implement IT pattern decompression
- Add IT-specific effects (new note actions, duplicate check, filter envelopes, etc.)
- Handle IT's different channel/instrument architecture vs. XM

## License

MIT

## Author

Gil-Ad Ben Or (gilad.benor@gmail.com)
