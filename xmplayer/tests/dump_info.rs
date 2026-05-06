use xmplayer::song_state::SongState;

#[test]
#[ignore = "Manual local debug test with machine-specific file path"]
fn dump_info() {
    let (song_handle, _) = SongState::new("/Users/gil-ad/Downloads/mods/strshine.s3m").unwrap();
    let song = song_handle.get_song().lock().unwrap();
    println!("Channel Count: {}", song.song_data.instruments[29].samples[0].length);
    println!("Song Length: {}", song.song_data.song_length);
}
