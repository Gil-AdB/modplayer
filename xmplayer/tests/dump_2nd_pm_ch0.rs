// Dump order-3 channel-0 of 2ND_PM.{S3M,xm} to track down the
// volume-update-too-slow bug. Ignored by default; run explicitly with
//   cargo test -p xmplayer --test dump_2nd_pm_ch0 -- --ignored --nocapture

use xmplayer::song_state::SongState;

fn dump_one(path: &str) {
    let (song_handle, _) = match SongState::new(path) {
        Ok(s) => s,
        Err(e) => { eprintln!("can't open {}: {}", path, e); return; }
    };
    let song = song_handle.get_song().lock().unwrap();
    let pat_idx = song.song_data.pattern_order[3] as usize;
    let pat = &song.song_data.patterns[pat_idx];
    println!("=== {}  order=3 pattern={} rows={} ===", path, pat_idx, pat.rows.len());
    for (r, row) in pat.rows.iter().enumerate() {
        let c = &row.channels[0];
        if c.note != 0 || c.instrument != 0 || c.volume != 255 || c.effect != 0 || c.effect_param != 0 {
            println!("  row {:02}  note {:>3}  inst {:>3}  vol {:>3}  eff {:02x} {:02x}",
                     r, c.note, c.instrument, c.volume, c.effect, c.effect_param);
        }
    }
}

#[test]
#[ignore = "Manual: dumps order-3 channel-0 of scratch/2ND_PM.{S3M,xm}"]
fn dump_2nd_pm_order3_ch0() {
    // Test runs from the workspace root.
    dump_one("/Users/gil-ad/work/modplayer/scratch/2ND_PM.S3M");
    dump_one("/Users/gil-ad/work/modplayer/scratch/2ND_PM.xm");
}

fn dump_all_channels_at_order(path: &str, order: usize) {
    let (song_handle, _) = match SongState::new(path) {
        Ok(s) => s,
        Err(e) => { eprintln!("can't open {}: {}", path, e); return; }
    };
    let song = song_handle.get_song().lock().unwrap();
    let pat_idx = song.song_data.pattern_order[order] as usize;
    let pat = &song.song_data.patterns[pat_idx];
    let n_chan = pat.rows[0].channels.len();
    println!("=== {}  order={} pattern={} rows={} channels={} ===", path, order, pat_idx, pat.rows.len(), n_chan);
    for ch in 0..n_chan {
        let mut any = false;
        for (r, row) in pat.rows.iter().enumerate() {
            let c = &row.channels[ch];
            if c.note != 0 || c.instrument != 0 || c.volume != 255 || c.effect != 0 || c.effect_param != 0 {
                if !any { println!("--- ch {} ---", ch); any = true; }
                println!("  row {:02}  note {:>3}  inst {:>3}  vol {:>3}  eff {:02x} {:02x}",
                         r, c.note, c.instrument, c.volume, c.effect, c.effect_param);
            }
        }
    }
}

#[test]
#[ignore = "Manual: dump all channels of order 12 in 2ND_PM.S3M"]
fn dump_2nd_pm_order12_all() {
    dump_all_channels_at_order("/Users/gil-ad/work/modplayer/scratch/2ND_PM.S3M", 12);
}

#[test]
#[ignore = "Manual: dump all channels of order 0x13 in 2ND_PM.S3M"]
fn dump_2nd_pm_order_0x13_all() {
    dump_all_channels_at_order("/Users/gil-ad/work/modplayer/scratch/2ND_PM.S3M", 0x13);
}

#[test]
#[ignore = "Manual: find orders where inst 33 (0x21) appears with note ~70"]
fn find_inst_33_a5() {
    let path = "/Users/gil-ad/work/modplayer/scratch/2ND_PM.S3M";
    let (song_handle, _) = match SongState::new(path) { Ok(s) => s, Err(e) => { eprintln!("{}", e); return; } };
    let song = song_handle.get_song().lock().unwrap();
    for (order_idx, &pat_idx) in song.song_data.pattern_order.iter().enumerate() {
        let pat = &song.song_data.patterns[pat_idx as usize];
        for (r, row) in pat.rows.iter().enumerate() {
            for (ch, c) in row.channels.iter().enumerate() {
                if c.instrument == 33 && c.note >= 60 && c.note <= 80 {
                    println!("order {:>3} pat {:>3} row {:>2} ch {} : note {} inst {} vol {} eff {:02x}{:02x}",
                             order_idx, pat_idx, r, ch, c.note, c.instrument, c.volume, c.effect, c.effect_param);
                }
            }
        }
    }
}

