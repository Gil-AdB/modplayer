
#[cfg(feature = "wasm")]
use wasm_bindgen::prelude::*;

#[cfg_attr(feature = "wasm", wasm_bindgen)]
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RGB {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[cfg_attr(feature = "wasm", wasm_bindgen)]
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Cell {
    pub c: char,
    pub fg: RGB,
    pub bg: RGB,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            c: ' ',
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
            self.cells[y * self.width + x] = Cell { c, fg, bg };
        }
    }

    pub fn merge_braille_cell(&mut self, x: usize, y: usize, dot_bit: u8, fg: RGB, bg: RGB) {
        if x < self.width && y < self.height {
            let idx = y * self.width + x;
            let current_c = self.cells[idx].c;
            let current_bits = if (0x2800..=0x28FF).contains(&(current_c as u32)) {
                (current_c as u32 - 0x2800) as u8
            } else {
                0
            };
            let new_bits = current_bits | dot_bit;
            let new_c = unsafe { std::char::from_u32_unchecked(0x2800 + new_bits as u32) };
            self.cells[idx] = Cell { c: new_c, fg, bg };
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
                result.push(cell.c);
            }
            // result.push('\n');
        }
        result.push_str("\x1b[0m");
        result
    }

    pub fn to_binary(&self) -> Vec<u8> {
        let mut result = Vec::with_capacity(self.cells.len() * 7);
        for cell in &self.cells {
            result.push(cell.c as u8);
            result.push(cell.fg.r);
            result.push(cell.fg.g);
            result.push(cell.fg.b);
            result.push(cell.bg.r);
            result.push(cell.bg.g);
            result.push(cell.bg.b);
        }
        result
    }
}
