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

/// Per-tick dump of 2ND_PM.S3M between orders 0x22 and 0x24, focused on
/// the SD2 note-delay rows the user reports chirping. Prints every tick
/// (not just first-tick) so we can compare against master at the
/// sample-per-tick level.
///
/// To diff against master, run this test on both branches and compare
/// the outputs around `[Order 035 | Row 050]` (= 0x23 / 0x32).
#[test]
#[ignore = "Manual local dump comparison helper"]
fn dump_2nd_pm_s3m_chirp_window() {
    let path = "/Users/gil-ad/work/modplayer/scratch/2ND_PM.S3M";
    let (song_handle, _consumer) = match SongState::new(path) {
        Ok(s) => s,
        Err(e) => { eprintln!("Failed to load: {:?}", e); return; }
    };

    let mut song = song_handle.get_song().lock().unwrap();

    loop {
        if song.song_position > 0x24 { break; }

        // Per-tick dump within the chirp window; sparser elsewhere so
        // the file stays readable.
        let in_window = song.song_position == 0x23 && (0x28..=0x36).contains(&song.row);
        let at_first_tick = song.tick == 0;

        song.process_tick();

        if in_window || at_first_tick {
            println!("{}", dump_tick(&song).to_string());
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
