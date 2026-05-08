/// Temporary test module for comparing dump output of 2ND_PM.xm
/// Run with: cargo test -p xmplayer dump_2nd_pm -- --nocapture

use crate::song_state::SongState;
use crate::song::test_dump::dump_tick;

#[test]
#[ignore = "Manual local dump comparison helper"]
fn dump_2nd_pm() {
    let path = "/Users/gil-ad/Downloads/mods/2ND_PM.xm";
    let (song_handle, _consumer) = match SongState::new(path) {
        Ok(s) => s,
        Err(e) => { eprintln!("Failed to load: {:?}", e); return; }
    };

    let mut song = song_handle.get_song().lock().unwrap();

    loop {
        if song.song_position > 9 { break; }

        if song.tick == 0 {
            song.process_tick();
            println!("{}", dump_tick(&song).to_string());
        } else {
            song.process_tick();
        }

        if !song.next_tick() {
            break;
        }
    }
}


#[test]
#[ignore = "Manual local dump comparison helper"]
fn dump_2nd_reality_s3m() {
    let path = "/Users/gil-ad/Downloads/mods/2nd_reality.s3m";
    let (song_handle, _consumer) = match SongState::new(path) {
        Ok(s) => s,
        Err(e) => { eprintln!("Failed to load: {:?}", e); return; }
    };

    let mut song = song_handle.get_song().lock().unwrap();

    loop {
        if song.song_position > 2 { break; }

        if song.tick == 0 {
            song.process_tick();
            println!("{}", dump_tick(&song).to_string());
        } else {
            song.process_tick();
        }

        if !song.next_tick() {
            break;
        }
    }
}
