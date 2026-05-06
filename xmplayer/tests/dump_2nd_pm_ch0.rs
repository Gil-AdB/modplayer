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

fn trace_voice_volume(path: &str, target_order: usize) {
    let (song_handle, _) = match SongState::new(path) {
        Ok(s) => s,
        Err(e) => { eprintln!("can't open {}: {}", path, e); return; }
    };
    let mut song = song_handle.get_song().lock().unwrap();
    println!("=== {}  voice-vol trace, order {} ===", path, target_order);

    // Step until we reach target_order, then trace tick by tick.
    let mut last_print_row = usize::MAX;
    loop {
        if song.song_position > target_order { break; }
        let was_in_target = song.song_position == target_order;
        let pre_tick = song.tick;
        let pre_row = song.row;
        // Capture pattern row info BEFORE process_tick (in case process_tick
        // would advance row — it doesn't, but just to be safe).
        let pat_idx_pre = song.song_data.pattern_order[song.song_position] as usize;
        let pat_row_pre = song.song_data.patterns[pat_idx_pre].rows[pre_row].channels[0].clone();
        song.process_tick();
        if was_in_target {
            // Print POST process_tick state — this is what gets mixed into audio.
            let ch = &song.channels[0];
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
    trace_voice_volume("/Users/gil-ad/work/modplayer/scratch/2ND_PM.S3M", 3);
    trace_voice_volume("/Users/gil-ad/work/modplayer/scratch/2ND_PM.xm",  3);
}
