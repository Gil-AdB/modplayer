use serde::Serialize;
use std::cmp::{max, min};
use xmplayer::song::PlayData;
use xmplayer::instrument::Instrument;
use xmplayer::module_reader::Patterns;

use wasm_bindgen::prelude::*;

#[derive(PartialEq, Eq, Copy, Clone)]
pub enum ViewMode {
    Pattern = 0,
    Instruments = 1,
    Message = 2,
    Help = 3,
}

impl From<u32> for ViewMode {
    fn from(v: u32) -> Self {
        match v {
            0 => ViewMode::Pattern,
            1 => ViewMode::Instruments,
            2 => ViewMode::Message,
            3 => ViewMode::Help,
            _ => ViewMode::Pattern,
        }
    }
}

#[derive(Debug)]
pub struct ViewPort {
    pub x1: isize,
    pub y1: isize,
    pub width: usize,
    pub height: usize,
}

#[wasm_bindgen]
#[derive(PartialEq, Eq, Copy, Clone, Serialize, Debug)]
pub struct RGB {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Clone, Serialize, Debug)]
pub struct Cell {
    pub c: char,
    pub fg: RGB,
    pub bg: RGB,
}

#[derive(Serialize, Clone)]
pub struct Line {
    pub cells:   Vec<Cell>,
}

impl Line {
    fn new(width: usize, background: RGB) -> Self {
        Self { 
            cells: vec![Cell { 
                c: ' ', 
                fg: RGB { r: 180, g: 180, b: 180 }, 
                bg: background 
            }; width] 
        }
    }

    fn from_string(s: &str, fg: RGB, bg: RGB, width: usize) -> Self {
        let mut cells = vec![];
        for (i, c) in s.chars().enumerate() {
            if i >= width { break; }
            cells.push(Cell { c, fg, bg });
        }
        while cells.len() < width {
            cells.push(Cell { c: ' ', fg, bg });
        }
        Self { cells }
    }
}

#[derive(Serialize)]
pub struct VirtualScreen {
    pub lines:      Vec<Line>,
    pub width:      usize,
}

impl VirtualScreen {
    fn new(width: usize) -> Self {
        VirtualScreen {
            lines: vec![],
            width,
        }
    }

    pub fn to_binary(&self) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(self.lines.len() * self.width * 7);
        for line in &self.lines {
            for cell in &line.cells {
                buffer.push(cell.c as u8);
                buffer.push(cell.fg.r);
                buffer.push(cell.fg.g);
                buffer.push(cell.fg.b);
                buffer.push(cell.bg.r);
                buffer.push(cell.bg.g);
                buffer.push(cell.bg.b);
            }
        }
        buffer
    }

    fn add_line(&mut self, data: String) {
        self.lines.push(Line::from_string(&data, RGB { r: 180, g: 180, b: 180 }, RGB { r: 0, g: 0, b: 0 }, self.width));
    }

    fn add_line_with_color(&mut self, data: String, background: RGB) {
        self.lines.push(Line::from_string(&data, RGB { r: 255, g: 255, b: 255 }, background, self.width));
    }

    fn add_cells(&mut self, cells: Vec<Cell>) {
        let mut line_cells = cells;
        while line_cells.len() < self.width {
            line_cells.push(Cell { c: ' ', fg: RGB { r: 180, g: 180, b: 180 }, bg: RGB { r: 0, g: 0, b: 0 } });
        }
        if line_cells.len() > self.width {
            line_cells.truncate(self.width);
        }
        self.lines.push(Line { cells: line_cells });
    }
}

pub struct Display {}

impl Display {
    fn fixed_width(s: &str, width: usize) -> String {
        let text = s.trim();
        let mut result = text.chars().take(width).collect::<String>();
        while result.chars().count() < width {
            result.push(' ');
        }
        result
    }