fn trace_voice_volume(path: &str, target_order: usize, channel_idx: usize) {
    let (song_handle, _) = match SongState::new(path) {
        Ok(s) => s,
        Err(e) => { eprintln!("can't open {}: {}", path, e); return; }
    };
    let mut song = song_handle.get_song().lock().unwrap();
    println!("=== {}  voice-vol trace, order {}, ch {} ===", path, target_order, channel_idx);

    let mut last_print_row = usize::MAX;
    loop {
        if song.song_position > target_order { break; }
        let was_in_target = song.song_position == target_order;
        let pre_tick = song.tick;
        let pre_row = song.row;
        let pat_idx_pre = song.song_data.pattern_order[song.song_position] as usize;
        let pat_row_pre = song.song_data.patterns[pat_idx_pre].rows[pre_row].channels[channel_idx].clone();
        song.process_tick();
        if was_in_target {
            let ch = &song.channels[channel_idx];
            let v = ch.voice_idx.map(|i| (i, &song.voices[i]));
            let new_row = pre_row != last_print_row;
            if new_row {
                println!(
                    "row {:02}  pattern: note={:>3} inst={:>3} vol={:>3} eff={:02x}{:02x}",
                    pre_row, pat_row_pre.note, pat_row_pre.instrument, pat_row_pre.volume,
                    pat_row_pre.effect, pat_row_pre.effect_param,
                );
                last_print_row = pre_row;
            }
            match v {
                None => println!("  tick {} | (no host voice)", pre_tick),
                Some((idx, v)) => println!(
                    "  tick {} | voice[{:>2}] vol.volume={:>3} output={:.4} | ch.cvol={:>3} pan={:>3}",
                    pre_tick, idx,
                    v.volume.volume, v.volume.output_volume,
                    ch.channel_volume, v.panning.final_panning,
                ),
            }
        }
        if !song.next_tick() { break; }
    }
}

#[test]
#[ignore = "Manual: tick-by-tick voice-volume trace for ch 0 across order 3"]
fn trace_2nd_pm_order3_ch0_voice_volume() {
    trace_voice_volume("/Users/gil-ad/work/modplayer/scratch/2ND_PM.S3M", 3, 0);
    trace_voice_volume("/Users/gil-ad/work/modplayer/scratch/2ND_PM.xm",  3, 0);
}

#[test]
#[ignore = "Manual: trace ch 7 (display CH08) across order 18 (user's '0x12')"]
fn trace_2nd_pm_order18_ch7_voice_volume() {
    trace_voice_volume("/Users/gil-ad/work/modplayer/scratch/2ND_PM.S3M", 18, 7);
    trace_voice_volume("/Users/gil-ad/work/modplayer/scratch/2ND_PM.xm",  18, 7);
}

fn trace_period_freq(path: &str, target_order: usize, channels: &[usize], rows: std::ops::Range<usize>) {
    let (song_handle, _) = match SongState::new(path) {
        Ok(s) => s,
        Err(e) => { eprintln!("can't open {}: {}", path, e); return; }
    };
    let mut song = song_handle.get_song().lock().unwrap();
    println!("=== {}  period/freq trace, order {}, rows {:?}, ch {:?} ===",
             path, target_order, rows, channels);

    loop {
        if song.song_position > target_order { break; }
        let was_in_target = song.song_position == target_order && rows.contains(&song.row);
        let pre_tick = song.tick;
        let pre_row = song.row;
        let pat_idx_pre = song.song_data.pattern_order[song.song_position] as usize;
        let pat_rows = &song.song_data.patterns[pat_idx_pre].rows[pre_row].channels;
        let row_pat: Vec<_> = channels.iter().map(|&c| pat_rows[c].clone()).collect();
        song.process_tick();
        if was_in_target {
            for (i, &c) in channels.iter().enumerate() {
                let p = &row_pat[i];
                let ch = &song.channels[c];
                let v = ch.voice_idx.map(|i| &song.voices[i]);
                if pre_tick == 0 {
                    println!(
                        "row {:02} ch{} pat: note={:>3} inst={:>3} vol={:>3} eff={:02x}{:02x}",
                        pre_row, c, p.note, p.instrument, p.volume, p.effect, p.effect_param,
                    );
                }
                match v {
                    None => println!("  tick {} ch{} | (no host voice)", pre_tick, c),
                    Some(v) => println!(
                        "  tick {} ch{} | period={:>5} freq={:>9.2} target_period={:>5}",
                        pre_tick, c,
                        ch.note.period, v.frequency,
                        ch.porta_to_note.target_note.period,
                    ),
                }
            }
        }
        if !song.next_tick() { break; }
    }
}

