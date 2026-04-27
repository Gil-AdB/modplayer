use crate::song::Song;
use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize)]
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
    pub effect: u8,
    pub effect_param: u8,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TickDump {
    pub song_position: usize,
    pub row: usize,
    pub tick: u32,
    pub speed: u32,
    pub bpm: u32,
    pub voices: Vec<VoiceDump>,
    pub active_voices: usize,
    pub active_channels: usize,
    pub global_volume: u32,
}

impl TickDump {
    pub fn to_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("[Order {:03} | Row {:03} | Tick {:03}] (Voices: {} / Channels: {}) (Speed: {} / BPM: {} / GVol: {})\n", 
            self.song_position, self.row, self.tick, self.active_voices, self.active_channels, self.speed, self.bpm, self.global_volume));
        
        let mut sorted_voices = self.voices.clone();
        sorted_voices.sort_by_key(|v| v.channel_idx);

        for v in &sorted_voices {
            if !v.is_on {
                continue;
            }
            out.push_str(&format!(
                "  Ch {:02}: ON | Inst {:02} | Samp {:02} | Pos {:>9.3} | dU {:>7.3} | Vol {:>7.3} | Pan {:03} ({:03}) | Sus {} | Env V:{:03} P:{:03} | Eff {:02x} {:02x}\n",
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
                v.panning_envelope_pos,
                v.effect,
                v.effect_param
            ));
        }
        out
    }
}

pub fn dump_tick(song: &Song) -> TickDump {
    let mut voices = Vec::new();
    for (v_idx, voice) in song.voices.iter().enumerate() {
        if voice.on {
        }
        let pattern = &song.song_data.patterns[song.song_data.pattern_order[song.song_position] as usize].rows[song.row].channels[voice.channel_idx];
        if voice.on {
        }
        if voice.on {
        }
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
            effect: pattern.effect,
            effect_param: pattern.effect_param,
        });
    }

    let active_voices = voices.iter().filter(|v| v.is_on).count();
    TickDump {
        song_position: song.song_position,
        row: song.row,
        tick: song.tick,
        speed: song.speed,
        bpm: song.bpm.bpm,
        active_voices,
        active_channels: song.channels.iter().filter(|c| c.voice_idx.is_some()).count(),
        voices,
        global_volume: song.global_volume.volume,
    }
}