    fn color(color: RGB, str: &str) -> Vec<Cell> {
        str.chars().map(|c| Cell { c, fg: color, bg: RGB { r: 0, g: 0, b: 0 } }).collect()
    }

    pub fn hide() -> String {
        "".to_string()
    }

    pub fn show() -> String {
        "".to_string()
    }

    pub fn move_to(_x: usize, _y:usize) -> String {
        "".to_string()
    }

    pub fn clear() -> String {
        "".to_string()
    }

    fn range(pos: u32, start: u32, end: u32, width: usize) -> Vec<Cell> {
        let mut cells = vec![];
        let mut indicator_pos = if end == start { 0 } else {
            ((pos - start) as f32 / (end - start) as f32 * (width) as f32) as usize
        };
        if indicator_pos > width {
            indicator_pos = width;
        }
        let fg = RGB { r: 150, g: 150, b: 150 };
        let bg = RGB { r: 0, g: 0, b: 0 };
        for _ in 0..indicator_pos {
            cells.push(Cell { c: '-', fg, bg });
        }
        if indicator_pos < width {
            cells.push(Cell { c: '=', fg: RGB { r: 255, g: 255, b: 255 }, bg });
            for _ in indicator_pos + 1..width {
                cells.push(Cell { c: '-', fg, bg });
            }
        }
        cells
    }

    fn range_with_color(pos: u32, start: u32, end: u32, width: usize, colors: &[RGB]) -> Vec<Cell> {
        let mut cells = vec![];
        if pos == 0 {
            for _ in 0..width {
                cells.push(Cell { c: ' ', fg: colors[0], bg: RGB { r: 0, g: 0, b: 0 } });
            }
            return cells;
        }

        let mut indicator_pos = if end == start { 0 } else {
            ((pos - start) as f32 / (end - start) as f32 * (width) as f32) as usize
        };
        if indicator_pos >= width {
            indicator_pos = width - 1;
        }
        for i in 0..=indicator_pos {
            cells.push(Cell { c: '=', fg: colors[i % colors.len()], bg: RGB { r: 0, g: 0, b: 0 } });
        }
        for _ in indicator_pos + 1..width {
            cells.push(Cell { c: ' ', fg: colors[0], bg: RGB { r: 0, g: 0, b: 0 } });
        }
        cells
    }

