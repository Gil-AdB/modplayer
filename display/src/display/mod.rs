use crate::grid::{Grid, RGB};
use std::cmp::min;
use xmplayer::instrument::Instrument;
use xmplayer::module_reader::Patterns;
use xmplayer::song::{PlayData, ChannelStatus, UserData};

#[derive(PartialEq, Eq, Copy, Clone)]
pub enum TargetPlatform {
    Native = 0,
    WASM = 1,
}

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

pub struct ViewPort {
    pub x1: isize,
    pub y1: isize,
    pub width: usize,
    pub height: usize,
}

pub struct Theme {
    pub meter_colors: [RGB; 24],
    pub freq_colors: [RGB; 24],
    pub header_bg: RGB,
    pub header_fg: RGB,
    pub accent_fg: RGB,
    pub table_hdr_bg: RGB,
    pub table_hdr_fg: RGB,
    pub row_bg_odd: RGB,
    pub row_bg_even: RGB,
    pub col_on: RGB,
    pub col_off: RGB,
    pub col_inst: RGB,
    pub col_freq: RGB,
    pub col_note: RGB,
    pub col_period: RGB,
    pub col_sep: RGB,
    pub pat_row_bg: RGB,
    pub pat_curr_bg: RGB,
    pub pat_note_fg: RGB,
    pub pat_inst_fg: RGB,
    pub pat_vol_fg: RGB,
    pub pat_eff_fg: RGB,
}

pub struct Display {}

impl Display {
    pub fn render(
        grid: &mut Grid,
        play_data: &PlayData,
        instruments: &Vec<Instrument>,
        patterns: &Vec<Patterns>,
        order: &Vec<u8>,
        width: usize,
        height: usize,
        view_mode_raw: u32,
        theme_id: u32,
        x_offset: isize,
        _y_offset: isize,
        platform: TargetPlatform,
    ) {
        let view_mode = ViewMode::from(view_mode_raw);
        
        let theme_id = match play_data.user_data.get("theme_id") {
            Some(UserData::USize(v)) => (*v % 4) as u32,
            _ => theme_id % 4
        };
        let theme = Self::get_theme(theme_id);

        let visualizer_mode = match play_data.user_data.get("visualizer_mode") {
            Some(UserData::USize(v)) => (*v % 3) as u32,
            _ => play_data.visualizer_mode % 3
        };

        // Pre-fill with background color to eliminate bleed
        for y in 0..height {
            for x in 0..width {
                grid.set_cell(x, y, ' ', theme.header_fg, theme.row_bg_even);
            }
        }

        // 1. Header (FIXED WIDTH TO ENSURE ALIGNMENT)
        let name_trimmed = Self::fixed_width(&play_data.name, 20);
        let header_str = format!("'{}' dur: {:6.2}ms tick: {:3} pos: {:3X}/{:<3X} row: {:3X}/{:<3X} bpm: {:3} spd: {:2} FPS: {:4.1} f: {:?}", 
            name_trimmed, play_data.tick_duration_in_ms, play_data.tick,
            play_data.song_position, play_data.song_length.saturating_sub(1), 
            play_data.row, play_data.pattern_len.saturating_sub(1), 
            play_data.bpm, play_data.speed, play_data.display_fps, play_data.filter
        );
        // Fill entire header row with header_bg
        for x in 0..grid.width {
            grid.set_cell(x, 0, ' ', theme.header_fg, theme.header_bg);
        }
        grid.print(0, 0, &header_str, theme.header_fg, theme.header_bg);

        // 2. Dynamic Layout Calculation
        let vis_height = if platform == TargetPlatform::Native && visualizer_mode < 3 {
             (height / 3).max(11).min(20)
        } else {
            0
        };
        let pat_max_y = height.saturating_sub(vis_height);

        match view_mode {
            ViewMode::Pattern => {
                Self::render_pattern(grid, play_data, instruments, patterns, order, &theme, x_offset, 0, platform, visualizer_mode, theme_id, pat_max_y);
            },
            ViewMode::Instruments => Self::render_instruments(grid, instruments, 0, &theme),
            ViewMode::Message => Self::render_message(grid, &play_data.song_message, 0, &theme),
            ViewMode::Help => Self::render_help(grid, 0, &theme),
        }

        // 3. High-Fidelity Visualizers at the bottom
        if vis_height > 0 {
            let vis_y = height.saturating_sub(vis_height);
            
            // ENSURE THEME CONSISTENCY: Pre-fill the entire visualizer area with theme background
            for fy in vis_y..height {
                for fx in 0..width {
                    grid.set_cell(fx, fy, ' ', theme.pat_note_fg, theme.pat_row_bg);
                }
            }

            match visualizer_mode {
                0 => Self::render_fft(grid, &play_data.master_spectrum, 0, vis_y, width, vis_height, &theme.meter_colors, theme.pat_row_bg),
                1 => Self::render_master_scope(grid, &play_data.master_oscilloscope, 0, vis_y, width, vis_height, &theme),
                2 => Self::render_multi_scope(grid, &play_data.channel_status, 0, vis_y, width, vis_height, &theme),
                _ => {}
            }
        }
    }

