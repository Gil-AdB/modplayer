use std::time::Instant;
use xmplayer::song::{Song, PlayData, InterleavedBufferAdaptar};
use xmplayer::module_reader::read_module;

#[test]
fn test_seek_performance() {
    let song_data = read_module("test_data/test.xm").unwrap();
    
    // Create Dummy Triple Buffer
    let tb = shared_sync_primitives::TripleBuffer::<PlayData>::new();
    let tb_writer = tb.split().1;

    let mut song = Song::new(&song_data, tb_writer, 48000.0);
    
    let start = Instant::now();
    let mut ticks_logic = 0;
    let mut rx = std::sync::mpsc::channel().1;
    let mut dummy_buf = vec![0.0; 512];
    let mut adapter = InterleavedBufferAdaptar { buf: &mut dummy_buf };

    // Process logic only for 10000 ticks
    while ticks_logic < 10000 {
        song.get_next_tick( &mut adapter, &mut rx);
        ticks_logic += 1;
    }
    let duration_logic = start.elapsed();
    println!("Logic Only ({} ticks): {:?}", ticks_logic, duration_logic);
}