    pub fn display(play_data: &PlayData, instruments: &Vec<Instrument>, patterns: &Vec<Patterns>,
                   order: &Vec<u8>, view_port: ViewPort, view_mode_raw: u32, theme_id: u32, scroll_offset: isize, panning_mode: u32) -> VirtualScreen {

        let mut lines_buffer = VirtualScreen::new(view_port.width);
        let view_mode = ViewMode::from(view_mode_raw);
        
        let (colors, header_bg, accent_fg) = match theme_id {
            1 => { // Cyberpunk
                ([
                    RGB { r: 255, g: 255, b: 0 }, RGB { r: 255, g: 200, b: 0 }, RGB { r: 255, g: 150, b: 0 },
                    RGB { r: 255, g: 100, b: 0 }, RGB { r: 255, g: 50, b: 255 }, RGB { r: 255, g: 0, b: 255 },
                    RGB { r: 200, g: 0, b: 255 }, RGB { r: 150, g: 0, b: 255 }, RGB { r: 100, g: 0, b: 255 },
                    RGB { r: 50, g: 0, b: 255 }, RGB { r: 0, g: 255, b: 255 }, RGB { r: 0, g: 200, b: 255 },
                ], RGB { r: 120, g: 120, b: 0 }, RGB { r: 255, g: 255, b: 0 })
            },
            2 => { // Obsidian Pro
                ([
                    RGB { r: 255, g: 120, b: 0 }, RGB { r: 255, g: 100, b: 0 }, RGB { r: 220, g: 80, b: 0 },
                    RGB { r: 200, g: 60, b: 0 }, RGB { r: 180, g: 40, b: 0 }, RGB { r: 160, g: 30, b: 0 },
                    RGB { r: 140, g: 20, b: 0 }, RGB { r: 120, g: 10, b: 0 }, RGB { r: 100, g: 5, b: 0 },
                    RGB { r: 80, g: 0, b: 0 }, RGB { r: 60, g: 60, b: 60 }, RGB { r: 40, g: 40, b: 40 },
                ], RGB { r: 60, g: 30, b: 0 }, RGB { r: 255, g: 140, b: 0 })
            },
            3 => { // Monochrome Pro
                ([
                    RGB { r: 0, g: 255, b: 0 }, RGB { r: 0, g: 220, b: 0 }, RGB { r: 0, g: 190, b: 0 },
                    RGB { r: 0, g: 160, b: 0 }, RGB { r: 0, g: 130, b: 0 }, RGB { r: 0, g: 100, b: 0 },
                    RGB { r: 0, g: 80, b: 0 }, RGB { r: 0, g: 60, b: 0 }, RGB { r: 0, g: 40, b: 0 },
                    RGB { r: 0, g: 30, b: 0 }, RGB { r: 0, g: 255, b: 100 }, RGB { r: 0, g: 200, b: 80 },
                ], RGB { r: 0, g: 60, b: 0 }, RGB { r: 0, g: 200, b: 0 })
            },
            _ => { // Pro Tracker (Default)
                ([
                    RGB { r: 50, g: 255, b: 50 }, RGB { r: 70, g: 255, b: 70 }, RGB { r: 90, g: 255, b: 90 },
                    RGB { r: 120, g: 255, b: 120 }, RGB { r: 255, g: 255, b: 50 }, RGB { r: 255, g: 255, b: 100 },
                    RGB { r: 255, g: 200, b: 50 }, RGB { r: 255, g: 150, b: 50 }, RGB { r: 255, g: 100, b: 50 },
                    RGB { r: 255, g: 50, b: 50 }, RGB { r: 255, g: 0, b: 0 }, RGB { r: 200, g: 0, b: 0 },
                ], RGB { r: 0, g: 0, b: 120 }, RGB { r: 0, g: 242, b: 254 })
            }
        };

        // Global Header
        let header = format!("NAME: {:26} POS: {:02X}/{:02X} ROW: {:02X}/{:02X} BPM: {:3} SPD: {:2} FILT: {:7} [F1-F4: VIEWS | F4: HELP]", 
            Self::fixed_width(&play_data.name, 26), 
            play_data.song_position, play_data.song_length.saturating_sub(1), 
            play_data.row, play_data.pattern_len.saturating_sub(1), 
            play_data.bpm, play_data.speed, format!("{:?}", play_data.filter)
        );
        lines_buffer.add_line_with_color(header, header_bg);

        match view_mode {
            ViewMode::Pattern => Self::render_pattern(&mut lines_buffer, play_data, instruments, patterns, order, &colors, accent_fg, panning_mode),
            ViewMode::Instruments => Self::render_instruments(&mut lines_buffer, instruments),
            ViewMode::Message => Self::render_message(&mut lines_buffer, &play_data.song_message),
            ViewMode::Help => Self::render_help(&mut lines_buffer),
        }

        // Viewport Application & Padding
        let mut screen = VirtualScreen::new(view_port.width);
        
        // Always include the Header (Line 0)
        if !lines_buffer.lines.is_empty() {
            screen.lines.push(lines_buffer.lines[0].clone());
        }

        // Apply scroll offset (y1) to the Rest of the lines
        let scroll_start = (view_port.y1.max(0) as usize).min(lines_buffer.lines.len().saturating_sub(1));
        let data_lines = &lines_buffer.lines[1..]; // Everything after the header
        
        let mut count = 1;
        for line in data_lines.iter().skip(scroll_start) {
            if count >= view_port.height { break; }
            screen.lines.push(line.clone());
            count += 1;
        }

        // Pad to exactly viewport height to clear previous content
        while screen.lines.len() < view_port.height {
            screen.lines.push(Line::new(view_port.width, RGB { r: 0, g: 0, b: 0 }));
        }

        screen
    }

