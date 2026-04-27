use xmplayer::module_reader::read_module;

fn main() {
    let filename = "/Users/gil-ad/Downloads/mods/strshine.s3m";
    let song_data = read_module(filename).expect("Failed to read module");
    let order_idx = 10;
    let pat_idx = song_data.pattern_order[order_idx] as usize;
    let pattern = &song_data.patterns[pat_idx];
    
    println!("Order {} -> Pattern {}", order_idx, pat_idx);
    for (r_idx, row) in pattern.rows.iter().enumerate() {
        let mut has_data = false;
        for ch in &row.channels {
            if ch.note != 0 || ch.instrument != 0 || ch.volume != 255 || ch.effect != 0 {
                has_data = true;
                break;
            }
        }
        if !has_data { continue; }
        
        print!("Row {:02}: ", r_idx);
        for (c_idx, ch) in row.channels.iter().enumerate() {
            if ch.note != 0 || ch.instrument != 0 || ch.volume != 255 || ch.effect != 0 {
                print!("| Ch{:02} N:{:3} I:{:2} V:{:3} E:{:02X} P:{:02X} ", c_idx, ch.note, ch.instrument, ch.volume, ch.effect, ch.effect_param);
            }
        }
        println!("|");
    }
}
