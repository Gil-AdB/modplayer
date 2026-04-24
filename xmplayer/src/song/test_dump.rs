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
    pub panning: u8,
    pub final_panning: u8,
    pub sustained: bool,
    pub volume_envelope_pos: u16,
    pub panning_envelope_pos: u16,
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
        
        let mut sorted_voices = self.voices.clone();
        sorted_voices.sort_by_key(|v| v.channel_idx);

        for v in &sorted_voices {
            if !v.is_on {
                continue;
            }
            out.push_str(&format!(
                "  Ch {:02}: ON | Inst {:02} | Samp {:02} | Pos {:>9.3} | dU {:>7.3} | Vol {:>7.3} | Pan {:03} ({:03}) | Sus {} | Env V:{:03} P:{:03}\n",
                v.channel_idx,
                v.instrument,
                v.sample,
                v.sample_pos,
                v.du,
                v.output_volume,
                v.panning,
                v.final_panning,
                if v.sustained { "Y" } else { "N" },
                v.volume_envelope_pos,
                v.panning_envelope_pos
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
            panning: voice.panning.panning,
            final_panning: voice.panning.final_panning,
            sustained: voice.sustained,
            volume_envelope_pos: voice.volume_envelope_state.frame,
            panning_envelope_pos: voice.panning_envelope_state.frame,
        });
    }

    TickDump {
        song_position: song.song_position,
        row: song.row,
        tick: song.tick,
        voices,
    }
}
