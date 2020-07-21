use xmplayer::song::Song;

fn color(color: RGB, str: &str) -> String {
    format!("\x1b[38;2;{};{};{}m{}", color.R, color.G, color.B, str)
}

fn range(pos: u32, start: u32, end: u32, width: usize) -> String {
    let mut result : String = String::from("");
    let mut indicator_pos = ((pos - start) as f32 / (end - start) as f32 * (width) as f32) as usize;
    if indicator_pos > width {
        indicator_pos = width;
    }
    for i in 0..indicator_pos {
        result += "-";
    }
    result += "=";
    for i in indicator_pos+1..(width+1) as usize {
        result += "-";
    }
    result
}

fn range_with_color(pos: u32, start: u32, end: u32, width: usize, colors: &[RGB]) -> String {
    let mut result : String = String::from("");
    if pos == 0 {
        for i in 0..width+1 {
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
    for i in indicator_pos+1..(width+1) as usize {
        result += " "; //&*Self::color(colors[i], "-");
    }
    result += "\x1b[0m";
    result
}


fn display(s: &Song, cur_tick: usize) {
    let colors: [RGB; 12] = [
        RGB {R:   0, G: 120, B:   0},
        RGB {R:   0, G: 140, B:   0},
        RGB {R:   0, G: 160, B:   0},
        RGB {R:   0, G: 180, B:   0},
        RGB {R: 180, G: 180, B:   0},
        RGB {R: 195, G: 195, B:   0},
        RGB {R: 210, G: 210, B:   0},
        RGB {R: 225, G: 225, B:   0},
        RGB {R: 225, G:  64, B:   0},
        RGB {R: 225, G:  64, B:   0},
        RGB {R: 225, G:  64, B:   0},
        RGB {R: 225, G:  64 , B:   0},
    ];
    let first_tick = s.tick == 0;
    if let Err(_e) = crossterm::execute!(stdout(), Hide, MoveTo(0,0)) {}
    println!("duration in frames: {:5} duration in ms: {:5} tick: {:3} pos: {:3X}/{:<3X}  row: {:3}/{:<3} bpm: {:3} speed: {:3}, buf: {}",
             s.bpm.tick_duration_in_frames, s.bpm.tick_duration_in_ms, s.tick, s.song_position, s.song_data.song_length, s.row,
             s.song_data.patterns[s.song_data.pattern_order[s.song_position] as usize].rows.len() - 1,
             s.bpm.bpm, s.speed,
             Song::range(cur_tick as u32, 0, s.bpm.tick_duration_in_frames as u32, 15),

    );
    if let Err(_e) = crossterm::execute!(stdout(), MoveTo(0,1)) {}

    println!("on | channel |         instrument         |frequency|   volume   |sample_position| note | period |  chan vol  |   envvol   | globalvol  |   fadeout  | panning |");

    let mut idx = 0u32;
    for channel in &s.channels {
        idx = idx + 1;
//            if idx != 1  {continue;}


        if channel.on {
            let final_vol =
                (channel.volume.get_volume() as f32 / 64.0) *
                    (channel.volume.envelope_vol as f32 / 16384.0) *
                    (channel.volume.global_vol as f32 / 64.0) *
                    (channel.volume.fadeout_vol as f32 / 65536.0);

            println!("{:3}| {:7} | {:26} |  {:<6} |{:11}|{:14}| {:4} | {:7}|{:11}|{:11}|{:11}|{:11}|{:8}|      ",
                     if channel.force_off { " x" } else if channel.on { "on" } else { "off" }, idx, channel.instrument.idx.to_string() + ": " + channel.instrument.name.trim(),
                     if channel.on { (channel.frequency + channel.frequency_shift) as u32 } else { 0 },
                     Song::range_with_color((final_vol * 12.0) as u32, 0, 12, 11, &colors),
                     Song::range((channel.sample_position + channel.du * s.bpm.tick_duration_in_frames as f32) as u32, 0, channel.sample.length - 1, 14),
                     channel.note.to_string(), channel.note.period,
                     Song::range_with_color(channel.volume.get_volume() as u32, 0, 64, 11, &colors),
                     Song::range_with_color(channel.volume.envelope_vol as u32, 0, 16384, 11, &colors),
                     Song::range_with_color(channel.volume.global_vol as u32, 0, 64, 11, &colors),
                     Song::range_with_color(channel.volume.fadeout_vol as u32, 0, 65536, 11, &colors),
                     Song::range(channel.panning.final_panning as u32, 0, 255, 8),
            );
        } else {
            println!("{:3}| {:7} | {:26} |  {:<6} |{:12}| {:14}| {:5}| {:7}|{:12}|{:12}|{:12}|{:12}| {:8}|      ", "off", idx, "", "", "",
                     "", "", "", "", "", "", "", "");
        }
    }
    if let Err(_e) = crossterm::execute!(stdout(), Show) {}
}