    fn get_theme(theme_id: u32) -> Theme {
        match theme_id {
            1 => { // Cyberpunk / Vibrant (SYNCHRONIZED WITH WEB)
                Theme {
                    meter_colors: [
                        RGB { r: 123, g: 39, b: 255 }, RGB { r: 115, g: 42, b: 255 }, RGB { r: 107, g: 45, b: 255 },
                        RGB { r: 99, g: 48, b: 255 }, RGB { r: 91, g: 51, b: 255 }, RGB { r: 83, g: 54, b: 255 },
                        RGB { r: 75, g: 57, b: 255 }, RGB { r: 67, g: 60, b: 255 }, RGB { r: 59, g: 63, b: 255 },
                        RGB { r: 51, g: 66, b: 255 }, RGB { r: 43, g: 69, b: 255 }, RGB { r: 35, g: 72, b: 255 },
                        RGB { r: 0, g: 142, b: 254 }, RGB { r: 0, g: 154, b: 254 }, RGB { r: 0, g: 166, b: 254 },
                        RGB { r: 0, g: 178, b: 254 }, RGB { r: 0, g: 190, b: 254 }, RGB { r: 0, g: 202, b: 254 },
                        RGB { r: 0, g: 214, b: 254 }, RGB { r: 0, g: 226, b: 254 }, RGB { r: 0, g: 234, b: 254 },
                        RGB { r: 0, g: 242, b: 254 }, RGB { r: 0, g: 250, b: 254 }, RGB { r: 0, g: 255, b: 255 },
                    ],
                    freq_colors: [
                        RGB { r: 0, g: 50, b: 150 }, RGB { r: 0, g: 60, b: 160 }, RGB { r: 0, g: 70, b: 170 },
                        RGB { r: 0, g: 80, b: 180 }, RGB { r: 0, g: 90, b: 190 }, RGB { r: 0, g: 100, b: 200 },
                        RGB { r: 0, g: 110, b: 210 }, RGB { r: 0, g: 120, b: 220 }, RGB { r: 0, g: 130, b: 230 },
                        RGB { r: 0, g: 140, b: 240 }, RGB { r: 0, g: 150, b: 250 }, RGB { r: 0, g: 160, b: 254 },
                        RGB { r: 0, g: 170, b: 254 }, RGB { r: 0, g: 180, b: 254 }, RGB { r: 0, g: 190, b: 254 },
                        RGB { r: 0, g: 200, b: 254 }, RGB { r: 0, g: 210, b: 254 }, RGB { r: 0, g: 220, b: 254 },
                        RGB { r: 0, g: 230, b: 254 }, RGB { r: 0, g: 242, b: 254 }, RGB { r: 0, g: 242, b: 254 },
                        RGB { r: 0, g: 242, b: 254 }, RGB { r: 0, g: 242, b: 254 }, RGB { r: 0, g: 242, b: 254 },
                    ],
                    header_bg: RGB { r: 15, g: 16, b: 45 },
                    header_fg: RGB { r: 0, g: 242, b: 254 },
                    accent_fg: RGB { r: 0, g: 242, b: 254 },
                    table_hdr_bg: RGB { r: 26, g: 27, b: 58 },
                    table_hdr_fg: RGB { r: 0, g: 242, b: 254 },
                    row_bg_odd: RGB { r: 20, g: 21, b: 50 },
                    row_bg_even: RGB { r: 10, g: 11, b: 30 },
                    col_on: RGB { r: 0, g: 255, b: 0 },
                    col_off: RGB { r: 100, g: 100, b: 100 },
                    col_inst: RGB { r: 0, g: 255, b: 80 },
                    col_freq: RGB { r: 0, g: 242, b: 254 },
                    col_note: RGB { r: 255, g: 255, b: 0 },
                    col_period: RGB { r: 123, g: 39, b: 255 },
                    col_sep: RGB { r: 26, g: 27, b: 58 },
                    pat_row_bg: RGB { r: 0, g: 0, b: 10 },
                    pat_curr_bg: RGB { r: 40, g: 20, b: 100 },
                    pat_note_fg: RGB { r: 0, g: 242, b: 254 }, // Cyan
                    pat_inst_fg: RGB { r: 0, g: 255, b: 80 },  // Green
                    pat_vol_fg: RGB { r: 255, g: 255, b: 0 },   // Yellow
                    pat_eff_fg: RGB { r: 123, g: 39, b: 255 },  // Purple
                }
            },
            2 => { // Obsidian / Monokai optimized
                Theme {
                    meter_colors: [
                        RGB { r: 102, g: 217, b: 239 }, RGB { r: 112, g: 220, b: 190 }, RGB { r: 122, g: 223, b: 141 },
                        RGB { r: 132, g: 226, b: 92 }, RGB { r: 142, g: 227, b: 43 }, RGB { r: 166, g: 226, b: 46 },
                        RGB { r: 186, g: 226, b: 46 }, RGB { r: 206, g: 226, b: 46 }, RGB { r: 220, g: 215, b: 46 },
                        RGB { r: 253, g: 211, b: 31 }, RGB { r: 253, g: 196, b: 31 }, RGB { r: 253, g: 181, b: 31 },
                        RGB { r: 253, g: 166, b: 31 }, RGB { r: 253, g: 151, b: 31 }, RGB { r: 250, g: 123, b: 51 },
                        RGB { r: 248, g: 95, b: 72 }, RGB { r: 249, g: 67, b: 93 }, RGB { r: 249, g: 38, b: 114 },
                        RGB { r: 229, g: 60, b: 149 }, RGB { r: 210, g: 83, b: 184 }, RGB { r: 191, g: 106, b: 219 },
                        RGB { r: 174, g: 129, b: 255 }, RGB { r: 174, g: 129, b: 255 }, RGB { r: 174, g: 129, b: 255 },
                    ],
                    freq_colors: [
                        RGB { r: 174, g: 129, b: 255 }, RGB { r: 164, g: 139, b: 255 }, RGB { r: 154, g: 149, b: 255 },
                        RGB { r: 144, g: 159, b: 255 }, RGB { r: 134, g: 169, b: 255 }, RGB { r: 124, g: 179, b: 255 },
                        RGB { r: 114, g: 189, b: 155 }, RGB { r: 108, g: 203, b: 197 }, RGB { r: 102, g: 217, b: 239 },
                        RGB { r: 102, g: 217, b: 239 }, RGB { r: 102, g: 217, b: 239 }, RGB { r: 102, g: 217, b: 239 },
                        RGB { r: 102, g: 217, b: 239 }, RGB { r: 102, g: 217, b: 239 }, RGB { r: 102, g: 217, b: 239 },
                        RGB { r: 102, g: 217, b: 239 }, RGB { r: 102, g: 217, b: 239 }, RGB { r: 102, g: 217, b: 239 },
                        RGB { r: 102, g: 217, b: 239 }, RGB { r: 102, g: 217, b: 239 }, RGB { r: 102, g: 217, b: 239 },
                        RGB { r: 102, g: 217, b: 239 }, RGB { r: 102, g: 217, b: 239 }, RGB { r: 102, g: 217, b: 239 },
                    ],
                    header_bg: RGB { r: 35, g: 35, b: 35 },
                    header_fg: RGB { r: 249, g: 38, b: 114 },
                    accent_fg: RGB { r: 249, g: 38, b: 114 },
                    table_hdr_bg: RGB { r: 45, g: 45, b: 45 },
                    table_hdr_fg: RGB { r: 102, g: 217, b: 239 },
                    row_bg_odd: RGB { r: 35, g: 35, b: 35 },
                    row_bg_even: RGB { r: 30, g: 30, b: 30 },
                    col_on: RGB { r: 166, g: 226, b: 46 },
                    col_off: RGB { r: 120, g: 120, b: 120 },
                    col_inst: RGB { r: 249, g: 38, b: 114 },
                    col_freq: RGB { r: 102, g: 217, b: 239 },
                    col_note: RGB { r: 166, g: 226, b: 46 },
                    col_period: RGB { r: 174, g: 129, b: 255 },
                    col_sep: RGB { r: 60, g: 60, b: 60 },
                    pat_row_bg: RGB { r: 35, g: 35, b: 35 },
                    pat_curr_bg: RGB { r: 73, g: 72, b: 62 },
                    pat_note_fg: RGB { r: 102, g: 217, b: 239 },
                    pat_inst_fg: RGB { r: 166, g: 226, b: 46 },
                    pat_vol_fg: RGB { r: 249, g: 38, b: 114 },
                    pat_eff_fg: RGB { r: 174, g: 129, b: 255 },
                }
            },
            3 => { // Mono / Amber
                Theme {
                    meter_colors: [
                        RGB { r: 100, g: 40, b: 0 }, RGB { r: 110, g: 45, b: 0 }, RGB { r: 120, g: 50, b: 0 },
                        RGB { r: 130, g: 55, b: 0 }, RGB { r: 140, g: 60, b: 0 }, RGB { r: 150, g: 65, b: 0 },
                        RGB { r: 160, g: 70, b: 0 }, RGB { r: 170, g: 75, b: 0 }, RGB { r: 180, g: 80, b: 0 },
                        RGB { r: 190, g: 85, b: 0 }, RGB { r: 200, g: 90, b: 0 }, RGB { r: 210, g: 95, b: 0 },
                        RGB { r: 220, g: 100, b: 0 }, RGB { r: 230, g: 105, b: 0 }, RGB { r: 240, g: 110, b: 0 },
                        RGB { r: 250, g: 115, b: 0 }, RGB { r: 255, g: 125, b: 0 }, RGB { r: 255, g: 135, b: 0 },
                        RGB { r: 255, g: 145, b: 0 }, RGB { r: 255, g: 145, b: 0 }, RGB { r: 255, g: 145, b: 0 },
                        RGB { r: 255, g: 145, b: 0 }, RGB { r: 255, g: 145, b: 0 }, RGB { r: 255, g: 145, b: 0 },
                    ],
                    freq_colors: [
                        RGB { r: 50, g: 20, b: 0 }, RGB { r: 60, g: 25, b: 0 }, RGB { r: 70, g: 30, b: 0 },
                        RGB { r: 80, g: 35, b: 0 }, RGB { r: 90, g: 40, b: 0 }, RGB { r: 100, g: 45, b: 0 },
                        RGB { r: 110, g: 50, b: 0 }, RGB { r: 120, g: 55, b: 0 }, RGB { r: 130, g: 60, b: 0 },
                        RGB { r: 140, g: 65, b: 0 }, RGB { r: 150, g: 70, b: 0 }, RGB { r: 160, g: 75, b: 0 },
                        RGB { r: 170, g: 80, b: 0 }, RGB { r: 180, g: 85, b: 0 }, RGB { r: 190, g: 90, b: 0 },
                        RGB { r: 200, g: 95, b: 0 }, RGB { r: 210, g: 100, b: 0 }, RGB { r: 225, g: 110, b: 0 },
                        RGB { r: 240, g: 120, b: 0 }, RGB { r: 255, g: 140, b: 0 }, RGB { r: 255, g: 140, b: 0 },
                        RGB { r: 255, g: 140, b: 0 }, RGB { r: 255, g: 140, b: 0 }, RGB { r: 255, g: 140, b: 0 },
                    ],
                    header_bg: RGB { r: 30, g: 10, b: 0 },
                    header_fg: RGB { r: 255, g: 140, b: 0 },
                    accent_fg: RGB { r: 255, g: 140, b: 0 },
                    table_hdr_bg: RGB { r: 40, g: 15, b: 0 },
                    table_hdr_fg: RGB { r: 255, g: 140, b: 0 },
                    row_bg_odd: RGB { r: 30, g: 10, b: 0 },
                    row_bg_even: RGB { r: 20, g: 5, b: 0 },
                    col_on: RGB { r: 255, g: 140, b: 0 },
                    col_off: RGB { r: 100, g: 40, b: 0 },
                    col_inst: RGB { r: 255, g: 140, b: 0 },
                    col_freq: RGB { r: 255, g: 140, b: 0 },
                    col_note: RGB { r: 255, g: 140, b: 0 },
                    col_period: RGB { r: 255, g: 140, b: 0 },
                    col_sep: RGB { r: 100, g: 40, b: 0 },
                    pat_row_bg: RGB { r: 0, g: 0, b: 0 },
                    pat_curr_bg: RGB { r: 120, g: 60, b: 0 },
                    pat_note_fg: RGB { r: 255, g: 140, b: 0 },
                    pat_inst_fg: RGB { r: 255, g: 140, b: 0 },
                    pat_vol_fg: RGB { r: 255, g: 140, b: 0 },
                    pat_eff_fg: RGB { r: 255, g: 140, b: 0 },
                }
            },
            4 => { // Studio Slate (Modernized Pro)
                Theme {
                    meter_colors: [
                        RGB { r: 47, g: 51, b: 77 }, RGB { r: 54, g: 61, b: 94 }, RGB { r: 61, g: 71, b: 112 },
                        RGB { r: 68, g: 81, b: 130 }, RGB { r: 75, g: 91, b: 148 }, RGB { r: 82, g: 101, b: 166 },
                        RGB { r: 122, g: 162, b: 247 }, RGB { r: 111, g: 172, b: 232 }, RGB { r: 100, g: 182, b: 217 },
                        RGB { r: 89, g: 192, b: 202 }, RGB { r: 78, g: 202, b: 187 }, RGB { r: 67, g: 212, b: 172 },
                        RGB { r: 158, g: 206, b: 106 }, RGB { r: 182, g: 201, b: 103 }, RGB { r: 206, g: 196, b: 100 },
                        RGB { r: 224, g: 175, b: 104 }, RGB { r: 230, g: 158, b: 116 }, RGB { r: 236, g: 141, b: 128 },
                        RGB { r: 247, g: 118, b: 142 }, RGB { r: 230, g: 100, b: 130 }, RGB { r: 210, g: 80, b: 110 },
                        RGB { r: 190, g: 60, b: 90 }, RGB { r: 170, g: 40, b: 70 }, RGB { r: 150, g: 20, b: 50 },
                    ],
                    freq_colors: [
                        RGB { r: 35, g: 55, b: 115 }, RGB { r: 45, g: 65, b: 135 }, RGB { r: 55, g: 75, b: 155 },
                        RGB { r: 65, g: 85, b: 175 }, RGB { r: 75, g: 95, b: 195 }, RGB { r: 85, g: 105, b: 215 },
                        RGB { r: 122, g: 162, b: 247 }, RGB { r: 112, g: 172, b: 237 }, RGB { r: 102, g: 182, b: 227 },
                        RGB { r: 92, g: 192, b: 217 }, RGB { r: 82, g: 202, b: 207 }, RGB { r: 72, g: 212, b: 197 },
                        RGB { r: 158, g: 206, b: 106 }, RGB { r: 158, g: 206, b: 106 }, RGB { r: 158, g: 206, b: 106 },
                        RGB { r: 158, g: 206, b: 106 }, RGB { r: 158, g: 206, b: 106 }, RGB { r: 150, g: 220, b: 100 },
                        RGB { r: 140, g: 240, b: 90 }, RGB { r: 130, g: 255, b: 80 }, RGB { r: 160, g: 255, b: 100 },
                        RGB { r: 190, g: 255, b: 120 }, RGB { r: 220, g: 255, b: 140 }, RGB { r: 255, g: 255, b: 255 },
                    ],
                    header_bg: RGB { r: 26, g: 27, b: 38 },
                    header_fg: RGB { r: 192, g: 202, b: 245 },
                    accent_fg: RGB { r: 122, g: 162, b: 247 },
                    table_hdr_bg: RGB { r: 22, g: 22, b: 30 },
                    table_hdr_fg: RGB { r: 122, g: 162, b: 247 },
                    row_bg_odd: RGB { r: 26, g: 27, b: 38 },
                    row_bg_even: RGB { r: 22, g: 22, b: 30 },
                    col_on: RGB { r: 158, g: 206, b: 106 },
                    col_off: RGB { r: 65, g: 72, b: 104 },
                    col_inst: RGB { r: 158, g: 206, b: 106 },
                    col_freq: RGB { r: 122, g: 162, b: 247 },
                    col_note: RGB { r: 192, g: 202, b: 245 },
                    col_period: RGB { r: 158, g: 206, b: 106 },
                    col_sep: RGB { r: 65, g: 72, b: 104 },
                    pat_row_bg: RGB { r: 22, g: 22, b: 30 },
                    pat_curr_bg: RGB { r: 47, g: 51, b: 77 },
                    pat_note_fg: RGB { r: 192, g: 202, b: 245 },
                    pat_inst_fg: RGB { r: 158, g: 206, b: 106 },
                    pat_vol_fg: RGB { r: 224, g: 175, b: 104 },
                    pat_eff_fg: RGB { r: 247, g: 118, b: 142 },
                }
            },
            _ => { // Classic Pro (Refined)
                Theme {
                    meter_colors: [
                        RGB { r: 0, g: 100, b: 0 }, RGB { r: 0, g: 120, b: 0 }, RGB { r: 0, g: 140, b: 0 },
                        RGB { r: 0, g: 160, b: 0 }, RGB { r: 0, g: 180, b: 0 }, RGB { r: 0, g: 200, b: 0 },
                        RGB { r: 0, g: 220, b: 0 }, RGB { r: 0, g: 240, b: 0 }, RGB { r: 60, g: 240, b: 0 },
                        RGB { r: 120, g: 240, b: 0 }, RGB { r: 180, g: 240, b: 0 }, RGB { r: 240, g: 240, b: 0 },
                        RGB { r: 240, g: 210, b: 0 }, RGB { r: 240, g: 180, b: 0 }, RGB { r: 240, g: 150, b: 0 },
                        RGB { r: 240, g: 120, b: 0 }, RGB { r: 240, g: 90, b: 0 }, RGB { r: 240, g: 60, b: 0 },
                        RGB { r: 240, g: 30, b: 0 }, RGB { r: 255, g: 0, b: 0 }, RGB { r: 255, g: 0, b: 0 },
                        RGB { r: 255, g: 0, b: 0 }, RGB { r: 255, g: 0, b: 0 }, RGB { r: 255, g: 0, b: 0 },
                    ],
                    freq_colors: [
                        RGB { r: 0, g: 0, b: 150 }, RGB { r: 0, g: 30, b: 180 }, RGB { r: 0, g: 60, b: 210 },
                        RGB { r: 0, g: 90, b: 240 }, RGB { r: 0, g: 120, b: 255 }, RGB { r: 0, g: 150, b: 255 },
                        RGB { r: 0, g: 180, b: 255 }, RGB { r: 0, g: 210, b: 180 }, RGB { r: 0, g: 240, b: 100 },
                        RGB { r: 30, g: 255, b: 0 }, RGB { r: 60, g: 255, b: 0 }, RGB { r: 90, g: 255, b: 0 },
                        RGB { r: 120, g: 255, b: 0 }, RGB { r: 150, g: 255, b: 0 }, RGB { r: 180, g: 255, b: 0 },
                        RGB { r: 210, g: 255, b: 0 }, RGB { r: 240, g: 255, b: 0 }, RGB { r: 255, g: 180, b: 0 },
                        RGB { r: 255, g: 120, b: 0 }, RGB { r: 255, g: 60, b: 0 }, RGB { r: 255, g: 0, b: 0 },
                        RGB { r: 255, g: 0, b: 0 }, RGB { r: 255, g: 0, b: 0 }, RGB { r: 255, g: 0, b: 0 },
                    ],
                    header_bg: RGB { r: 0, g: 0, b: 128 },
                    header_fg: RGB { r: 255, g: 255, b: 255 },
                    accent_fg: RGB { r: 255, g: 255, b: 255 },
                    table_hdr_bg: RGB { r: 0, g: 0, b: 64 },
                    table_hdr_fg: RGB { r: 255, g: 255, b: 255 },
                    row_bg_odd: RGB { r: 15, g: 15, b: 15 },
                    row_bg_even: RGB { r: 0, g: 0, b: 0 },
                    col_on: RGB { r: 0, g: 255, b: 0 },
                    col_off: RGB { r: 128, g: 128, b: 128 },
                    col_inst: RGB { r: 220, g: 220, b: 220 },
                    col_freq: RGB { r: 0, g: 242, b: 254 },
                    col_note: RGB { r: 255, g: 255, b: 0 },
                    col_period: RGB { r: 0, g: 255, b: 0 },
                    col_sep: RGB { r: 128, g: 128, b: 128 },
                    pat_row_bg: RGB { r: 0, g: 0, b: 0 },
                    pat_curr_bg: RGB { r: 42, g: 60, b: 126 },
                    pat_note_fg: RGB { r: 255, g: 255, b: 255 },
                    pat_inst_fg: RGB { r: 74, g: 246, b: 38 },
                    pat_vol_fg: RGB { r: 246, g: 211, b: 45 },
                    pat_eff_fg: RGB { r: 241, g: 76, b: 76 },
                }
            }
        }
    }

