// Backend trait + per-tick context. Per-format implementations live in
// the backend/ submodule (one file per format).

use crate::song::{GlobalVolume, BPM, PatternChange};
use crate::module_reader::{SongData, SongType};
use crate::channel_state::{ChannelState, Voice};
use crate::instrument::Instrument;
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

/// Set the basic voice fields shared by every format's note-trigger path.
/// Caller is responsible for sample-volume retrig, panning, trigger_note,
/// envelope reset, and the channel.voice_idx assignment - those vary
/// per format.
pub(super) fn init_voice_basics(voice: &mut Voice, channel_idx: usize, instrument: usize, sample: usize) {
    voice.on = true;
    voice.channel_idx = channel_idx;
    voice.instrument = instrument;
    voice.sample = sample;
    voice.sustained = true;
    voice.sample_position = 4.0;
    voice.loop_started = false;
}

/// Apply the previous-voice-on-channel handling that runs before alloc_voice
/// in XM, MOD, and S3M backends. XM and MOD always cut; S3M dispatches via
/// the instrument's NNA. Cmwt with IT's DCT/DCA logic isn't covered here -
/// that block is more involved and stays inline in ItBackend.
pub(super) fn cut_or_nna_existing_voice(
    voices: &mut [Voice],
    instruments: &Vec<Instrument>,
    song_type: SongType,
    channel_idx: usize,
    prev_voice_idx: usize,
) {
    let v = &voices[prev_voice_idx];
    if !(v.on && v.channel_idx == channel_idx) { return; }
    match song_type {
        SongType::XM | SongType::MOD => {
            voices[prev_voice_idx].on = false;
        }
        _ => {
            let nna = instruments[voices[prev_voice_idx].instrument].nna;
            match nna {
                0 => { voices[prev_voice_idx].on = false; }       // Cut
                1 => { /* Continue */ }
                2 => { voices[prev_voice_idx].key_off(instruments, false); } // Note Off
                3 => { voices[prev_voice_idx].sustained = false; } // Fade
                _ => { voices[prev_voice_idx].on = false; }
            }
        }
    }
}

/// End-of-tick mute pass: zero out a voice's output and mark it inactive
/// when it has finished fading or has dropped below the audibility floor.
/// Repeated identically in all four backends after their volume formula.
pub(super) fn mute_silent_voices(voices: &mut [Voice], channels: &[ChannelState]) {
    const SILENCE_FLOOR: f32 = 0.00001;
    for (v_idx, voice) in voices.iter_mut().enumerate() {
        if !voice.on { continue; }
        let is_host_voice = channels[voice.channel_idx].voice_idx == Some(v_idx);
        if !voice.sustained
            && (voice.volume.fadeout_vol == 0 || voice.volume.output_volume < SILENCE_FLOOR)
        {
            voice.on = false;
        } else if !is_host_voice && voice.volume.output_volume < SILENCE_FLOOR {
            voice.on = false;
        }
    }
}

/// Kind of extended-subcommand effect (XM `Exy`, S3M `Sxy`, IT `Sxy`).
///
/// Each format has its own 16-entry table that maps the high nibble (`x`) of
/// the param to an `ExtendedCmdKind`. The `y` nibble is the parameter and is
/// passed to `apply_extended` along with the kind. `None` is "no-op / not
/// implemented for this format / handled elsewhere (e.g. note delay, which
/// runs in the note-trigger path)".
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum ExtendedCmdKind {
    None,
    FinePortaUp,        // XM E1, MOD E1
    FinePortaDown,      // XM E2, MOD E2
    Glissando,          // XM E3
    VibratoWaveform,    // XM E4
    SetFinetune,        // XM E5
    PatternLoop,        // XM E6, MOD E6, S3M SB
    TremoloWaveform,    // XM E7
    NoteRetrig,         // XM E9, MOD E9 (extended retrig with no volume change)
    FineVolSlideUp,     // XM EA, MOD EA
    FineVolSlideDown,   // XM EB, MOD EB
    NoteCutAtTick,      // XM EC, MOD EC, S3M SC, IT SC
    PatternDelay,       // XM EE, MOD EE, S3M SE, IT SE
    SetExtraPanning,    // S3M S8, IT S8 (param << 4)
    SetItPanning,       // IT only - 0..255 panning at first tick (currently goes through 0x18)
}

/// XM's Exy table.
pub(super) const XM_E_TABLE: [ExtendedCmdKind; 16] = {
    use ExtendedCmdKind::*;
    [
        None,             // E0  set filter (Amiga LED) - not implemented
        FinePortaUp,      // E1
        FinePortaDown,    // E2
        Glissando,        // E3
        VibratoWaveform,  // E4
        SetFinetune,      // E5
        PatternLoop,      // E6
        TremoloWaveform,  // E7
        None,             // E8
        NoteRetrig,       // E9
        FineVolSlideUp,   // EA
        FineVolSlideDown, // EB
        NoteCutAtTick,    // EC
        None,             // ED  note delay - handled at note-trigger time
        PatternDelay,     // EE
        None,             // EF
    ]
};

/// MOD's Exy table. Identical to XM except E3/E4/E5/E7 (waveform / glissando /
/// finetune) are intentionally unimplemented to preserve historical behavior.
pub(super) const MOD_E_TABLE: [ExtendedCmdKind; 16] = {
    use ExtendedCmdKind::*;
    [
        None,             // E0
        FinePortaUp,      // E1
        FinePortaDown,    // E2
        None,             // E3
        None,             // E4
        None,             // E5
        PatternLoop,      // E6  (was a TODO stub — now wired via the table)
        None,             // E7
        None,             // E8
        NoteRetrig,       // E9
        FineVolSlideUp,   // EA
        FineVolSlideDown, // EB
        NoteCutAtTick,    // EC
        None,             // ED  note delay
        PatternDelay,     // EE
        None,             // EF
    ]
};

