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
    /// Raw voice slot volume (0..64) before envelope / fadeout / mixing.
    /// `output_volume` already folds in the slot, the envelope, the
    /// channel volume and the global volume — when chasing a "vol
    /// reaches 0 too early" bug we want to see the slot in isolation.
    pub voice_volume: u8,
    pub panning: u8,
    pub final_panning: u8,
    pub sustained: bool,
    pub volume_envelope_pos: u16,
    pub panning_envelope_pos: u16,
    /// Auto-vibrato accumulator (instrument-level vibrato, XM
    /// vib_type/sweep/depth/rate). Maps onto ft2play's
    /// `eVibPos / eVibAmp / eVibSweep` for cross-engine diffs against
    /// `[FT2]` trace lines. Stays 0 when the instrument has no
    /// auto-vibrato (depth == 0).
    pub auto_vibrato_pos: u16,
    pub auto_vibrato_amp: u16,
    pub auto_vibrato_sweep: u16,
    pub effect: u8,
    pub effect_param: u8,
    pub note_str: String,
    pub channel_volume: u8,
    pub relative_note: i8,
    pub finetune: i8,
    /// Mixer telemetry — last sample-frame this voice was rendered.
    /// 0 = trigger has fired but mixer hasn't run yet (synthetic test).
    pub last_render_tick: u64,
    /// Reason for the last cut, if any. `None` while the voice is on
    /// or before any cut was recorded.
    pub cut_reason: Option<String>,
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
    pub frequency_type: String,
}

impl TickDump {
    pub fn to_string(&self) -> String {
        self.to_string_filtered(&[])
    }

    /// Filtered variant: when `channels` is non-empty, only voices whose
    /// `channel_idx` is in the slice are printed. Both ON voices and cut
    /// voices respect the filter — important for grep-by-time
    /// investigations where the user wants ALL state for one channel,
    /// including the cut_reason that explains why a voice fell silent.
    /// Single source of truth so state_dump's --channels flag and the
    /// unfiltered `to_string()` always print the same fields (the prior
    /// channel-filter format was a separate string that omitted
    /// `last_render_tick` and dropped cut voices entirely — a state_dump
    /// blind spot that misled the t=119-121s investigation).
    pub fn to_string_filtered(&self, channels: &[usize]) -> String {
        let want = |idx: usize| channels.is_empty() || channels.contains(&idx);
        let mut out = String::new();
        out.push_str(&format!("[Order {:03} | Row {:03} | Tick {:03}] (Voices: {} / Channels: {}) (Speed: {} / BPM: {} / GVol: {} / Freq: {})\n",
            self.song_position, self.row, self.tick, self.active_voices, self.active_channels, self.speed, self.bpm, self.global_volume, self.frequency_type));

        let mut sorted_voices = self.voices.clone();
        sorted_voices.sort_by_key(|v| v.channel_idx);

        for v in &sorted_voices {
            if !want(v.channel_idx) { continue; }
            if !v.is_on { continue; }
            // last_render_tick is in sample-frames; format as `R:N` where
            // N=0 means the trigger ran but the mixer hasn't (synthetic
            // tests / paused state). A non-zero value lets state_dump
            // distinguish "still being mixed" from "frozen at trigger" —
            // the prior 119-121s investigation made the wrong call there.
            out.push_str(&format!(
                "  Ch {:02}: ON | Inst {:02} | Samp {:02} | {} | Pos {:>9.3} | dU {:>7.3} | Vol {:>7.3} (Vraw:{:02} CV:{:02}) | Pan {:03} ({:03}) | Sus {} | Env V:{:03} P:{:03} | AVib pos:{:03} amp:{:>5} swp:{:>5} | Eff {:02x} {:02x} | Rel:{:4} Fine:{:4} | R:{:>10}\n",
                v.channel_idx,
                v.instrument,
                v.sample,
                v.note_str,
                v.sample_pos,
                v.du,
                v.output_volume,
                v.voice_volume,
                v.channel_volume,
                v.panning,
                v.final_panning,
                if v.sustained { "Y" } else { "N" },
                v.volume_envelope_pos,
                v.panning_envelope_pos,
                v.auto_vibrato_pos,
                v.auto_vibrato_amp,
                v.auto_vibrato_sweep,
                v.effect,
                v.effect_param,
                v.relative_note,
                v.finetune,
                v.last_render_tick,
            ));
        }
        // Recently-cut voices: dump the reason so a 23×-RMS-spike-style
        // investigation can see "ch5 cut at frame N for SampleEnd" rather
        // than guessing from frozen sample_position values. We only print
        // voices that have a cut_reason set (None means never cut, i.e.
        // never triggered).
        for v in &sorted_voices {
            if !want(v.channel_idx) { continue; }
            if v.is_on { continue; }
            if let Some(reason) = &v.cut_reason {
                out.push_str(&format!(
                    "  Ch {:02}: cut={} | Inst {:02} | Samp {:02} | last_render={}\n",
                    v.channel_idx, reason, v.instrument, v.sample, v.last_render_tick,
                ));
            }
        }
        out
    }
}

pub fn dump_tick(song: &Song) -> TickDump {
    let mut voices = Vec::new();
    for voice in song.voices.iter() {
        let pattern = &song.song_data.patterns[song.song_data.pattern_order[song.song_position] as usize].rows[song.row].channels[voice.channel_idx];
        voices.push(VoiceDump {
            is_on: voice.on,
            channel_idx: voice.channel_idx,
            instrument: voice.instrument,
            sample: voice.sample,
            sample_pos: voice.sample_position,
            du: voice.du,
            output_volume: voice.volume.output_volume,
            voice_volume: voice.volume.volume,
            panning: voice.panning.panning,
            final_panning: voice.panning.final_panning,
            sustained: voice.sustained,
            volume_envelope_pos: voice.volume_envelope_state.frame,
            panning_envelope_pos: voice.panning_envelope_state.frame,
            auto_vibrato_pos: voice.vibrato_envelope_state.vibrato_pos,
            auto_vibrato_amp: voice.vibrato_envelope_state.vibrato_amp,
            auto_vibrato_sweep: voice.vibrato_envelope_state.vibrato_sweep,
            effect: pattern.effect,
            effect_param: pattern.effect_param,
            note_str: match voice.last_played_note {
                0 => "   ".to_string(),
                97 | 254 => "OFF".to_string(),
                121 | 255 => "CUT".to_string(),
                122 | 253 => "FAD".to_string(),
                n if n <= 120 => format!("{}{}", ["C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-"][((n - 1) % 12) as usize], (((n - 1) / 12) + b'0') as char),
                _ => "???".to_string(),
            },
            channel_volume: song.channels[voice.channel_idx].channel_volume,
            relative_note: song.song_data.instruments[voice.instrument].samples[voice.sample].relative_note,
            finetune: song.song_data.instruments[voice.instrument].samples[voice.sample].finetune,
            last_render_tick: voice.last_render_tick,
            cut_reason: voice.cut_reason.map(|r| format!("{:?}", r)),
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
        frequency_type: format!("{:?}", song.song_data.frequency_type),
    }
}