    fn render_pattern(
        grid: &mut Grid,
        play_data: &PlayData,
        instruments: &Vec<Instrument>,
        patterns: &Vec<Patterns>,
        order: &Vec<u8>,
        theme: &Theme,
        x_offset: isize,
        y_offset: isize,
        _platform: TargetPlatform,
        _visualizer_mode: u32,
        theme_id: u32,
        max_y: usize,
    ) {
        let x_start = x_offset.max(0) as usize;
        let y_start = (y_offset.max(0) as usize) + 1;

        let num_channels = play_data.channel_status.len();

        let _theme = Self::get_theme(theme_id); // we still use this to ensure it's loaded if needed, but we use the passed 'theme'
        let _show_scopes = play_data.scopes_enabled;
        let use_two_columns = grid.width > 260 && num_channels > 16;
        let channels_to_show = num_channels.min(64);
        let per_col = if use_two_columns { (channels_to_show + 1) / 2 } else { channels_to_show };

        let channel_scroll = match play_data.user_data.get("channel_scroll") {
            Some(UserData::USize(v)) => *v,
            _ => 0
        };

        // Table Header (ABSOLUTE PARITY WITH WEB SCREENSHOT)
        let table_hdr = "STAT| CH |      INSTRUMENT      | FREQ | VOLUME | POSITION | NOTE | PITC | CHAN VOL | ENVELOPE | GLOBAL VOL | FADEOUT | PANNING |";
        grid.print(x_start, y_start, table_hdr, theme.table_hdr_fg, theme.table_hdr_bg);
        if use_two_columns {
            grid.print(x_start + 130, y_start, table_hdr, theme.table_hdr_fg, theme.table_hdr_bg);
        }

        let mut _max_y_reached = false;
        for i in 0..channels_to_show {
            let actual_ch = (i + channel_scroll) % num_channels.max(1);
            let col = if use_two_columns { i / per_col } else { 0 };
            let row = if use_two_columns { i % per_col } else { i };
            let x = x_start + (col * 130);
            let y = y_start + 1 + row;

            if y >= max_y { 
                _max_y_reached = true;
                break; 
            }

            let channel = &play_data.channel_status[actual_ch];
            let row_bg = if i % 2 == 1 { theme.row_bg_odd } else { theme.row_bg_even };

            let status = if channel.force_off { " x " } else if channel.on { " ON" } else { "OFF" };
            let col_status = if channel.on { theme.col_on } else { theme.col_off };
            
            // PIXEL PERFECT CURSOR-BASED LAYOUT
            grid.print(x, y, status, col_status, row_bg);
            grid.print(x + 4, y, "|", theme.col_sep, row_bg);
            grid.print(x + 5, y, &format!(" {:02} ", actual_ch + 1), theme.col_note, row_bg);
            grid.print(x + 9, y, "|", theme.col_sep, row_bg);

            if channel.on {
                grid.print(x + 10, y, &format!(" {:>2}:{:17} ", channel.instrument, Self::fixed_width(&channel.instrument_name, 17)), theme.col_inst, row_bg);
                grid.print(x + 32, y, "|", theme.col_sep, row_bg);
                grid.print(x + 33, y, &format!(" {:<4} ", channel.frequency as u32 % 100000), theme.col_freq, row_bg);
                grid.print(x + 39, y, "|", theme.col_sep, row_bg);
                
                Self::grid_range_with_color(grid, x + 40, y, (channel.volume as f32 / 64.0 * 8.0).ceil() as u32, 8, 8, &theme.meter_colors, row_bg); 
                grid.print(x + 48, y, "|", theme.col_sep, row_bg);
                
                let inst_len = if channel.instrument < instruments.len() && channel.sample < instruments[channel.instrument].samples.len() { instruments[channel.instrument].samples[channel.sample].length.max(1) } else { 1 };
                Self::grid_range(grid, x + 49, y, channel.sample_position as u32, inst_len - 1, 10, theme.accent_fg, row_bg);
                grid.print(x + 59, y, "|", theme.col_sep, row_bg);
                
                grid.print(x + 60, y, &format!(" {:3} ", channel.note), theme.col_note, row_bg);
                grid.print(x + 66, y, "|", theme.col_sep, row_bg);
                
                let f_min = 27.5f32;
                let f_max = 131072.0f32;
                let f_pos = if channel.frequency > 0.0 {
                    ((channel.frequency.max(f_min).min(f_max).ln() - f_min.ln()) / (f_max.ln() - f_min.ln()) * 6.0).ceil() as u32
                } else {
                    0
                };
                Self::grid_range_with_color(grid, x + 67, y, f_pos, 6, 6, &theme.freq_colors, row_bg); 
                grid.print(x + 73, y, "|", theme.col_sep, row_bg);
                
                Self::grid_range_with_color(grid, x + 74, y, (channel.volume as f32 / 64.0 * 10.0).ceil() as u32, 10, 10, &theme.meter_colors, row_bg);
                grid.print(x + 84, y, "|", theme.col_sep, row_bg);
                Self::grid_range_with_color(grid, x + 85, y, (channel.envelope_volume as f32 / 16383.0 * 10.0).ceil() as u32, 10, 10, &theme.meter_colors, row_bg);
                grid.print(x + 95, y, "|", theme.col_sep, row_bg);
                Self::grid_range_with_color(grid, x + 96, y, (channel.global_volume as f32 / 64.0 * 12.0).ceil() as u32, 12, 12, &theme.meter_colors, row_bg);
                grid.print(x + 108, y, "|", theme.col_sep, row_bg);
                Self::grid_range_with_color(grid, x + 109, y, (channel.fadeout_volume / 7282.0) as u32, 9, 9, &theme.meter_colors, row_bg);
                grid.print(x + 118, y, "|", theme.col_sep, row_bg);
                
                // ODD WIDTH: 9 chars for perfect centering
                Self::grid_range(grid, x + 119, y, channel.final_panning as u32, 255, 9, theme.accent_fg, row_bg);
                grid.print(x + 128, y, "|", theme.col_sep, row_bg);
            } else {
                grid.print(x + 10, y, &" ".repeat(118), theme.col_off, row_bg);
                grid.print(x + 32, y, "|", theme.col_sep, row_bg);
                grid.print(x + 39, y, "|", theme.col_sep, row_bg);
                grid.print(x + 48, y, "|", theme.col_sep, row_bg);
                grid.print(x + 59, y, "|", theme.col_sep, row_bg);
                grid.print(x + 66, y, "|", theme.col_sep, row_bg);
                grid.print(x + 73, y, "|", theme.col_sep, row_bg);
                grid.print(x + 84, y, "|", theme.col_sep, row_bg);
                grid.print(x + 95, y, "|", theme.col_sep, row_bg);
                grid.print(x + 108, y, "|", theme.col_sep, row_bg);
                grid.print(x + 118, y, "|", theme.col_sep, row_bg);
                grid.print(x + 128, y, "|", theme.col_sep, row_bg);
            }
        }

        // --- RENDER PATTERN TRACKER ---
        // Dynamically anchor pattern tracker after the channel list
        let mut last_chan_y = y_start + 1;
        for i in 0..channels_to_show {
            let row = if use_two_columns { i % per_col } else { i };
            let y = y_start + 1 + row;
            if y >= max_y { break; }
            last_chan_y = y + 1;
        }

        let pat_split_y = last_chan_y + 1;
        let fft_area_y = max_y;
        
        if pat_split_y < fft_area_y {
            let pat_header = "--- PATTERN TRACKER ---";
            grid.print(x_start, pat_split_y, pat_header, theme.accent_fg, theme.pat_row_bg);
            
            // Channel Column Header (PIXEL PERFECT ALIGNMENT)
            grid.print(x_start, pat_split_y + 1, "idx    | ", theme.table_hdr_fg, theme.table_hdr_bg);
            let num_ch_render = (grid.width.saturating_sub(x_start + 8)) / 14;
            for i in 0..num_ch_render.min(num_channels) {
                let actual_ch = (i + channel_scroll) % num_channels;
                if x_start + 8 + i * 14 + 13 > grid.width { break; }
                grid.print(x_start + 8 + i * 14, pat_split_y + 1, &format!("CH{:02}         |", actual_ch + 1), theme.table_hdr_fg, theme.table_hdr_bg);
            }

            if play_data.song_position < order.len() && order[play_data.song_position] < patterns.len() as u8 {
                let pattern = &patterns[order[play_data.song_position] as usize];
                
                // ADJUSTABLE VISIBLE ROWS
                let visible_rows = fft_area_y.saturating_sub(pat_split_y + 2);
                let total_pattern_rows = pattern.rows.len();
                
                // Three-Phase Scroller: Anchor Start, Center Scroll, Anchor End
                let mid = visible_rows / 2;
                let first_row = if play_data.row < mid {
                    0
                } else if play_data.row >= total_pattern_rows.saturating_sub(visible_rows.saturating_sub(mid)) {
                    total_pattern_rows.saturating_sub(visible_rows)
                } else {
                    play_data.row - mid
                };
                
                for i in 0..visible_rows {
                    let row_idx = first_row + i;
                    if row_idx >= pattern.rows.len() { break; }
                    
                    let draw_y = pat_split_y + 2 + i;
                    let is_current = row_idx == play_data.row;
                    let row_bg = if is_current { theme.pat_curr_bg } else { theme.pat_row_bg };
                    
                    // SOLID BACKGROUND (Fill whole row first to avoid gaps)
                    for fill_x in x_start..grid.width {
                        grid.set_cell(fill_x, draw_y, ' ', theme.pat_note_fg, row_bg);
                    }

                    grid.print(x_start, draw_y, &format!("{:02X}     | ", row_idx), theme.col_note, row_bg);
                    
                    for ch_i in 0..num_ch_render.min(num_channels) {
                        let actual_ch = (ch_i + channel_scroll) % num_channels;
                        let curr_x = x_start + 8 + ch_i * 14;
                        if curr_x + 13 >= grid.width { break; }
                        
                        let p = &pattern.rows[row_idx].channels[actual_ch];
                        
                        let note = if p.note == 0 { "---".to_string() } else if p.note == 97 { "OFF".to_string() } else {
                            format!("{}{}", xmplayer::pattern::Pattern::NOTES[((p.note - 1) % 12) as usize], (p.note - 1) / 12)
                        };
                        let inst = if p.instrument == 0 { "..".to_string() } else { format!("{:02X}", p.instrument) };
                        let vol = if p.volume == 0 { "..".to_string() } else { format!("{:02X}", p.volume) };
                        let effect = if p.effect == 0 && p.effect_param == 0 { "...".to_string() } else { format!("{:01X}{:02X}", p.effect, p.effect_param) };
                        
                        // MULTICOLOR TRACKER RENDERING WITH CHANNEL SEPARATORS
                        grid.print(curr_x, draw_y, &note, theme.pat_note_fg, row_bg);
                        grid.print(curr_x + 4, draw_y, &inst, theme.pat_inst_fg, row_bg);
                        grid.print(curr_x + 7, draw_y, &vol, theme.pat_vol_fg, row_bg);
                        grid.print(curr_x + 10, draw_y, &effect, theme.pat_eff_fg, row_bg);
                        grid.print(curr_x + 13, draw_y, "|", theme.col_sep, row_bg);
                    }
                }
            }
        }
    }

