use xmplayer::module_reader::read_module;

#[test]
fn debug_it_patterns() {
    let filename = "/Users/gil-ad/Downloads/mods/a-windf.it";
    
    match read_module(filename) {
        Ok(song_data) => {
            println!("Song: {}", song_data.name);
            println!("Channels: {}", song_data.channel_count);
            println!("Instruments: {}", song_data.instrument_count);
            println!("Patterns: {}", song_data.patterns.len());
            
            for (i, inst) in song_data.instruments.iter().enumerate().take(5) {
                println!("Instrument {}: {} ({} samples) GlobalVol: {}", i, inst.name, inst.samples.len(), inst.global_volume);
                println!("  Vol Env: active={} size={} sustain={} loop={}-{}", 
                    inst.volume_envelope.on, inst.volume_envelope.size, inst.volume_envelope.sustain_point,
                    inst.volume_envelope.loop_start_point, inst.volume_envelope.loop_end_point);
                for (p_idx, p) in inst.volume_envelope.points.iter().enumerate().take(inst.volume_envelope.size as usize) {
                    print!("({}:{}) ", p.frame, p.value);
                }
                println!();
                for (s_idx, sample) in inst.samples.iter().enumerate().take(1) {
                    let mut min = 1.0f32;
                    let mut max = -1.0f32;
                    let mut sum = 0.0f32;
                    for &val in &sample.data {
                        if val < min { min = val; }
                        if val > max { max = val; }
                        sum += val.abs();
                    }
                    println!("  Sample {}: len={} min={:.3} max={:.3} avg_abs={:.3}", s_idx, sample.data.len(), min, max, sum / (sample.data.len() + 1) as f32);
                }
            }
            
            if !song_data.patterns.is_empty() {
                let pat = &song_data.patterns[0];
                println!("Pattern 0 rows: {}", pat.rows.len());
                for (r_idx, row) in pat.rows.iter().enumerate().take(32) {
                    print!("Row {:02}: ", r_idx);
                    for p in row.channels.iter().take(8) {
                        if p.note != 0 || p.instrument != 0 || p.volume != 255 || p.effect != 0 {
                            print!("| N:{:3} I:{:2} V:{:3} E:{:02X} P:{:02X} ", p.note, p.instrument, p.volume, p.effect, p.effect_param);
                        } else {
                            print!("| .................. ");
                        }
                    }
                    println!("|");
                }
            }
        }
        Err(e) => {
            println!("Error: {}", e);
        }
    }
}
