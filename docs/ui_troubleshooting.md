# Display Architecture & Cross-Platform Implementation

This document describes the unified display architecture of the Modplayer project and how it bifurcates between Native (Terminal) and Web (WASM) targets.

## 1. Unified Core: The `Grid` Data Structure

At the heart of both displays is the `Grid` and `Cell` structure located in `display/src/grid.rs`.

```rust
#[repr(C)]
pub struct Cell {
    pub c: u32,   // Unicode character code (4 bytes)
    pub fg: RGB,  // Foreground color (3 bytes: R, G, B)
    pub bg: RGB,  // Background color (3 bytes: R, G, B)
}
```

- **Abstraction**: The entire player logic (song position, pattern data, scopes, spectrum) is rendered into this abstract `Grid` of `Cells`.
- **Target Agnostic**: Rust doesn't care if it's drawing to a real terminal or a WASM memory buffer; it simply manipulates this grid.

## 2. Dispatch Mechanism: `Display::render`

The `Display::render` function in `display/src/display/mod.rs` acts as the orchestrator. It receives a mutable reference to a `Grid` and a `TargetPlatform` flag.

```rust
pub enum TargetPlatform {
    Native = 0,
    WASM = 1,
}
```

### Key Differences in Dispatch:
1.  **Vertical Space Allocation**:
    - **Native**: Calculates `vis_height` (visualizer height) based on actual terminal dimensions and partitions the screen between the pattern tracker and the braille visualizers.
    - **WASM**: Often assumes a fixed resolution or leaves complex resizing to the JavaScript host.
2.  **Visualizer Logic**:
    - **Native**: Calls `render_fft`, `render_master_scope`, or `render_multi_scope` using Braille characters.
    - **WASM**: While the WASM side *can* render to the grid, the Web UI often uses the canvas visualizers in parallel for higher resolution, making the `Grid` primarily responsible for the tracker text.

## 3. The Implementation Split

### Terminal-Side (Native)
After `Display::render` fills the `Grid`, the native binary calls `grid.to_ansi()`.
- **ANSI Translation**: Converts the `Cell` array into a large string containing ANSI escape codes for 24-bit color (`\x1b[38;2;R;G;Bm`).
- **Color Caching**: To minimize serial data, `to_ansi` only emits color codes when they change from the previous cell.
- **Output**: The string is written to `stdout` using `crossterm`.

### Web-Side (WASM/JS)
The Web UI takes a "Zero-Copy" approach for maximum performance.
- **Shared Memory**: The Rust `Grid` is stored in a `Vec<Cell>`. Through `wasm-bindgen`, the JavaScript side receives a pointer to this contiguous memory block.
- **JS-Driven Polling**: The browser's `requestAnimationFrame` loop calls into WASM to run the render logic, then immediately reads the `Cell` array from the WASM heap.
- **Rendering**:
  - **Terminal Overlay**: JS iterates over the byte buffer, reconstructs the `u32` character codes, and updates the `wglt` WebGL terminal.
  - **Canvas Visualizers**: The `audio-worklet.js` and `analyzerNode` handle the high-speed oscilloscope and FFT separately from the text grid for smoother 60fps performance without clogging the main JS thread.

## 4. Architectural Summary

| Feature | Native (Terminal) | Web (WASM/JS) |
| :--- | :--- | :--- |
| **Grid Processing** | `Display::render` (Rust) | `Display::render` (Rust) |
| **Output Format** | ANSI String (24-bit color) | Binary Shared Memory (12-byte stride) |
| **Visualizers** | Unicode Braille (in-grid) | WebGL Grid + HTML5 Canvas |
| **Sync Primitive** | Mutex/TripleBuffer (Local) | Atomic SharedArrayBuffer (threaded) |
| **Performance constraint** | Serial Baud/TTY throughput | Main Thread Latency |

## 5. Maintenance Note
When updating the UI, always update `Display::render` first. If you add a field to `Cell`, you **must** update the 12-byte stride calculation in `modplayer-wasm/www/index.js` to prevent the display from shifting or "melting".