    fn render_multi_scope(grid: &mut Grid, status: &[ChannelStatus], x: usize, y: usize, width: usize, height: usize, theme: &Theme) {
        if status.is_empty() || height == 0 || width == 0 { return; }
        
        let n = status.len();
        let cols = ((n as f32).sqrt().ceil() as usize).max(2);
        let rows = ((n as f32 / cols as f32).ceil() as usize).max(1);
        
        let cell_w = width / cols;
        let cell_h = height / rows;
        if cell_w < 4 || cell_h < 1 { return; }

        for i in 0..n {
            let ch_x = x + (i % cols) * cell_w;
            let ch_y = y + (i / cols) * cell_h;
            if ch_y >= y + height { break; }

            if status[i].on {
                // High-fidelity per-channel Braille scope with local AGC
                let data = &status[i].oscilloscope;
                let mut peak: f32 = 0.0001;
                for &s in data.iter() {
                    let abs_s = s.abs();
                    if abs_s > peak { peak = abs_s; }
                }
                
                let gain = (0.8 / peak).min(10.0);
                let vertical_dots = cell_h * 4;
                let display_samples = data.len().min(cell_w * 2);
                
                let mut prev_dot_y: Option<usize> = None;
                for dx in 0..cell_w.saturating_sub(1) {
                    let sample_idx = (dx * display_samples) / cell_w;
                    if sample_idx >= data.len() { break; }
                    let sample = data[sample_idx] * gain;
                    
                    let center = vertical_dots as f32 / 2.0;
                    let dot_y = (center - (sample * center)).round() as i32;
                    let dot_y = dot_y.max(0).min(vertical_dots as i32 - 1) as usize;
                    
                    let (sy, ey) = match prev_dot_y {
                        Some(p) => (p.min(dot_y), p.max(dot_y)),
                        None => (dot_y, dot_y)
                    };

                    for y_dot in sy..=ey {
                        let char_r = y_dot / 4;
                        let dot_in_c = y_dot % 4;
                        let cell_y = ch_y + char_r;
                        if cell_y >= ch_y + cell_h { continue; }
                        
                        // Per-channel LOCAL color gradient relative to scope height
                        let color_idx = ((cell_y - ch_y) * 12) / cell_h;
                        let scope_color = theme.meter_colors[color_idx.min(11)];

                        let dot_patterns = [0x01, 0x02, 0x04, 0x40];
                        grid.merge_braille_cell(ch_x + dx, cell_y, dot_patterns[dot_in_c], scope_color, theme.pat_row_bg);
                    }
                    prev_dot_y = Some(dot_y);
                }
            } else {
                // Background fill for inactive channels
                for fy in 0..cell_h {
                    for fx in 0..cell_w {
                        grid.set_cell(ch_x + fx, ch_y + fy, ' ', theme.col_off, theme.pat_row_bg);
                    }
                }
            }
        }
    }


