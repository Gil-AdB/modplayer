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

    fn add_separator(&mut self) {
        if self.lines.is_empty() { return; }
        let mut cells = vec![];
        let last_line = self.lines.last().unwrap();
        
        let last_pipe_idx = last_line.cells.iter().rposition(|c| c.c == '|').unwrap_or(0);

        for i in 0..self.width {
            if i <= last_pipe_idx {
                let c = if i < last_line.cells.len() && last_line.cells[i].c == '|' { '+' } else { '-' };
                cells.push(Cell { c, fg: RGB { r: 255, g: 255, b: 255 }, bg: RGB { r: 0, g: 0, b: 0 } });
            } else {
                cells.push(Cell { c: ' ', fg: RGB { r: 0, g: 0, b: 0 }, bg: RGB { r: 0, g: 0, b: 0 } });
            }
        }
        self.lines.push(Line { cells });
    }

    fn add_line_with_cells(&mut self, cells: Vec<Cell>) {
        let mut line_cells = cells;
        if line_cells.len() > self.width {
            line_cells.truncate(self.width);
        }
        while line_cells.len() < self.width {
            line_cells.push(Cell { c: ' ', fg: RGB { r: 180, g: 180, b: 180 }, bg: RGB { r: 0, g: 0, b: 0 } });
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

    fn center_text(s: &str, width: usize) -> String {
        let text = s.trim();
        let len = text.chars().count();
        if len >= width {
            return text.chars().take(width).collect();
        }
        let left_pad = (width - len) / 2;
        let right_pad = width - len - left_pad;
        format!("{}{}{}", " ".repeat(left_pad), text, " ".repeat(right_pad))
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
        cells.push(Cell { c: '=', fg: RGB { r: 255, g: 255, b: 255 }, bg });
        for _ in indicator_pos + 1..(width + 1) {
            cells.push(Cell { c: '-', fg, bg });
        }
        cells
    }

    fn range_with_color(pos: u32, start: u32, end: u32, width: usize, colors: &[RGB]) -> Vec<Cell> {
        let mut cells = vec![];
        if pos == 0 {
            for _ in 0..(width + 1) {
                cells.push(Cell { c: '-', fg: RGB { r: 50, g: 50, b: 50 }, bg: RGB { r: 0, g: 0, b: 0 } });
            }
            return cells;
        }

        let mut indicator_pos = if end == start { 0 } else {
            ((pos - start) as f32 / (end - start) as f32 * (width) as f32) as usize
        };
        if indicator_pos > width {
            indicator_pos = width;
        }
        for i in 0..indicator_pos {
            cells.push(Cell { c: '=', fg: colors[std::cmp::min(i, colors.len() - 1)], bg: RGB { r: 0, g: 0, b: 0 } });
        }
        cells.push(Cell { c: '=', fg: colors[std::cmp::min(indicator_pos, colors.len() - 1)], bg: RGB { r: 0, g: 0, b: 0 } });
        for _ in indicator_pos + 1..(width + 1) {
            cells.push(Cell { c: '-', fg: RGB { r: 50, g: 50, b: 50 }, bg: RGB { r: 0, g: 0, b: 0 } });
        }
        cells
    }


    pub fn display(play_data: &PlayData, instruments: &Vec<Instrument>, patterns: &Vec<Patterns>,
                   order: &Vec<u8>, view_port: ViewPort, view_mode_raw: u32, theme_id: u32, _scroll_offset: isize, panning_mode: u32) -> VirtualScreen {

        let mut lines_buffer = VirtualScreen::new(1024);
        let view_mode = ViewMode::from(view_mode_raw);
        
        let (colors, header_bg, accent_fg, note_fg, inst_fg, vol_fg, eff_fg) = match theme_id {
            1 => { // Cyberpunk (Cyan to Purple)
                ([
                    RGB { r: 0, g: 242, b: 254 }, RGB { r: 11, g: 222, b: 254 }, RGB { r: 22, g: 202, b: 254 },
                    RGB { r: 33, g: 182, b: 254 }, RGB { r: 44, g: 162, b: 254 }, RGB { r: 55, g: 142, b: 255 },
                    RGB { r: 66, g: 122, b: 255 }, RGB { r: 77, g: 102, b: 255 }, RGB { r: 88, g: 82, b: 255 },
                    RGB { r: 99, g: 62, b: 255 }, RGB { r: 110, g: 42, b: 255 }, RGB { r: 123, g: 39, b: 255 },
                ], RGB { r: 60, g: 0, b: 100 }, RGB { r: 0, g: 242, b: 254 },
                RGB { r: 0, g: 242, b: 254 }, RGB { r: 0, g: 255, b: 80 }, RGB { r: 255, g: 255, b: 0 }, RGB { r: 123, g: 39, b: 255 })
            },
            2 => { // Obsidian Pro (Yellow to Magenta)
                ([
                    RGB { r: 255, g: 255, b: 0 }, RGB { r: 255, g: 231, b: 23 }, RGB { r: 255, g: 208, b: 46 },
                    RGB { r: 255, g: 185, b: 69 }, RGB { r: 255, g: 162, b: 92 }, RGB { r: 255, g: 139, b: 115 },
                    RGB { r: 255, g: 115, b: 139 }, RGB { r: 255, g: 92, b: 162 }, RGB { r: 255, g: 69, b: 185 },
                    RGB { r: 255, g: 46, b: 208 }, RGB { r: 255, g: 23, b: 231 }, RGB { r: 255, g: 0, b: 255 },
                ], RGB { r: 60, g: 30, b: 0 }, RGB { r: 255, g: 140, b: 0 },
                RGB { r: 255, g: 140, b: 0 }, RGB { r: 0, g: 200, b: 0 }, RGB { r: 255, g: 200, b: 0 }, RGB { r: 255, g: 0, b: 255 })
            },
            3 => { // Monochrome Pro (Orange to Dark Gray)
                ([
                    RGB { r: 255, g: 140, b: 0 }, RGB { r: 237, g: 133, b: 5 }, RGB { r: 220, g: 126, b: 11 },
                    RGB { r: 203, g: 119, b: 17 }, RGB { r: 185, g: 112, b: 23 }, RGB { r: 168, g: 105, b: 29 },
                    RGB { r: 151, g: 98, b: 34 }, RGB { r: 133, g: 91, b: 40 }, RGB { r: 116, g: 84, b: 46 },
                    RGB { r: 99, g: 77, b: 52 }, RGB { r: 81, g: 70, b: 58 }, RGB { r: 64, g: 64, b: 64 },
                ], RGB { r: 40, g: 40, b: 40 }, RGB { r: 200, g: 200, b: 200 },
                RGB { r: 200, g: 200, b: 200 }, RGB { r: 150, g: 150, b: 150 }, RGB { r: 120, g: 120, b: 120 }, RGB { r: 90, g: 90, b: 90 })
            },
            _ => { // Pro Tracker Default (Green to Yellow to Red)
                ([
                    RGB { r: 0, g: 255, b: 0 }, RGB { r: 51, g: 255, b: 0 }, RGB { r: 102, g: 255, b: 0 },
                    RGB { r: 153, g: 255, b: 0 }, RGB { r: 204, g: 255, b: 0 }, RGB { r: 255, g: 255, b: 0 },
                    RGB { r: 255, g: 204, b: 0 }, RGB { r: 255, g: 153, b: 0 }, RGB { r: 255, g: 102, b: 0 },
                    RGB { r: 255, g: 51, b: 0 }, RGB { r: 255, g: 0, b: 0 }, RGB { r: 255, g: 0, b: 0 },
                ], RGB { r: 0, g: 0, b: 120 }, RGB { r: 255, g: 255, b: 255 },
                RGB { r: 0, g: 242, b: 254 }, RGB { r: 0, g: 255, b: 0 }, RGB { r: 255, g: 255, b: 0 }, RGB { r: 255, g: 0, b: 0 })
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
            ViewMode::Pattern => Self::render_pattern(&mut lines_buffer, play_data, instruments, patterns, order, &colors, accent_fg, note_fg, inst_fg, vol_fg, eff_fg, panning_mode),
            ViewMode::Instruments => Self::render_instruments(&mut lines_buffer, instruments),
            ViewMode::Message => Self::render_message(&mut lines_buffer, &play_data.song_message),
            ViewMode::Help => Self::render_help(&mut lines_buffer),
        }

        // Viewport Application & Padding
        let mut screen = VirtualScreen::new(view_port.width);
        
        // Always include the Header (Line 0) as a fixed-width line (no horizontal scroll)
        if !lines_buffer.lines.is_empty() {
            let mut header_cells = lines_buffer.lines[0].cells.clone();
            if header_cells.len() > view_port.width {
                header_cells.truncate(view_port.width);
            }
            while header_cells.len() < view_port.width {
                header_cells.push(Cell { c: ' ', fg: RGB { r: 255, g: 255, b: 255 }, bg: header_bg });
            }
            screen.lines.push(Line { cells: header_cells });
        }

        // Apply scroll offset (y1) to the rest of the lines
        let scroll_start = (view_port.y1.max(0) as usize).min(lines_buffer.lines.len().saturating_sub(1));
        let x_start = view_port.x1.max(0) as usize;
        
        let mut count = 1;
        if let Some(data_lines) = lines_buffer.lines.get(1..) {
            for line in data_lines.iter().skip(scroll_start) {
                if count >= view_port.height { break; }
                
                let mut sliced_cells = vec![];
                for i in 0..view_port.width {
                    let idx = x_start + i;
                    if idx < line.cells.len() {
                        sliced_cells.push(line.cells[idx].clone());
                    } else {
                        sliced_cells.push(Cell { c: ' ', fg: RGB { r: 0, g: 0, b: 0 }, bg: RGB { r: 0, g: 0, b: 0 } });
                    }
                }
                screen.lines.push(Line { cells: sliced_cells });
                count += 1;
            }
        }

        // Pad to exactly viewport height to clear previous content
        while screen.lines.len() < view_port.height {
            screen.lines.push(Line::new(view_port.width, RGB { r: 0, g: 0, b: 0 }));
        }

        screen
    }

    fn render_pattern(screen: &mut VirtualScreen, play_data: &PlayData, instruments: &Vec<Instrument>, patterns: &Vec<Patterns>, order: &Vec<u8>, colors: &[RGB], accent_fg: RGB, note_fg: RGB, inst_fg: RGB, vol_fg: RGB, eff_fg: RGB, panning_mode: u32) {
        let mut header = vec![];
        header.extend(Self::color(RGB { r: 255, g: 255, b: 255 }, &Self::center_text("STAT", 4)));
        header.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, "| CH | "));
        header.extend(Self::color(RGB { r: 255, g: 255, b: 255 }, &Self::center_text("INSTRUMENT", 26)));
        header.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, " | "));
        header.extend(Self::color(RGB { r: 255, g: 255, b: 255 }, &Self::center_text("FREQ", 8)));
        header.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, "|"));
        header.extend(Self::color(RGB { r: 255, g: 255, b: 255 }, &Self::center_text("VOLUME", 12)));
        header.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, "|"));
        header.extend(Self::color(RGB { r: 255, g: 255, b: 255 }, &Self::center_text("POSITION", 15)));
        header.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, "|"));
        header.extend(Self::color(RGB { r: 255, g: 255, b: 255 }, &format!(" {} | {:7}", Self::center_text("NOTE", 4), Self::center_text("PERD", 7))));
        header.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, "|"));
        
        let labels = ["CHAN VOL", "ENVELOPE", "GLOBAL VOL", "FADEOUT"];
        for label in labels.iter() {
            header.extend(Self::color(RGB { r: 255, g: 255, b: 255 }, &Self::center_text(label, 12)));
            header.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, "|"));
        }
        header.extend(Self::color(RGB { r: 255, g: 255, b: 255 }, &Self::center_text("PANNING", 9)));
        header.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, "|"));
        screen.add_line_with_cells(header);

        // Separator line between header and data
        screen.add_separator();
        
        let mut idx = 0;
        for channel in &play_data.channel_status {
            idx += 1;
            let mut cells: Vec<Cell> = vec![];
            
            // 1. STAT (4)
            let status = if channel.on { if channel.force_off { "X   " } else { "ON  " } } else { "OFF " };
            cells.extend(Self::color(if channel.on { RGB { r: 0, g: 255, b: 0 } } else { RGB { r: 100, g: 100, b: 100 } }, status));
            
            // 2. CH (6)
            cells.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, &format!("| {:2} | ", idx)));
            
            // 3. INSTRUMENT (26)
            let inst_name = if channel.on { Self::fixed_width(&channel.instrument_name, 26) } else { " ".repeat(26) };
            cells.extend(Self::color(accent_fg, &inst_name));
            
            // 4. FREQ
            cells.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, " | "));
            let freq_str = if channel.on { format!("  {:<6}", channel.frequency as u32) } else { " ".repeat(8) };
            cells.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, &freq_str));
            
            // 5. VOLUME (12 header)
            cells.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, "|"));
            if channel.on {
                let final_vol = (channel.volume / 64.0) * (channel.envelope_volume / 16384.0) * (channel.global_volume / 64.0) * (channel.fadeout_volume / 65536.0);
                cells.extend(Self::range_with_color((final_vol * 11.0).ceil() as u32, 0, 11, 11, colors));
            } else {
                cells.extend(Self::color(RGB { r: 50, g: 50, b: 50 }, &" ".repeat(12)));
            }
            
            // 6. POSITION (15 header)
            cells.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, "|"));
            if channel.on {
                cells.extend(Self::range(channel.sample_position as u32, 0, max(instruments[channel.instrument].samples[channel.sample].length, 1) - 1, 14));
            } else {
                cells.extend(Self::color(RGB { r: 50, g: 50, b: 50 }, &" ".repeat(15)));
            }
            
            // 7. NOTE & PERD (15 header)
            cells.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, "|"));
            if channel.on {
                cells.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, &format!(" {:4} | {:7}", channel.note, channel.period)));
            } else {
                cells.extend(Self::color(RGB { r: 50, g: 50, b: 50 }, &format!(" {} | {}", " ".repeat(4), " ".repeat(7))));
            }
            
            // 8-11. VOLUMES (12 header each)
            let val_cols = [
                (channel.volume as u32, 64),
                (channel.envelope_volume as u32, 16384),
                (channel.global_volume as u32, 64),
                (channel.fadeout_volume as u32, 65536),
            ];
            for (val, max_val) in val_cols.iter() {
                cells.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, "|"));
                if channel.on {
                    cells.extend(Self::range_with_color(*val, 0, *max_val, 11, colors));
                } else {
                    cells.extend(Self::color(RGB { r: 50, g: 50, b: 50 }, &" ".repeat(12)));
                }
            }
            
            // 12. PANNING (9 header)
            cells.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, "|"));
            if channel.on {
                if panning_mode == 1 {
                    cells.extend(Self::range(channel.final_panning as u32, 0, 255, 8));
                } else {
                    cells.extend(Self::color(accent_fg, &format!("  {:3}    ", channel.final_panning as u32)));
                }
            } else {
                cells.extend(Self::color(RGB { r: 50, g: 50, b: 50 }, &" ".repeat(9)));
            }
            
            cells.extend(Self::color(RGB { r: 200, g: 200, b: 200 }, "|"));
            screen.add_cells(cells);
        }

        screen.add_line("".to_string());

        if play_data.song_position < play_data.song_length as usize && order[play_data.song_position] < patterns.len() as u8 {
            let pattern = &patterns[order[play_data.song_position] as usize];
            let first_row = play_data.row.saturating_sub(10);
            let last_row = min(first_row + 21, pattern.rows.len());

            for i in first_row..last_row {
                let mut cells: Vec<Cell> = vec![];
                let row_fg = if i == play_data.row { RGB { r: 255, g: 255, b: 255 } } else { RGB { r: 150, g: 150, b: 150 } };
                let row_bg = if i == play_data.row { RGB { r: 40, g: 40, b: 100 } } else { RGB { r: 0, g: 0, b: 0 } };

                let row_num = format!("{:02X} | ", i);
                cells.extend(row_num.chars().map(|c| Cell { c, fg: RGB { r: 255, g: 255, b: 100 }, bg: row_bg }));

                for ch_idx in 0..play_data.channel_status.len() {
                    let p_opt = pattern.rows[i].channels.get(ch_idx);
                    
                    let (note, inst, vol, eff) = if let Some(p) = p_opt {
                        let note = if p.note == 0 { "---".to_string() } else if p.note == 97 { "OFF".to_string() } else {
                            let octave = (p.note - 1) / 12;
                            format!("{}{}", xmplayer::pattern::Pattern::NOTES[((p.note - 1) % 12) as usize], if octave > 9 { 9 } else { octave })
                        };
                        let inst = if p.instrument == 0 { "..".to_string() } else { format!("{:02X}", p.instrument) };
                        let vol = if p.volume == 0 { "..".to_string() } else { format!("{:02X}", p.volume) };
                        let eff = if p.effect == 0 && p.effect_param == 0 { 
                            "..." .to_string() 
                        } else if p.effect < 16 {
                            format!("{:X}{:02X}", p.effect, p.effect_param)
                        } else {
                            format!("{}{:02X}", (b'A' + (p.effect - 10) as u8) as char, p.effect_param)
                        };
                        (note, inst, vol, eff)
                    } else {
                        ("---".to_string(), "..".to_string(), "..".to_string(), "...".to_string())
                    };
                    
                    cells.extend(note.chars().map(|c| Cell { c, fg: if i == play_data.row { row_fg } else { note_fg }, bg: row_bg }));
                    cells.push(Cell { c: ' ', fg: row_fg, bg: row_bg });
                    cells.extend(inst.chars().map(|c| Cell { c, fg: if i == play_data.row { row_fg } else { inst_fg }, bg: row_bg }));
                    cells.push(Cell { c: ' ', fg: row_fg, bg: row_bg });
                    cells.extend(vol.chars().map(|c| Cell { c, fg: if i == play_data.row { row_fg } else { vol_fg }, bg: row_bg }));
                    cells.push(Cell { c: ' ', fg: row_fg, bg: row_bg });
                    cells.extend(eff.chars().map(|c| Cell { c, fg: if i == play_data.row { row_fg } else { eff_fg }, bg: row_bg }));
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

}
