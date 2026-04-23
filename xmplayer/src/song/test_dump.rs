use crate::song::Song;

#[derive(Debug, Clone)]
pub struct VoiceDump {
    pub is_on: bool,
    pub channel_idx: usize,
    pub instrument: usize,
    pub sample: usize,
    pub sample_pos: f32,
    pub du: f32,
    pub output_volume: f32,
    pub final_panning: u8,
    pub filter_cutoff: u8,
}

#[derive(Debug, Clone)]
pub struct TickDump {
    pub song_position: usize,
    pub row: usize,
    pub tick: u32,
    pub voices: Vec<VoiceDump>,
}

impl TickDump {
    pub fn to_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("[Order {:03} | Row {:03} | Tick {:03}]\n", self.song_position, self.row, self.tick));
        for (i, v) in self.voices.iter().enumerate() {
            if !v.is_on {
                continue; // Skip inactive voices to keep the diff clean
            }
            out.push_str(&format!(
                "  V {:02} (Ch {:02}): ON | Inst {:02} | Samp {:02} | Pos {:>9.3} | dU {:>7.3} | Vol {:>7.3} | Pan {:03} | Cutoff {:03}\n",
                i,
                v.channel_idx,
                v.instrument,
                v.sample,
                v.sample_pos,
                v.du,
                v.output_volume,
                v.final_panning,
                v.filter_cutoff
            ));
        }
        out
    }
}

pub fn dump_tick(song: &Song) -> TickDump {
    let mut voices = Vec::new();
    for voice in &song.voices {
        voices.push(VoiceDump {
            is_on: voice.on,
            channel_idx: voice.channel_idx,
            instrument: voice.instrument,
            sample: voice.sample,
            sample_pos: voice.sample_position,
            du: voice.du,
            output_volume: voice.volume.output_volume,
            final_panning: voice.panning.final_panning,
            filter_cutoff: voice.filter_cutoff,
        });
    }

    TickDump {
        song_position: song.song_position,
        row: song.row,
        tick: song.tick,
        voices,
    }
}
