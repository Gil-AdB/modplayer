# UI & Visualization Skill

This skill provides the necessary context and rules for maintaining and debugging the Modplayer's visualization engine across terminal and web platforms.

## Terminal Rendering Logic
- **Braille Encoding**: When updating scopes/plots, use the `Unicode Braille` block (U+2800). Each character represents a 2x4 dot matrix.
- **ANSI Efficiency**: The `Grid::to_ansi` method uses color caching. Always ensure that `last_fg` and `last_bg` are reset if the rendering context changes outside of the grid's control.
- **UTF-8 Requirement**: Ensure the output stream supports UTF-8, as Braille characters are 3-byte sequences.

## Web/WASM Rendering Logic
- **Zero-Copy Access**: The web frontend accesses Rust's `Vec<Cell>` directly. 
- **Offset Math**: The stride is **12 bytes**.
  - `charCode`: `gridData[offset]` (4 bytes, but usually ASCII in first byte)
  - `fgColor`: `wglt.fromRgb(gridData[offset+4], gridData[offset+5], gridData[offset+6])`
  - `bgColor`: `wglt.fromRgb(gridData[offset+7], gridData[offset+8], gridData[offset+9])`
- **wglt API Rules**:
  - **DO**: Use `cell.setValue(char, fg, bg)`.
  - **DO**: Use `wglt.fromRgb(r, g, b)`.
  - **DON'T**: Call `term.setCell` or `cell.setChar` (these are non-existent).

## Common UI Fixes
1. **TypeError/ReferenceError**: Usually a hallucinated wglt method or a missing variable definition in the animation loop (`index.js`).
2. **Shifted Display**: Check the `offset` stride (12 bytes). If the struct `Cell` is modified in `grid.rs`, the JS offset must be updated.
3. **404 test.mod**: If the player fails to start, ensure a local `test.mod` exists in the `www/` directory or update the default load path in `index.js`.