#[test]
#[ignore = "Manual: order 13 channels 6+7 (display CH07/CH08) period+freq trace"]
fn trace_2nd_pm_order13_ch67_period() {
    trace_period_freq(
        "/Users/gil-ad/work/modplayer/scratch/2ND_PM.S3M",
        13, &[6, 7], 0x08..0x14,
    );
}

#[test]
#[ignore = "Manual: order 0x13 (=19) channels 6+7 period+freq trace"]
fn trace_2nd_pm_order_0x13_ch67_period() {
    trace_period_freq(
        "/Users/gil-ad/work/modplayer/scratch/2ND_PM.S3M",
        0x13, &[6, 7], 0x08..0x14,
    );
}

fn trace_full_state(path: &str, target_order: usize, channels: &[usize], rows: std::ops::Range<usize>) {
    let (song_handle, _) = match SongState::new(path) {
        Ok(s) => s,
        Err(e) => { eprintln!("can't open {}: {}", path, e); return; }
    };
    let mut song = song_handle.get_song().lock().unwrap();
    println!("=== {}  full-state trace, order 0x{:x}, rows {:?}, ch {:?} ===",
             path, target_order, rows, channels);

    loop {
        if song.song_position > target_order { break; }
        let was_in_target = song.song_position == target_order && rows.contains(&song.row);
        let pre_tick = song.tick;
        let pre_row = song.row;
        let pat_idx_pre = song.song_data.pattern_order[song.song_position] as usize;
        let pat_rows = &song.song_data.patterns[pat_idx_pre].rows[pre_row].channels;
        let row_pat: Vec<_> = channels.iter().map(|&c| pat_rows[c].clone()).collect();
        song.process_tick();
        if was_in_target {
            for (i, &c) in channels.iter().enumerate() {
                let p = &row_pat[i];
                let ch = &song.channels[c];
                let v = ch.voice_idx.map(|i| (i, &song.voices[i]));
                if pre_tick == 0 {
                    println!(
                        "row {:02x} ch{} pat: note={:>3} inst={:>3} vol={:>3} eff={:02x}{:02x}  | song.speed={} bpm={}",
                        pre_row, c, p.note, p.instrument, p.volume, p.effect, p.effect_param,
                        song.speed, song.bpm.bpm,
                    );
                }
                match v {
                    None => println!("  t{} ch{} | (no host voice)", pre_tick, c),
                    Some((vi, v)) => println!(
                        "  t{} ch{} | v{:>2} on={} freq={:>9.2} pos={:>10.1} period={:>4} vol={:>3} out={:.4} fadeout={:>5} env={:>4} chvol={:>3}",
                        pre_tick, c, vi, v.on as u8, v.frequency, v.sample_position,
                        ch.note.period, v.volume.volume, v.volume.output_volume,
                        v.volume.fadeout_vol, v.volume.envelope_vol, ch.channel_volume,
                    ),
                }
            }
        }
        if !song.next_tick() { break; }
    }
}

#[test]
#[ignore = "Manual: order 0x14 ch7 (display CH08) full state across rows 0x30-0x40"]
fn trace_2nd_pm_order_0x14_ch7_full() {
    trace_full_state(
        "/Users/gil-ad/work/modplayer/scratch/2ND_PM.S3M",
        0x14, &[7], 0x30..0x40,
    );
}

