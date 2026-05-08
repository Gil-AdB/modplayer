use xmplayer::song_state::SongState;
use xmplayer::song::test_dump::dump_tick;
use std::fs::File;
use std::io::Write;

fn generate_state_dump(path: &str, output_filename: &str, max_ticks: usize) {
    let (song_handle, _consumer) = match SongState::new(path) {
        Ok(s) => s,
        Err(e) => panic!("Failed to load test file: {}", e),
    };

    let mut output_file = File::create(output_filename).expect("Failed to create dump file");

    let _dummy_buffer = vec![0.0f32; 1000000];
    
    let mut ticks = 0;
    
    while ticks < max_ticks {
        let mut song = song_handle.get_song().lock().unwrap();
        
        song.process_tick();
        let dump = dump_tick(&song);
        writeln!(output_file, "{}", dump.to_string()).unwrap();

        if !song.next_tick() {
            break;
        }
        
        ticks += 1;
    }
}

#[test]
fn test_dump_milky() {
    generate_state_dump("test_data/milky.xm", "test_data/milky_dump.txt", 100);
}

#[test]
fn test_dump_spacedeb() {
    generate_state_dump("test_data/spacedeb.mod", "test_data/spacedeb_dump.txt", 100);
}

#[test]
fn test_dump_amiga_limits() {
    generate_state_dump("test_data/AmigaLimitsFinetune.mod", "test_data/AmigaLimitsFinetune_dump.txt", 100);
}
#[test]
#[ignore = "Manual dump regen; reads /Users/gil-ad/Downloads/mods/strshine.s3m"]
fn test_dump_strshine() {
    generate_state_dump("/Users/gil-ad/Downloads/mods/strshine.s3m", "test_data/strshine_refactor.txt", 1000);
}

#[test]
#[ignore = "Manual dump regen; reads /Users/gil-ad/Downloads/mods/2nd_reality.s3m"]
fn test_dump_2nd_reality() {
    generate_state_dump("/Users/gil-ad/Downloads/mods/2nd_reality.s3m", "test_data/2nd_reality_refactor.txt", 1000);
}

#[test]
#[ignore = "Manual dump regen; reads /Users/gil-ad/work/modplayer/2ND_PM.xm"]
fn test_dump_2nd_pm_xm() {
    generate_state_dump("/Users/gil-ad/work/modplayer/2ND_PM.xm", "/Users/gil-ad/work/modplayer/2ND_PM_xm_refactor.txt", 2000);
}

#[test]
#[ignore = "Manual dump regen; reads /Users/gil-ad/work/modplayer/2ND_PM.S3M"]
fn test_dump_2nd_pm_s3m() {
    generate_state_dump("/Users/gil-ad/work/modplayer/2ND_PM.S3M", "/Users/gil-ad/work/modplayer/2ND_PM_s3m_refactor.txt", 2000);
}

/// Per-tick dump of 2ND_PM.S3M up to order 0x24, all-tick within the chirp
/// window (rows 40-54 of order 0x23) and just first-tick elsewhere.
/// Outputs to /tmp/2ND_PM_chirp_refactor.txt for diffing against master.
#[test]
#[ignore = "Manual chirp-window dump for master/refactor diff"]
fn test_dump_2nd_pm_s3m_chirp_window() {
    let path = "/Users/gil-ad/work/modplayer/scratch/2ND_PM.S3M";
    let output_filename = "/tmp/2ND_PM_chirp_refactor.txt";

    let (song_handle, _consumer) = match SongState::new(path) {
        Ok(s) => s,
        Err(e) => { eprintln!("Failed to load: {}", e); return; }
    };

    let mut output_file = File::create(output_filename).expect("Failed to create dump file");
    let mut song = song_handle.get_song().lock().unwrap();

    loop {
        if song.song_position > 0x24 { break; }
        let in_window = song.song_position == 0x23 && (40..=54).contains(&song.row);
        let at_first_tick = song.tick == 0;

        song.process_tick();

        if in_window || at_first_tick {
            writeln!(output_file, "{}", dump_tick(&song).to_string()).unwrap();
        }

        if !song.next_tick() { break; }
    }
    eprintln!("dump written to {}", output_filename);
}
