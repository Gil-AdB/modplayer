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
                println!("Instrument {}: {} ({} samples) GlobalVol: {} NNA: {} DCT: {} DCA: {}", 
                    i, inst.name, inst.samples.len(), inst.global_volume, inst.nna, inst.dct, inst.dca);
                println!("  Vol Env: active={} size={} sustain={} loop={}-{}", 
                    inst.volume_envelope.on, inst.volume_envelope.size, inst.volume_envelope.sustain_point,
                    inst.volume_envelope.loop_start_point, inst.volume_envelope.loop_end_point);
                // ...
            }
            
            if !song_data.patterns.is_empty() {
                for (p_idx, pat) in song_data.patterns.iter().enumerate().take(2) {
                    println!("Pattern {} rows: {}", p_idx, pat.rows.len());
                    for (r_idx, row) in pat.rows.iter().enumerate() {
                        let mut has_data = false;
                        for p in row.channels.iter() {
                            if p.note != 0 || p.instrument != 0 || p.volume != 255 || p.effect != 0 {
                                has_data = true; break;
                            }
                        }
                        if !has_data { continue; }

                        print!("Row {:02}: ", r_idx);
                        for (c_idx, p) in row.channels.iter().enumerate() {
                            if p.note != 0 || p.instrument != 0 || p.volume != 255 || p.effect != 0 {
                                print!("|Ch{:02} N:{:3} I:{:2} V:{:3} E:{:02X} P:{:02X} ", c_idx, p.note, p.instrument, p.volume, p.effect, p.effect_param);
                            }
                        }
                        println!("|");
                    }
                }
            }
        }
        Err(e) => {
            println!("Error: {}", e);
        }
    }
}
