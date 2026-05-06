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
fn test_dump_strshine() {
    generate_state_dump("/Users/gil-ad/Downloads/mods/strshine.s3m", "test_data/strshine_refactor.txt", 1000);
}

#[test]
fn test_dump_2nd_reality() {
    generate_state_dump("/Users/gil-ad/Downloads/mods/2nd_reality.s3m", "test_data/2nd_reality_refactor.txt", 1000);
}

#[test]
fn test_dump_2nd_pm_xm() {
    generate_state_dump("/Users/gil-ad/work/modplayer/2ND_PM.xm", "/Users/gil-ad/work/modplayer/2ND_PM_xm_refactor.txt", 2000);
}

#[test]
fn test_dump_2nd_pm_s3m() {
    generate_state_dump("/Users/gil-ad/work/modplayer/2ND_PM.S3M", "/Users/gil-ad/work/modplayer/2ND_PM_s3m_refactor.txt", 2000);
}
