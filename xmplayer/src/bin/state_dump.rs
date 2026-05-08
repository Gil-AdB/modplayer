// Generalized per-tick state dumper for any module file. Replaces the
// previous ad-hoc per-investigation tests in tests/state_dump_test.rs
// and tests/dump_2nd_pm_ch0.rs.
//
// Usage:
//   cargo run --bin state_dump -- <module_path> [options]
//
//   --order <N>         Only dump this song-order index (hex 0x13 or decimal).
//                       Repeatable. Default: every order.
//   --rows <START..END> Row range, half-open, hex or decimal (e.g. 0x00..0x10
//                       or 0..16). Default: every row.
//   --channels <list>   Comma-separated 0-based channel indices to print
//                       (e.g. 6,7). Default: every channel.
//   --all-ticks         Print every tick within the row window. Default:
//                       first tick of each row only.
//   --output <path>     Write to file instead of stdout.
//   --help              Print usage.
//
// Examples:
//   state_dump 2ND_PM.S3M --order 0x13 --rows 0..16 --channels 6,7 --all-ticks
//   state_dump 2ND_PM.S3M --order 0x23 --rows 0x30..0x36 --all-ticks
//   state_dump 2ND_PM.xm  --order 0x13 --output /tmp/order13.xm.txt

use shared_sync_primitives::TripleBuffer;
use std::env;
use std::fs::File;
use std::io::{stdout, Write};
use xmplayer::module_reader::read_module;
use xmplayer::song::test_dump::dump_tick;
use xmplayer::song::Song;

struct Args {
    path: String,
    orders: Vec<usize>,         // empty = all
    row_range: Option<(usize, usize)>, // None = all (half-open [start, end))
    channels: Vec<usize>,       // empty = all
    all_ticks: bool,
    output: Option<String>,
}

fn parse_int(s: &str) -> Option<usize> {
    if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        usize::from_str_radix(rest, 16).ok()
    } else {
        s.parse::<usize>().ok()
    }
}

fn parse_args() -> Result<Args, String> {
    let raw: Vec<String> = env::args().skip(1).collect();
    if raw.iter().any(|a| a == "--help" || a == "-h") || raw.is_empty() {
        return Err("usage".to_string());
    }
    let mut path: Option<String> = None;
    let mut orders = Vec::new();
    let mut row_range: Option<(usize, usize)> = None;
    let mut channels: Vec<usize> = Vec::new();
    let mut all_ticks = false;
    let mut output: Option<String> = None;

    let mut i = 0;
    while i < raw.len() {
        let a = &raw[i];
        match a.as_str() {
            "--order" => {
                i += 1;
                let v = raw.get(i).ok_or("--order needs a value")?;
                orders.push(parse_int(v).ok_or_else(|| format!("bad --order: {}", v))?);
            }
            "--rows" => {
                i += 1;
                let v = raw.get(i).ok_or("--rows needs a START..END")?;
                let (s, e) = v.split_once("..").ok_or("--rows must be START..END")?;
                let start = parse_int(s).ok_or_else(|| format!("bad --rows start: {}", s))?;
                let end   = parse_int(e).ok_or_else(|| format!("bad --rows end: {}", e))?;
                row_range = Some((start, end));
            }
            "--channels" => {
                i += 1;
                let v = raw.get(i).ok_or("--channels needs a list")?;
                for piece in v.split(',') {
                    let n = parse_int(piece.trim())
                        .ok_or_else(|| format!("bad channel: {}", piece))?;
                    channels.push(n);
                }
            }
            "--all-ticks" => all_ticks = true,
            "--output" => {
                i += 1;
                output = Some(raw.get(i).ok_or("--output needs a path")?.clone());
            }
            "--help" | "-h" => return Err("usage".to_string()),
            other if other.starts_with("--") => return Err(format!("unknown flag: {}", other)),
            _ => {
                if path.is_some() {
                    return Err(format!("unexpected positional: {}", a));
                }
                path = Some(a.clone());
            }
        }
        i += 1;
    }
    let path = path.ok_or("missing module path")?;
    Ok(Args { path, orders, row_range, channels, all_ticks, output })
}

