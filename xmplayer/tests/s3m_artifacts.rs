use xmplayer::module_reader::read_module;
use xmplayer::song::Song;
use shared_sync_primitives::TripleBuffer;

#[test]
fn test_2nd_reality_s3m_artifacts() {
    let filename = "/Users/gil-ad/Downloads/mods/2nd_reality.s3m";
    let song_data = read_module(filename).expect("Failed to read module");
    
    let triple_buffer = TripleBuffer::new();
    let (_, triple_buffer_writer) = triple_buffer.split();
    
    let mut song = Song::new(&song_data, triple_buffer_writer, 44100.0);
    
    // In S3M, "pattern position C" usually refers to the 13th entry in the order list.
    println!("Pattern order: {:?}", song.song_data.pattern_order);
    
    let order_idx = 12; // Pattern position C
    let pattern_idx = song.song_data.pattern_order[order_idx] as usize;
    
    // Jump to the pattern
    song.song_position = order_idx;
    song.row = 0;
    song.tick = 0;
    
    println!("Testing 2nd_reality.s3m Position {} (Pattern Index {})", order_idx, pattern_idx);
    
    let mut channel_3_periods = vec![];
    
    // Run for 64 rows * ticks_per_row
    let ticks_per_row = song.speed; 
    for r in 0..64 {
        for t in 0..ticks_per_row {
            song.process_tick();
            let period = song.channels[3].note.period;
            if period != 0 {
                // println!("Row {} Tick {}: Period {}", r, t, period);
                if period < 113 || period > 27392 {
                    println!("!!! Row {} Tick {}: Period {} OUT OF BOUNDS!", r, t, period);
                }
            }
            channel_3_periods.push(period);
        }
    }
    
    // Check for "weird frequency artifacts" (e.g. sudden jumps or extreme values)
    let mut last_p = channel_3_periods[0];
    for (i, &p) in channel_3_periods.iter().enumerate().skip(1) {
        if p != 0 && last_p != 0 {
            let diff = (p as i32 - last_p as i32).abs();
            if diff > 1000 {
                println!("Significant period jump at tick {}: {} -> {} (diff {})", i, last_p, p, diff);
            }
        }
        last_p = p;
    }
}