/// S3M's Sxy table.
pub(super) const S3M_S_TABLE: [ExtendedCmdKind; 16] = {
    use ExtendedCmdKind::*;
    [
        None,             // S0
        None,             // S1  set glissando (not implemented)
        None,             // S2  set finetune (not implemented)
        None,             // S3  vibrato waveform (not implemented)
        None,             // S4  tremolo waveform (not implemented)
        None,             // S5
        None,             // S6  delay note retrigger
        None,             // S7  NNA controls
        SetExtraPanning,  // S8  panning (param * 17)
        None,             // S9  surround
        None,             // SA  high sample offset
        PatternLoop,      // SB
        NoteCutAtTick,    // SC
        None,             // SD  note delay
        PatternDelay,     // SE
        None,             // SF
    ]
};

/// IT's Sxy table — only the subcommands the engine currently honors.
pub(super) const IT_S_TABLE: [ExtendedCmdKind; 16] = {
    use ExtendedCmdKind::*;
    [
        None,             // S0
        None,             // S1
        None,             // S2
        None,             // S3
        None,             // S4
        None,             // S5
        None,             // S6
        None,             // S7
        SetItPanning,     // S8 (param << 4 - 16-step coarse panning)
        None,             // S9
        None,             // SA
        None,             // SB
        NoteCutAtTick,    // SC
        None,             // SD  note delay
        PatternDelay,     // SE
        None,             // SF
    ]
};

/// Apply an extended-subcommand effect. Inputs come from the channel-loop
/// scope: the caller already holds `&mut ChannelState` and an optional
/// `&mut Voice`, and supplies the read-only context bits (tick, row, song
/// type, etc.) by value or shared reference.
pub(super) fn apply_extended(
    kind: ExtendedCmdKind,
    channel: &mut ChannelState,
    mut voice: Option<&mut Voice>,
    pattern_change: &mut PatternChange,
    instruments: &Vec<Instrument>,
    tick: u32,
    row: usize,
    first_tick: bool,
    first_row_tick: bool,
    song_type: SongType,
    rate: f32,
    frequency_tables: &AudioTables,
    y: u8,
) {
    match kind {
        ExtendedCmdKind::None => {}
        ExtendedCmdKind::FinePortaUp => {
            channel.fine_porta_up(song_type, first_tick, y);
        }
        ExtendedCmdKind::FinePortaDown => {
            channel.fine_porta_down(song_type, first_tick, y);
        }
        ExtendedCmdKind::Glissando => {
            if first_tick { channel.glissando = y != 0; }
        }
        ExtendedCmdKind::VibratoWaveform => {
            if first_tick {
                channel.vibrato_waveform = y & 3;
                channel.vibrato_retrig = (y & 4) == 0;
            }
        }
        ExtendedCmdKind::SetFinetune => {
            if first_tick {
                channel.note.finetune = (((y as i16) << 4) - 128) as i8;
            }
        }
        ExtendedCmdKind::PatternLoop => {
            // Per-channel pattern loop. y == 0 marks the loop start row;
            // y > 0 fires the back-jump up to y times.
            if first_tick {
                if y == 0 {
                    channel.loop_row = row as u8;
                } else if channel.loop_count == 0 {
                    channel.loop_count = y;
                    pattern_change.set_loop(channel.loop_row);
                } else {
                    channel.loop_count -= 1;
                    if channel.loop_count > 0 {
                        pattern_change.set_loop(channel.loop_row);
                    }
                }
            }
        }
        ExtendedCmdKind::TremoloWaveform => {
            if first_tick {
                channel.tremolo_waveform = y & 3;
                channel.tremolo_retrig = (y & 4) == 0;
            }
        }
        ExtendedCmdKind::NoteRetrig => {
            channel.it_retrig(voice.as_deref_mut(), instruments, tick, y);
        }
        ExtendedCmdKind::FineVolSlideUp => {
            channel.fine_volume_slide(voice.as_deref_mut(), first_tick, y as i8);
        }
        ExtendedCmdKind::FineVolSlideDown => {
            channel.fine_volume_slide(voice.as_deref_mut(), first_tick, -(y as i8));
        }
        ExtendedCmdKind::NoteCutAtTick => {
            if tick == y as u32 {
                channel.on = false;
                if let Some(v) = voice.as_deref_mut() { v.on = false; }
            }
        }
        ExtendedCmdKind::PatternDelay => {
            // XM uses first_row_tick gating; S3M/IT use first_tick.
            // Both end up at "set once at the row's first tick".
            let gate = match song_type { SongType::XM => first_row_tick, _ => first_tick };
            if gate && !pattern_change.delay_processed {
                pattern_change.pattern_delay = y;
                pattern_change.delay_processed = true;
            }
        }
        ExtendedCmdKind::SetExtraPanning => {
            // S3M S8x: y nibble * 17 maps 0..15 to 0..255.
            if first_tick {
                if let Some(v) = voice.as_deref_mut() {
                    v.panning.set_panning(((y as i32) * 17).min(255));
                }
            }
        }
        ExtendedCmdKind::SetItPanning => {
            // IT S8x: y << 4 — 16-step coarse panning.
            if first_tick {
                if let Some(v) = voice.as_deref_mut() {
                    v.panning.set_panning((y << 4) as i32);
                }
            }
        }
    }

    let _ = (frequency_tables, rate); // currently only fine_porta_* uses these via channel methods
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
