# modplayer

A module music player written in Rust with terminal and WebAssembly backends.

[**Live Demo**](https://diffuse2000.github.io/rust-modplayer/)

## 🌟 Overview

**modplayer** is a module player that provides a way to listen to classic tracker music in the browser and the command line. It features a custom terminal-inspired interface with real-time channel visualization and basic effect support.

---

## 🚀 Features

### 🎮 Web Interface (`modplayer-wasm`)
- **Terminal UI**: A responsive, canvas-based terminal display with multiple color themes (**Pro**, **Cyberpunk**, **Obsidian**, **Monochrome**).
- **Navigation**: Horizontal scrolling (Arrow keys / Mouse Wheel) for viewing wide track layouts.
- **Visualizers**: Simple real-time oscilloscope and spectrum analyzer views.
- **Easy Loading**: Drag-and-drop support for local module files.

### 🎧 Audio Engine (`xmplayer`)
- **Effect Handling**: Support for standard tracker effects including volume slides, portamento, vibrato, tremolo, and volume/panning envelopes.
- **Pitch Accuracy**: Uses precomputed Amiga and Linear period tables for faithful reproduction.
- **WASM Optimized**: Low-latency mixing engine compiled to WebAssembly for browser-based playback.

---

## 🎹 Supported Formats

| Format | Status | Notes |
|--------|--------|-------|
| **XM** (FastTracker II) | ✅ Supported | Primary format; most stable and thoroughly tested |
| **MOD** (ProTracker/Amiga) | ✅ Supported | Classic 4-channel and expanded MOD support |
| **S3M** (Scream Tracker 3) | ✅ Supported | Functional playback via internal conversion to XM logic |
| **STM** (Scream Tracker 2) | ✅ Supported | Basic parsing and playback |
| **IT** (Impulse Tracker) | 🚧 WIP | Parsing implemented; pattern mixing in development |

---

## 🛠️ Build & Usage

### Web (WASM)
1. `cd modplayer-wasm && wasm-pack build --target bundler`
2. `cd www && npm install && npm run start`
3. Open `http://localhost:8080`.

### Native (CLI)
Requires SDL2:
```bash
cargo run --release -p modplayer-bin -- <module_file.xm>
```

---

## 🧪 Testing
Includes a regression suite with **OpenMPT** test modules and automated golden renders to ensure playback stability.

---

## 📄 License & Credits

- **License**: MIT
- **Reference Logic**: Some engine logic and loader implementations were informed by the [ft2-clone](https://github.com/8bitbubsy/ft2-clone) project. Testing modules and framework helped by [libopenmpt](https://github.com/OpenMPT/openmpt).

---
Built by [Gil-Ad Ben Or](https://github.com/Gil-AdB)
