use xmplayer::song_state::SongState;

#[test]
fn dump_order() {
    let (song_handle, _) = SongState::new("/Users/gil-ad/Downloads/mods/strshine.s3m").unwrap();
    let song = song_handle.get_song().lock().unwrap();
    println!("Pattern Order: {:?}", song.song_data.pattern_order);
    println!("Song Length: {}", song.song_data.song_length);
}
