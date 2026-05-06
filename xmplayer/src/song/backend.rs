// Backend trait + per-tick context. Per-format implementations live in
// the backend/ submodule (one file per format).

use crate::song::{GlobalVolume, BPM, PatternChange};
use crate::module_reader::SongData;
use crate::channel_state::{ChannelState, Voice};
use crate::tables::AudioTables;

mod it;
mod xm;
mod s3m;
mod mod_;

pub use it::ItBackend;
pub use xm::XmBackend;
pub use s3m::S3MBackend;
pub use mod_::ModBackend;

pub struct SongPlaybackResources<'a> {
    pub song_position:              &'a mut usize,
    pub row:                        &'a mut usize,
    pub tick:                       &'a mut u32,
    pub speed:                      &'a mut u32,
    pub global_volume:              &'a mut GlobalVolume,
    pub song_data:                  &'a SongData,
    pub channels:                   &'a mut [ChannelState],
    pub voices:                     &'a mut [Voice],
    pub pattern_change:             &'a mut PatternChange,
    pub bpm:                        &'a mut BPM,
    pub frequency_tables:           &'a AudioTables,
    pub rate:                       f32,
    pub first_row_tick:             bool,
    pub old_effects:                bool,
    pub compatible_g:               bool,
}

pub trait ModuleBackend: Send {
    fn process_tick(&mut self, resources: &mut SongPlaybackResources);
}

/// Pick a voice slot for a new note: prefer the first idle voice, otherwise
/// steal the quietest one. Used by every backend's note-trigger block.
pub(super) fn alloc_voice(voices: &mut [Voice]) -> usize {
    for (vi, v) in voices.iter().enumerate() {
        if !v.on { return vi; }
    }
    let mut idx = 0;
    let mut min_vol = f32::INFINITY;
    for (vi, v) in voices.iter().enumerate() {
        if v.volume.output_volume < min_vol {
            min_vol = v.volume.output_volume;
            idx = vi;
        }
    }
    idx
}

/// Compute real_note (mapped_note + sample.relative_note clamped) and push
/// it into the channel + voice frequency state. Common across IT/XM/S3M/MOD.
pub(super) fn set_channel_note(
    channel: &mut ChannelState,
    voice: &mut Voice,
    sample_relative_note: i8,
    sample_finetune: i8,
    mapped_note: u8,
    rate: f32,
    frequency_tables: &AudioTables,
) {
    let real_note = (mapped_note as i16 + sample_relative_note as i16).clamp(1, 120) as u8;
    channel.note.set_note(real_note, sample_finetune, mapped_note, frequency_tables);
    channel.update_frequency_voice(voice, rate, false, frequency_tables);
}