    fn render_fft(grid: &mut Grid, spectrum: &[f32], x: usize, y: usize, width: usize, height: usize, colors: &[RGB], bg: RGB) {
        if spectrum.is_empty() { return; }
        
        // 1. Peak normalization for spectral AGC
        let mut max_val: f32 = 0.0001;
        for &v in spectrum.iter() {
            if v > max_val { max_val = v; }
        }
        let gain = (2.0 / max_val).min(10.0); // Increased max gain for better visibility
        
        let blocks = [' ', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let bar_height = height.saturating_sub(1); 

        for i in 0..width {
            let sample_idx = (i * spectrum.len()) / width;
            let val = spectrum[sample_idx.min(spectrum.len() - 1)] * gain;
            
            // Smoother scaling: emphasize higher magnitudes slightly more
            let log_val = (val * 4.0).ln_1p() / (4.0f32).ln_1p();
            let h_filled = (log_val * (bar_height as f32 * 8.0)).round() as u32;

            for h in 0..bar_height {
                let cell_y = y + bar_height - 1 - h;
                if cell_y >= grid.height { continue; }
                
                let block_idx = if h_filled >= ((h + 1) * 8) as u32 { 7 } else if h_filled < (h * 8) as u32 { 0 } else { (h_filled % 8) as usize };
                if block_idx > 0 {
                    let color_idx = (h * colors.len()) / bar_height;
                    grid.set_cell(x + i, cell_y, blocks[block_idx], colors[color_idx.min(colors.len()-1)], bg);
                } else {
                    grid.set_cell(x + i, cell_y, ' ', colors[0], bg);
                }
            }
        }

        // 3. Frequency Labels (Markers at standard frequency bands)
        let labels_y = y + height - 1;
        if labels_y < grid.height {
            grid.print(x, labels_y, "20Hz", colors[0], bg);
            let labels = [
                (width * 10 / 128, "100"), 
                (width * 28 / 128, "500"), 
                (width * 56 / 128, "1k"), 
                (width * 80 / 128, "2k"), 
                (width * 100 / 128, "5k"),
                (width * 115 / 128, "10k"),
                (width.saturating_sub(4), "20k")
            ];
            for (lx, txt) in labels {
                grid.print(x + lx, labels_y, txt, colors[min(lx * colors.len() / width, colors.len()-1)], bg);
            }
        }
    }

    fn render_master_scope(grid: &mut Grid, data: &[f32], x: usize, y: usize, width: usize, height: usize, theme: &Theme) {
        if data.is_empty() || height == 0 { return; }
        
        let vertical_dots = height * 4;
        let mut trigger_offset = 0;
        
        // 1. Automatic Gain Control (AGC) / Peak Normalization
        let mut peak: f32 = 0.0001; 
        for &s in data.iter() {
            let abs_s = s.abs();
            if abs_s > peak { peak = abs_s; }
        }
        let gain = (0.9 / peak).min(10.0); // Cap gain at 10x to avoid noise floor amplification

        // 2. Simple zero-crossing trigger for stability
        for i in 0..data.len() / 2 {
            if data[i] <= 0.0 && data[i+1] > 0.0 {
                trigger_offset = i;
                break;
            }
        }

        let display_samples = data.len().saturating_sub(trigger_offset).min(width * 2);
        let mut prev_dot_y: Option<usize> = None;

        for i in 0..width {
            let sample_idx = trigger_offset + (i * display_samples) / width;
            if sample_idx >= data.len() { break; }
            let sample = data[sample_idx] * gain;
            
            // Map sample to middle of the vertical dots range
            let center = vertical_dots as f32 / 2.0;
            let dot_y = (center - (sample * center)).round() as i32;
            let dot_y = dot_y.max(0).min(vertical_dots as i32 - 1) as usize;
            
            let (start_y, end_y) = match prev_dot_y {
                Some(p) => (p.min(dot_y), p.max(dot_y)),
                None => (dot_y, dot_y)
            };

            for y_dot in start_y..=end_y {
                let char_row = y_dot / 4;
                let dot_in_char = y_dot % 4;
                
                let cell_x = x + i;
                let cell_y = y + char_row;
                if cell_y >= grid.height { continue; }
                
                let dot_patterns = [0x01, 0x02, 0x04, 0x40];
                let color_idx = (y_dot * 24) / vertical_dots;
                grid.merge_braille_cell(cell_x, cell_y, dot_patterns[dot_in_char], theme.meter_colors[color_idx % 24], theme.pat_row_bg);
            }
            prev_dot_y = Some(dot_y);
        }
    }

    fn render_instruments(grid: &mut Grid, instruments: &Vec<Instrument>, y_offset: isize, theme: &Theme) {
        let start_y = (y_offset.max(0) as usize) + 2;
        grid.print(2, start_y, "--- INSTRUMENT LIST ---", theme.accent_fg, theme.table_hdr_bg);
        
        for (i, inst) in instruments.iter().enumerate() {
            let draw_y = start_y + 2 + i;
            if draw_y >= grid.height.saturating_sub(2) { break; }
            
            grid.print(2, draw_y, &format!("{:02X}: {}", i, Self::fixed_width(&inst.name, 40)), theme.col_inst, theme.row_bg_even);
        }
    }

    fn grid_range(grid: &mut Grid, x: usize, y: usize, pos: u32, end: u32, width: usize, color: RGB, bg: RGB) {
        if width == 0 { return; }
        let indicator_pos = if end == 0 { 0 } else { ((pos as f32 / end as f32) * (width as f32 - 1.0)).round() as usize }.min(width - 1);
        for i in 0..width {
            let c = if i == indicator_pos { '=' } else { '-' };
            grid.set_cell(x + i, y, c, color, bg);
        }
    }

    fn grid_range_with_color(grid: &mut Grid, x: usize, y: usize, pos: u32, end: u32, width: usize, colors: &[RGB; 24], bg: RGB) {
        if width == 0 { return; }
        let indicator_pos = if end == 0 { 0 } else { ((pos as f32 / end as f32) * (width as f32)).round() as usize }.min(width);
        for i in 0..width {
            let c = if i == indicator_pos.min(width - 1) && pos > 0 { '=' } else if i < indicator_pos { '=' } else { ' ' };
            let color_idx = (i * colors.len()) / width;
            grid.set_cell(x + i, y, c, colors[color_idx.min(colors.len() - 1)], bg);
        }
    }

    fn render_message(grid: &mut Grid, message: &str, y_offset: isize, theme: &Theme) {
        let start_y = (y_offset.max(0) as usize) + 2;
        for (i, line) in message.lines().enumerate() {
            let draw_y = start_y + i;
            if draw_y >= grid.height { break; }
            grid.print(2, draw_y, line, theme.pat_note_fg, theme.row_bg_even);
        }
    }

    fn render_help(grid: &mut Grid, y_offset: isize, theme: &Theme) {
        let start_y = (y_offset.max(0) as usize) + 2;
        let c1 = 2;
        let c2 = 36;
        let c3 = 68;

        grid.print(c1, start_y,     "--- VIEW MODES ---", theme.accent_fg, theme.row_bg_even);
        grid.print(c1, start_y + 1, "F1: Pattern View", theme.col_note, theme.row_bg_odd);
        grid.print(c1, start_y + 2, "F2: Instrument View", theme.col_note, theme.row_bg_odd);
        grid.print(c1, start_y + 3, "F3: Message View", theme.col_note, theme.row_bg_odd);
        grid.print(c1, start_y + 4, "F4: Help View", theme.col_note, theme.row_bg_odd);

        grid.print(c2, start_y,     "--- NAVIGATION ---", theme.accent_fg, theme.row_bg_even);
        grid.print(c2, start_y + 1, "n / p: Next/Prev Module", theme.col_note, theme.row_bg_odd);
        grid.print(c2, start_y + 2, "r    : Restart Module", theme.col_note, theme.row_bg_odd);
        grid.print(c2, start_y + 3, "Space: Pause / Resume", theme.col_note, theme.row_bg_odd);
        grid.print(c2, start_y + 4, "q/Esc: Quit Player", theme.col_note, theme.row_bg_odd);

        grid.print(c3, start_y,     "--- TRACKER ---", theme.accent_fg, theme.row_bg_even);
        grid.print(c3, start_y + 1, "[ / ]: Scroll Channels", theme.col_note, theme.row_bg_odd);
        grid.print(c3, start_y + 2, "3    : Cycle Channel Height", theme.col_note, theme.row_bg_odd);
        grid.print(c3, start_y + 3, "/    : Loop Pattern", theme.col_note, theme.row_bg_odd);
        grid.print(c3, start_y + 4, "0-9  : Toggle Channel (2-digit)", theme.col_note, theme.row_bg_odd);

        grid.print(c1, start_y + 7, "--- AUDIO ---", theme.accent_fg, theme.row_bg_even);
        grid.print(c1, start_y + 8, "+ / -: Increase/Decrease Speed", theme.col_note, theme.row_bg_odd);
        grid.print(c1, start_y + 9, ". / ,: Increase/Decrease BPM", theme.col_note, theme.row_bg_odd);
        grid.print(c1, start_y + 10,"f    : Cycle Low-pass Filter", theme.col_note, theme.row_bg_odd);
        grid.print(c1, start_y + 11,"a / l: Amiga / Linear Tables", theme.col_note, theme.row_bg_odd);

        grid.print(c2, start_y + 7, "--- VISUALS ---", theme.accent_fg, theme.row_bg_even);
        grid.print(c2, start_y + 8, "T    : Cycle Color Theme", theme.col_note, theme.row_bg_odd);
        grid.print(c2, start_y + 9, "v    : Cycle Master Visualizer", theme.col_note, theme.row_bg_odd);
        grid.print(c2, start_y + 10,"S    : Toggle Channel Scopes", theme.col_note, theme.row_bg_odd);
        grid.print(c2, start_y + 11,"d    : Toggle LCD Display", theme.col_note, theme.row_bg_odd);
    }

    fn fixed_width(s: &str, width: usize) -> String {
        let mut r = s.trim().to_string();
        while r.len() < width { r.push(' '); }
        r.chars().take(width).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grid_range_precision() {
        let mut grid = Grid::new(20, 1);
        let theme = Display::get_theme(0);
        
        // 1. Edge reach: 255/255 on width 9 -> index 8
        Display::grid_range(&mut grid, 0, 0, 255, 255, 9, theme.accent_fg, theme.row_bg_even);
        assert_eq!(grid.cells[8].c, b'=' as u32);
        assert_eq!(grid.cells[7].c, b'-' as u32);

        // 2. Centering: 127/255 on width 9 -> index 4 (127/255 * 8 = 3.98 -> 4)
        let mut grid2 = Grid::new(20, 1);
        Display::grid_range(&mut grid2, 0, 0, 127, 255, 9, theme.accent_fg, theme.row_bg_even);
        assert_eq!(grid2.cells[4].c, b'=' as u32); 
        assert_eq!(grid2.cells[3].c, b'-' as u32);
        assert_eq!(grid2.cells[5].c, b'-' as u32);
    }

    #[test]
    fn test_grid_range_with_color_precision() {
        let mut grid = Grid::new(20, 1);
        let theme = Display::get_theme(0);
        
        // Max volume (64/64) on width 12 -> all cells filled
        Display::grid_range_with_color(&mut grid, 0, 0, 64, 64, 12, &theme.meter_colors, theme.row_bg_even);
        assert_eq!(grid.cells[11].c, b'=' as u32);
        assert_eq!(grid.cells[10].c, b'=' as u32);
    }
}
