
#[cfg(feature = "wasm")]
use wasm_bindgen::prelude::*;

#[cfg_attr(feature = "wasm", wasm_bindgen)]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(C)]
pub struct RGB {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[cfg_attr(feature = "wasm", wasm_bindgen)]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(C)]
pub struct Cell {
    pub c: u32,
    pub fg: RGB,
    pub bg: RGB,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            c: ' ' as u32,
            fg: RGB { r: 255, g: 255, b: 255 },
            bg: RGB { r: 0, g: 0, b: 0 },
        }
    }
}

pub struct Grid {
    pub width:  usize,
    pub height: usize,
    pub cells:  Vec<Cell>,
}

impl Grid {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            cells: vec![Cell::default(); width * height],
        }
    }

    pub fn set_cell(&mut self, x: usize, y: usize, c: char, fg: RGB, bg: RGB) {
        if x < self.width && y < self.height {
            self.cells[y * self.width + x] = Cell { c: c as u32, fg, bg };
        }
    }

    pub fn merge_braille_cell(&mut self, x: usize, y: usize, dot_bit: u8, fg: RGB, bg: RGB) {
        if x < self.width && y < self.height {
            let idx = y * self.width + x;
            let current_c = self.cells[idx].c;
            let current_bits = if (0x2800..=0x28FF).contains(&current_c) {
                (current_c - 0x2800) as u8
            } else {
                0
            };
            let new_bits = current_bits | dot_bit;
            self.cells[idx] = Cell { c: 0x2800 + new_bits as u32, fg, bg };
        }
    }

    pub fn print(&mut self, x: usize, y: usize, str: &str, fg: RGB, bg: RGB) {
        let mut curr_x = x;
        for c in str.chars() {
            if curr_x >= self.width { break; }
            self.set_cell(curr_x, y, c, fg, bg);
            curr_x += 1;
        }
    }

    pub fn to_ansi(&self) -> String {
        let mut result = String::new();
        let mut last_fg = None;
        let mut last_bg = None;

        for y in 0..self.height {
            result.push_str(&format!("\x1b[{};1H", y + 1));
            for x in 0..self.width {
                let cell = self.cells[y * self.width + x];
                if Some(cell.fg) != last_fg || Some(cell.bg) != last_bg {
                    result.push_str(&format!("\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m", 
                        cell.fg.r, cell.fg.g, cell.fg.b,
                        cell.bg.r, cell.bg.g, cell.bg.b
                    ));
                    last_fg = Some(cell.fg);
                    last_bg = Some(cell.bg);
                }
                result.push(std::char::from_u32(cell.c).unwrap_or(' '));
            }
        }
        result.push_str("\x1b[0m");
        result
    }
}