    fn render_pattern(screen: &mut VirtualScreen, play_data: &PlayData, instruments: &Vec<Instrument>, patterns: &Vec<Patterns>, order: &Vec<u8>, colors: &[RGB], accent_fg: RGB, panning_mode: u32) {
        screen.add_line("STAT| CH | INSTRUMENT                  | FREQ   | VOLUME      | POSITION       | NOTE | PERD | CHAN VOL   | ENV VOL    | GLO VOL    | FADEOUT    | PANNING  |".to_string());
        
        let mut idx = 0;
        for channel in &play_data.channel_status {
            idx += 1;
            let mut cells = vec![];
            if channel.on {
                let final_vol = (channel.volume / 64.0) * (channel.envelope_volume / 16384.0) * (channel.global_volume / 64.0) * (channel.fadeout_volume / 65536.0);
                
                cells.extend(Self::color(RGB { r: 0, g: 255, b: 0 }, if channel.force_off { "X   " } else { "ON  " }));
                cells.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, &format!("| {:2} | ", idx)));
                cells.extend(Self::color(accent_fg, &Self::fixed_width(&channel.instrument_name, 26)));
                cells.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, &format!(" | {:6} | ", channel.frequency as u32)));
                cells.extend(Self::range_with_color((final_vol * 12.0).ceil() as u32, 0, 12, 11, colors));
                cells.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, " | "));
                cells.extend(Self::range(channel.sample_position as u32, 0, max(instruments[channel.instrument].samples[channel.sample].length, 1) - 1, 14));
                cells.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, &format!(" | {:4} | {:4} | ", channel.note, channel.period)));
                cells.extend(Self::range_with_color(channel.volume as u32, 0, 64, 10, colors));
                cells.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, " | "));
                cells.extend(Self::range_with_color(channel.envelope_volume as u32, 0, 16384, 10, colors));
                cells.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, " | "));
                cells.extend(Self::range_with_color(channel.global_volume as u32, 0, 64, 10, colors));
                cells.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, " | "));
                cells.extend(Self::range_with_color(channel.fadeout_volume as u32, 0, 65536, 10, colors));
                cells.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, " | "));
                if panning_mode == 1 {
                    cells.extend(Self::panning_bar(channel.final_panning as u32, 10));
                } else {
                    cells.extend(Self::color(RGB { r: 150, g: 150, b: 255 }, &format!("  {:3}     ", channel.final_panning as u32)));
                }
                cells.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, " |"));
            } else {
                cells.extend(Self::color(RGB { r: 100, g: 100, b: 100 }, &format!("OFF | {:2} | {:26} |        |             |                |      |      |            |            |            |            |          |", idx, "")));
            }
            screen.add_cells(cells);
        }

        screen.add_line("".to_string());

        if play_data.song_position < play_data.song_length as usize && order[play_data.song_position] < patterns.len() as u8 {
            let pattern = &patterns[order[play_data.song_position] as usize];
            let first_row = play_data.row.saturating_sub(10);
            let last_row = min(first_row + 21, pattern.rows.len());

            for i in first_row..last_row {
                let mut cells = vec![];
                let row_fg = if i == play_data.row { RGB { r: 255, g: 255, b: 255 } } else { RGB { r: 150, g: 150, b: 150 } };
                let row_bg = if i == play_data.row { RGB { r: 40, g: 40, b: 100 } } else { RGB { r: 0, g: 0, b: 0 } };

                let row_num = format!("{:02X} | ", i);
                cells.extend(row_num.chars().map(|c| Cell { c, fg: RGB { r: 255, g: 255, b: 100 }, bg: row_bg }));

                for p in pattern.rows[i].channels.iter() {
                    let note = if p.note == 0 { "---".to_string() } else if p.note == 97 { "OFF".to_string() } else {
                        format!("{}{}", xmplayer::pattern::Pattern::NOTES[((p.note - 1) % 12) as usize], (p.note - 1) / 12)
                    };
                    let inst = if p.instrument == 0 { "..".to_string() } else { format!("{:02X}", p.instrument) };
                    let vol = if p.volume == 0 { "..".to_string() } else { format!("{:02X}", p.volume) };
                    let eff = if p.effect == 0 && p.effect_param == 0 { "..." .to_string() } else { format!("{:X}{:02X}", p.effect, p.effect_param) };
                    
                    // Note: Cyan/Accent
                    cells.extend(note.chars().map(|c| Cell { c, fg: if i == play_data.row { row_fg } else { accent_fg }, bg: row_bg }));
                    cells.push(Cell { c: ' ', fg: row_fg, bg: row_bg });
                    // Instrument: Green
                    cells.extend(inst.chars().map(|c| Cell { c, fg: if i == play_data.row { row_fg } else { RGB { r: 0, g: 255, b: 0 } }, bg: row_bg }));
                    cells.push(Cell { c: ' ', fg: row_fg, bg: row_bg });
                    // Volume: Yellow
                    cells.extend(vol.chars().map(|c| Cell { c, fg: if i == play_data.row { row_fg } else { RGB { r: 255, g: 255, b: 0 } }, bg: row_bg }));
                    cells.push(Cell { c: ' ', fg: row_fg, bg: row_bg });
                    // Effect: Purple
                    cells.extend(eff.chars().map(|c| Cell { c, fg: if i == play_data.row { row_fg } else { RGB { r: 180, g: 100, b: 255 } }, bg: row_bg }));
                    cells.extend(" | ".chars().map(|c| Cell { c, fg: row_fg, bg: row_bg }));
                }
                screen.add_cells(cells);
            }
        }
    }

    fn render_instruments(screen: &mut VirtualScreen, instruments: &Vec<Instrument>) {
        screen.add_line("".to_string());
        screen.add_line("--- INSTRUMENT LIST ---".to_string());
        screen.add_line("".to_string());
        for (i, inst) in instruments.iter().enumerate() {
            if i == 0 || inst.name.trim().is_empty() { continue; }
            screen.add_line(format!("{:02X}: {}", i, Self::fixed_width(&inst.name, 40)));
        }
    }

    fn render_message(screen: &mut VirtualScreen, message: &str) {
        screen.add_line("".to_string());
        screen.add_line("--- SONG MESSAGE ---".to_string());
        screen.add_line("".to_string());
        for line in message.lines() {
            screen.add_line(line.to_string());
        }
    }

    fn render_help(screen: &mut VirtualScreen) {
        screen.add_line("".to_string());
        screen.add_line("--- KEYBOARD SHORTCUTS ---".to_string());
        screen.add_line("".to_string());
        screen.add_line("F1: Pattern View        Space: Play/Pause".to_string());
        screen.add_line("F2: Instrument View     [ / ]: Prev/Next Pattern".to_string());
        screen.add_line("F3: Song Message        + / -: Inc/Dec Speed".to_string());
        screen.add_line("F4: Help Screen         . / ,: Inc/Dec BPM".to_string());
        screen.add_line("".to_string());
        screen.add_line("O: Global Oscilloscope  Shift+O: Per-Channel Scope".to_string());
        screen.add_line("Shift+P: Toggle Panning  Escape: Quit".to_string());
        screen.add_line("0-9: Mute/Solo Groups".to_string());
    }

    fn panning_bar(panning: u32, width: usize) -> Vec<Cell> {
        let pos = (panning as f32 / 255.0 * (width - 3) as f32) as usize;
        let s = format!("[{}]", (0..width-2).map(|i| if i == pos { '=' } else { ' ' }).collect::<String>());
        Self::color(RGB { r: 150, g: 150, b: 255 }, &s)
    }
}