fn print_usage() {
    eprintln!("usage: state_dump <path> [--order N]... [--rows S..E] [--channels a,b]");
    eprintln!("                  [--all-ticks] [--output FILE]");
    eprintln!();
    eprintln!("  N, S, E and channel indices accept decimal or 0x hex.");
    eprintln!("  Without --order, every order is dumped.");
    eprintln!("  Without --all-ticks, only first-tick of each row is dumped.");
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) if e == "usage" => { print_usage(); return; }
        Err(e) => { eprintln!("error: {}", e); print_usage(); std::process::exit(2); }
    };

    let song_data = match read_module(&args.path) {
        Ok(d) => d,
        Err(e) => { eprintln!("failed to read {}: {:?}", args.path, e); std::process::exit(1); }
    };

    let (_reader, writer) = TripleBuffer::new().split();
    let mut song = Song::new(&song_data, writer, 48000.0);

    // Output sink.
    let mut out: Box<dyn Write> = match &args.output {
        Some(p) => match File::create(p) {
            Ok(f) => Box::new(f),
            Err(e) => { eprintln!("failed to create {}: {}", p, e); std::process::exit(1); }
        },
        None => Box::new(stdout()),
    };

    let song_length = song_data.song_length as usize;
    let order_filter_active = !args.orders.is_empty();
    let want_order = |o: usize| -> bool {
        if !order_filter_active { return true; }
        args.orders.iter().any(|x| *x == o)
    };

    // Walk the song tick-by-tick. Stop once we've passed every requested
    // order (so the user doesn't wait for the whole song when they asked
    // for one slice).
    let max_requested_order = args.orders.iter().copied().max();
    loop {
        let pos = song.song_position;
        if let Some(max_o) = max_requested_order {
            if pos > max_o { break; }
        }
        if pos >= song_length { break; }

        let row = song.row;
        let in_row_window = match args.row_range {
            Some((s, e)) => row >= s && row < e,
            None => true,
        };
        let in_target_order = want_order(pos);
        let at_first_tick = song.tick == 0;

        // Decide whether to print this tick.
        let print_now = in_target_order
            && (in_row_window || (args.row_range.is_none() && at_first_tick))
            && (args.all_ticks || at_first_tick);

        // Run effects for the current tick first, then dump (so the dump
        // reflects post-effect state).
        song.process_tick();

        if print_now {
            let dump = dump_tick(&song);
            // If channel filter is active, redact (skip) other voice rows.
            let s = if args.channels.is_empty() {
                dump.to_string()
            } else {
                let header = format!(
                    "[Order {:03} | Row {:03} | Tick {:03}] (Voices: {} / Channels: {})\n",
                    dump.song_position, dump.row, dump.tick,
                    dump.active_voices, dump.active_channels,
                );
                let mut s = header;
                let mut sorted = dump.voices.clone();
                sorted.sort_by_key(|v| v.channel_idx);
                for v in &sorted {
                    if !v.is_on { continue; }
                    if !args.channels.contains(&v.channel_idx) { continue; }
                    s.push_str(&format!(
                        "  Ch {:02}: ON | Inst {:02} | Samp {:02} | {} | Pos {:>9.3} | dU {:>7.3} | Vol {:>7.3} | Pan {:03} ({:03}) | Eff {:02x} {:02x}\n",
                        v.channel_idx, v.instrument, v.sample, v.note_str, v.sample_pos,
                        v.du, v.output_volume, v.panning, v.final_panning,
                        v.effect, v.effect_param,
                    ));
                }
                s
            };
            let _ = out.write_all(s.as_bytes());
        }

        if !song.next_tick() { break; }
    }
}
