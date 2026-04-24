use xmplayer::song_state::SongState;

#[test]
fn dump_pattern_4() {
    let (song_handle, _) = SongState::new("/Users/gil-ad/Downloads/mods/strshine.s3m").unwrap();
    let song = song_handle.get_song().lock().unwrap();
    let pattern_idx = song.song_data.pattern_order[3] as usize;
    let pattern = &song.song_data.patterns[pattern_idx];
    println!("Pattern 4 (Order 3) has {} rows", pattern.rows.len());
    let row_44 = &pattern.rows[44];
    for (i, chan) in row_44.channels.iter().enumerate() {
        if chan.effect != 0 || chan.volume != 0 {
            println!("  Ch {}: Effect {:x} Param {:x} Vol {:x}", i, chan.effect, chan.effect_param, chan.volume);
        }
    }
}
