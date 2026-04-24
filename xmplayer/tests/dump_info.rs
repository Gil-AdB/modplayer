use xmplayer::song_state::SongState;

#[test]
fn dump_info() {
    let (song_handle, _) = SongState::new("/Users/gil-ad/Downloads/mods/strshine.s3m").unwrap();
    let song = song_handle.get_song().lock().unwrap();
    println!("Channel Count: {}", song.song_data.channel_count);
    println!("Song Length: {}", song.song_data.song_length);
}