#[test]
#[ignore = "Manual: order 0x23 ch3 (display CH04) effect 13D (SetSpeed 0x3D)"]
fn trace_2nd_pm_order_0x23_ch3_speed() {
    trace_full_state(
        "/Users/gil-ad/work/modplayer/scratch/2ND_PM.S3M",
        0x23, &[3], 0x00..0x14,
    );
}

#[test]
#[ignore = "Manual: order 0x23 row 0x32 ch3 — F-4 with SD2 note-delay (vol=0C)"]
fn trace_2nd_pm_order_0x23_row_0x32_f4() {
    trace_full_state(
        "/Users/gil-ad/work/modplayer/scratch/2ND_PM.S3M",
        0x23, &[3], 0x30..0x36,
    );
}

#[test]
#[ignore = "Manual: order 0x23 ch3 wider — rows 0x28-0x38 around the SD note-delays"]
fn trace_2nd_pm_order_0x23_ch3_wide() {
    trace_full_state(
        "/Users/gil-ad/work/modplayer/scratch/2ND_PM.S3M",
        0x23, &[3], 0x28..0x39,
    );
}

#[test]
#[ignore = "Manual: find ALL rows where eff=0x13 (S extended) on any channel"]
fn find_all_extended_s() {
    let path = "/Users/gil-ad/work/modplayer/scratch/2ND_PM.S3M";
    let (h, _) = match SongState::new(path) { Ok(s) => s, Err(_) => return };
    let song = h.get_song().lock().unwrap();
    for (order_idx, &pat_idx) in song.song_data.pattern_order.iter().enumerate() {
        let pat = &song.song_data.patterns[pat_idx as usize];
        for (r, row) in pat.rows.iter().enumerate() {
            for (ch, c) in row.channels.iter().enumerate() {
                if c.effect == 0x13 {
                    let high = c.effect_param >> 4;
                    let low = c.effect_param & 0x0F;
                    let kind = match high {
                        0x0 => "S0x set filter",
                        0x1 => "S1x set glissando",
                        0x2 => "S2x set finetune",
                        0x3 => "S3x set vibrato wave",
                        0x4 => "S4x set tremolo wave",
                        0x8 => "S8x set pan",
                        0xB => "SBx pattern loop",
                        0xC => "SCx note cut",
                        0xD => "SDx note delay",
                        0xE => "SEx pattern delay",
                        0xF => "SFx funk repeat",
                        _ => "S?x unknown",
                    };
                    println!("order=0x{:02x} row=0x{:02x} ch={} note={} inst={} vol=0x{:02X} param=0x{:02X} ({} arg=0x{:X})",
                             order_idx, r, ch, c.note, c.instrument, c.volume, c.effect_param, kind, low);
                }
            }
        }
    }
}

#[test]
#[ignore = "Manual: dump inst 25 (the one used in the SDx chirp rows)"]
fn dump_inst_25_params() {
    let (h, _) = SongState::new("/Users/gil-ad/work/modplayer/scratch/2ND_PM.S3M").unwrap();
    let song = h.get_song().lock().unwrap();
    let inst = &song.song_data.instruments[25];
    println!("inst 25 name={}", inst.name);
    for (i, s) in inst.samples.iter().enumerate() {
        println!("  sample {} name='{}' len={} loop_start={} loop_end={} loop_type={:?} relnote={} finetune={} vol={}",
                 i, s.name, s.length, s.loop_start, s.loop_end, s.loop_type, s.relative_note, s.finetune, s.volume);
    }
}

#[test]
#[ignore = "Manual: dump inst 33 sample params"]
fn dump_inst_33_params() {
    let (h, _) = SongState::new("/Users/gil-ad/work/modplayer/scratch/2ND_PM.S3M").unwrap();
    let song = h.get_song().lock().unwrap();
    let inst = &song.song_data.instruments[33];
    println!("inst 33 name={}", inst.name);
    for (i, s) in inst.samples.iter().enumerate() {
        println!("  sample {} name={} len={} relnote={} finetune={} vol={} c2spd_inferred=8363*2^(({}+{}/128)/12) = {:.2}",
                 i, s.name, s.length, s.relative_note, s.finetune, s.volume,
                 s.relative_note, s.finetune,
                 8363.0 * 2.0_f64.powf(((s.relative_note as f64) + (s.finetune as f64) / 128.0) / 12.0));
    }
}
