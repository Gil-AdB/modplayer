use xmplayer::song::PlayData;
use std::io::{stdout, Write};
use std::cmp::max;
use xmplayer::instrument::Instrument;


#[derive(Copy, Clone)]
struct RGB {
    r: u8,
    g: u8,
    b: u8,
}

pub(crate) struct Display {}

impl Display {
    fn color(color: RGB, str: &str) -> String {
        format!("\x1b[38;2;{};{};{}m{}", color.r, color.g, color.b, str)
    }

    fn hide() -> String {
        "\x1b[?25l".to_string()
    }

    fn show() -> String {
        "\x1b[?25h".to_string()
    }

    fn move_to(x: usize, y:usize) -> String {
        format!("\x1b[{};{}H", x+1, y+1)
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
        result += "\x1b[0m";
        result
    }


    pub(crate) fn display(play_data: &PlayData, instruments: &Vec<Instrument>, display: &mut dyn FnMut(String)) {
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
        display(Self::hide());
        display(Self::move_to(0, 0));
        display(format!("'{}' duration in frames: {:5} duration in ms: {:5} tick: {:3} pos: {:3X}/{:<3X}  row: {:3X}/{:<3X} bpm: {:3} speed: {:3} filter: {:5}", play_data.name,
                        play_data.tick_duration_in_frames, play_data.tick_duration_in_ms, play_data.tick, play_data.song_position, play_data.song_length - 1, play_data.row,
                        play_data.pattern_len,
                        play_data.bpm, play_data.speed,
                        play_data.filter
        ));
        // display(Self::move_to(0, 2));

        display("on |channel|            instrument            |frequency|   volume   |sample_position| note | period |  chan vol  |   envvol   | globalvol  |   fadeout  | panning |".to_string());

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

                display(format!("{:3}| {:5} | {:2}: {:28} |  {:<6} |{:11}|{:14}| {:4} | {:7}|{:11}|{:11}|{:11}|{:11}|{:8}|      ",
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
                display(format!("{:3}| {:5} | {:32} |  {:<6} |{:12}| {:14}| {:5}| {:7}|{:12}|{:12}|{:12}|{:12}| {:8}|      ", "off", idx, "", "", "",
                                "", "", "", "", "", "", "", ""));
            }
        }
        display(Self::show());
    }
}
