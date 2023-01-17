

use xmplayer::song::PlayData;
use std::cmp::{max, min};
use xmplayer::instrument::Instrument;
use xmplayer::module_reader::Patterns;
extern crate wasm_bindgen;

use wasm_bindgen::prelude::*;


#[derive(Debug)]
pub struct ViewPort {
    pub x1: isize,
    pub y1: isize,
    pub width: usize,
    pub height: usize,
}

#[wasm_bindgen]
#[derive(PartialEq, Eq, Copy, Clone)]
pub struct RGB {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

struct Line {
    data:   String,
    background: RGB,
}


impl Line {
    fn new(data: String, background: RGB) -> Self {
        Self { data, background }
    }
}

struct VirtualScreen {
    lines:      Vec<Line>,
}


impl VirtualScreen {
    fn new() -> Self {
        VirtualScreen {
            lines: vec![],
        }
    }

    fn add_line(&mut self, data: String) {
        self.lines.push(Line::new(data, RGB{ r: 0, g: 0, b: 0 }));
    }

    fn add_line_with_color(&mut self, data: String, background: RGB) {
        self.lines.push(Line::new(data, background));
    }
}


pub struct Display {}

impl Display {
    fn color(_color: RGB, str: &str) -> String {
        format!("{}", str)
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

    fn range(pos: u32, start: u32, end: u32, width: usize) -> String {
        let mut result: String = String::from("");
        let mut indicator_pos = ((pos - start) as f32 / (end - start) as f32 * (width) as f32) as usize;
        if indicator_pos > width {
            indicator_pos = width;
        }
        for _ in 0..indicator_pos {
            result += "-";
        }
        result += "=";
        for _ in indicator_pos + 1..(width + 1) as usize {
            result += "-";
        }
        result
    }

    fn range_with_color(pos: u32, start: u32, end: u32, width: usize, colors: &[RGB]) -> String {
        let mut result: String = String::from("");
        if pos == 0 {
            for _ in 0..width + 1 {
                result += " ";
            }
            return result;
        }

        let mut indicator_pos = ((pos - start) as f32 / (end - start) as f32 * (width) as f32) as usize;
        if indicator_pos > width {
            indicator_pos = width;
        }
        for i in 0..indicator_pos {
            result += &*Self::color(colors[i], "=");
        }
        result += &*Self::color(colors[indicator_pos], "=");
        for _ in indicator_pos + 1..(width + 1) as usize {
            result += " "; //&*Self::color(colors[i], "-");
        }
        // result += "\x1b[0m";
        result
    }


    pub fn display(play_data: &PlayData, instruments: &Vec<Instrument>, patterns: &Vec<Patterns>,
                   order: &Vec<u8>, view_port: ViewPort, display: &mut dyn FnMut(String),
                   display_with_background: &mut dyn FnMut(String, RGB)) {

        let mut screen = VirtualScreen::new();

        let colors: [RGB; 12] = [
            RGB { r: 0, g: 120, b: 0 },
            RGB { r: 0, g: 140, b: 0 },
            RGB { r: 0, g: 160, b: 0 },
            RGB { r: 0, g: 180, b: 0 },
            RGB { r: 180, g: 180, b: 0 },
            RGB { r: 195, g: 195, b: 0 },
            RGB { r: 210, g: 210, b: 0 },
            RGB { r: 225, g: 225, b: 0 },
            RGB { r: 225, g: 64, b: 0 },
            RGB { r: 225, g: 64, b: 0 },
            RGB { r: 225, g: 64, b: 0 },
            RGB { r: 225, g: 64, b: 0 },
        ];
        // let first_tick = play_data.tick == 0;
        screen.add_line(format!("'{}' duration in frames: {:5} duration in ms: {:5} tick: {:3} pos: {:3X}/{:<3X}  row: {:3X}/{:<3X} bpm: {:3} speed: {:3} filter: {:5}", play_data.name,
                 play_data.tick_duration_in_frames, play_data.tick_duration_in_ms, play_data.tick, play_data.song_position, play_data.song_length - 1, play_data.row,
                 play_data.pattern_len,
                 play_data.bpm, play_data.speed,
                 play_data.filter
        ));
        // display(Self::move_to(0, 2));

        screen.add_line("on |channel|            instrument            |frequency|   volume   |sample position| note | period |  chan vol  |   envvol   | globalvol  |   fadeout  | panning |".to_string());

        let mut idx = 0u32;
        for channel in &play_data.channel_status {
            idx = idx + 1;
//            if idx != 1  {continue;}
            if channel.on {
                let final_vol =
                    (channel.volume / 64.0) *
                        (channel.envelope_volume / 16384.0) *
                        (channel.global_volume / 64.0) *
                        (channel.fadeout_volume / 65536.0);

                screen.add_line(format!("{:3}| {:5} |{:3}: {:28} |  {:<6} |{:11}|{:14}| {:4} | {:7}|{:11}|{:11}|{:11}|{:11}|{:8}|      ",
                         if channel.force_off { " x" } else if channel.on { "on" } else { "off" }, idx, channel.instrument, instruments[channel.instrument].name.trim(),
                         if channel.on { (channel.frequency) as u32 } else { 0 },
                         Self::range_with_color((final_vol * 12.0).ceil() as u32, 0, 12, 11, &colors),
                         Self::range(channel.sample_position as u32, 0, max(instruments[channel.instrument].samples[channel.sample].length, 1) - 1, 14),
                         channel.note, channel.period,
                         Self::range_with_color(channel.volume as u32, 0, 64, 11, &colors),
                         Self::range_with_color(channel.envelope_volume as u32, 0, 16384, 11, &colors),
                         Self::range_with_color(channel.global_volume as u32, 0, 64, 11, &colors),
                         Self::range_with_color(channel.fadeout_volume as u32, 0, 65536, 11, &colors),
                         Self::range(channel.final_panning as u32, 0, 255, 8),
                ));
            } else {
                screen.add_line(format!("{:3}| {:5} | {:32} |  {:<6} |{:12}| {:14}| {:5}| {:7}|{:12}|{:12}|{:12}|{:12}| {:8}|      ", "off", idx, "", "", "",
                         "", "", "", "", "", "", "", ""));
            }
        }

        screen.add_line("".to_string());
        screen.add_line("".to_string());

        if play_data.song_position < play_data.song_length as usize && order[play_data.song_position] < patterns.len() as u8 {
            let pattern = &patterns[order[play_data.song_position] as usize];
            let mut first_row;
            let last_row;
            if play_data.row < 10 {
                first_row = 0;
                last_row = 20;
            } else {
                first_row = play_data.row - 10;

                if play_data.row + 10 > pattern.rows.len() {
                    first_row = pattern.rows.len() - 20;
                    last_row = pattern.rows.len();
                } else {
                   last_row = play_data.row + 10;
                }
            }
            for i in first_row..last_row {
                if i == play_data.row {
                    screen.add_line_with_color(pattern.rows[i].to_string(), RGB{
                        r: 50,
                        g: 50,
                        b: 128
                    });
                } else {
                    screen.add_line_with_color(pattern.rows[i].to_string(), RGB{
                        r: 10,
                        g: 10,
                        b: 10
                    });
                }
            }
        }

        // screen.add_line(format!("{:?}", view_port));
        // display(Self::hide());
        // display(Self::move_to(0, 0));

        for y in view_port.y1..(view_port.y1+view_port.height as isize) {
            if y < 0 || y as usize >= screen.lines.len() {
                display("".to_string());
                continue;
            }

            let line = &screen.lines[y as usize];
            let len = if line.data.is_empty() { 0 } else { line.data.len() - 1 };
            // let width = min(view_port.width, len);
            if view_port.x1.abs() as usize > len {
                display("".to_string());
                continue;
            }
            let start = max(view_port.x1, 0);
            let mut preamble: String = "".to_string();
            if view_port.x1 <= 0 { for _ in 0..(view_port.x1.abs() as usize) { preamble.push(' '); } };
            let end = min(len, (view_port.x1 + (view_port.width as isize)) as usize);
            let range = start as usize..end;
            let black = RGB {
                r: 0,
                g: 0,
                b: 0
            };
            if black != line.background {
                display_with_background(String::from(preamble + &line.data[range]), line.background);
            } else {
                display(String::from(preamble + &line.data[range]));
            }
        }
    }
}
