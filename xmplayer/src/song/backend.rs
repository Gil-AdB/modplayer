// Backend trait + per-tick context + shared effect dispatch tables.
//
// Architecture
// ============
//
// `Song` calls a single `ModuleBackend::process_tick` per tick. Each format
// (XM / MOD / S3M / IT) has its own backend struct in the `backend/` sub-
// module that implements the trait. The backends share a common shape that
// lives mostly in this file as small helpers; each per-format file slots
// in its own note-action / volume-column / voice-mixdown logic between the
// shared scaffolding calls.
//
// Per-channel iteration shape:
//
//   for each channel:
//     init_channel_iter            transient flag reset + last_instrument
//                                  latch + note-delay gate
//     [per-format: note-action match]   Trigger / Off / Cut / Fade / None
//     apply_porta_retrig_if_needed instrument-on-porta-row vol+envelope reset
//     bind_voice_for_channel       Option<&mut Voice> with back-pointer guard
//     [per-format: volume column]
//     apply_flow_control_effect    short-circuits the rest of effect dispatch
//                                  if it fires; identical impl is reused by
//                                  the duration-calc fast path in playback.rs
//     dispatch_main_and_extended   main effect (32-entry table) + Exy/Sxy
//                                  follow-up (16-entry table) when Extended
//     update_frequency_voice       refresh voice frequency from period
//   for each voice: [per-format: volume formula]
//   mute_silent_voices             end-of-tick cleanup
//
// Effect dispatch is data-driven through three layers:
//
//   1. `apply_flow_control_effect` handles the simple flow effects
//      (Pattern Jump / Pattern Break / Set Speed / Set BPM). Both each
//      backend and the `is_calculating_duration` fast path in `playback.rs`
//      call it, so the two paths stay in sync.
//
//   2. `apply_effect` + per-format `*_EFFECT_TABLE: [EffectKind; 32]`
//      route the main effect column. Each backend looks up its raw effect
//      byte in its table to get an `EffectKind`, calls `apply_effect`. The
//      kind variants encode format-aware behaviour (e.g. `SetPanningXm`
//      vs `SetPanningIt` because the panning byte is interpreted
//      differently). `apply_effect` returns `true` for `Extended`, in
//      which case the caller follows up with...
//
//   3. `apply_extended` + per-format `*_E_TABLE: [ExtendedCmdKind; 16]`
//      handle the XM `Exy` / S3M `Sxy` / IT `Sxy` extended-subcommand
//      tables. Same shape as layer 2, but indexed by the param's high
//      nibble.
//
// `dispatch_main_and_extended` wraps layers 2 and 3 so a backend just
// passes its two tables. Per-tick context shared between layers 2 and 3
// lives in `EffectCtx<'a>`, constructed once per channel iteration.
//
// Per-channel "memory" for "param=0 means recall last param" semantics
// lives on `ChannelState::effect_memory: [u8; N]` indexed by the
// `EffectMemorySlot` enum (defined in `channel_state`). The
// `recall_or_set` / `recall_or_set_shared` helpers wrap the canonical
// "if param != 0 update; else recall" pattern.
//
// Per-channel scaffolding (used by every backend, shape above):
//
//   init_channel_iter             tremor reset + last_instrument latch +
//                                 note-delay gate; returns
//                                 note_delay_first_tick
//   apply_porta_retrig_if_needed  on porta-to-note + instrument byte: the
//                                 instrument re-reads sample default vol
//                                 and rewinds envelopes (no audio retrig).
//                                 Matches ST3/FT2/IT.
//   bind_voice_for_channel        Option<&mut Voice> with channel.voice_idx
//                                 guard against stolen slots
//   dispatch_main_and_extended    main + Exy/Sxy effect dispatch
//
// Voice / instrument helpers shared across formats:
//
//   alloc_voice                 prefer-idle / steal-quietest pick
//   cut_or_nna_existing_voice   apply prev-voice cut or NNA before alloc
//   init_voice_basics           set channel_idx / instrument / sample
//                               (Voice::trigger_note owns playback state)
//   set_channel_note            real_note clamp + Note::set_note +
//                               update_frequency_voice
//   mute_silent_voices          end-of-tick cleanup (voice.on = false on
//                               faded / non-host-silent voices)

use crate::song::{GlobalVolume, BPM, PatternChange};
use crate::module_reader::{SongData, SongType};
use crate::channel_state::{ChannelState, Voice};
use crate::instrument::Instrument;
use crate::pattern::Pattern;
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

/// Dispatch the simple flow-control effects: pattern jump, pattern break,
/// set speed, set BPM. Returns true if `pattern.effect` was a flow-control
/// effect that has been fully handled (caller should not dispatch it again).
///
/// Pattern Loop and Pattern Delay live inside the E/S extended-command
/// table and go through `apply_extended` instead — see the per-format
/// E/S tables and the dispatch in each backend.
///
/// Used by both per-format backends (so they can share one source of truth
/// for B/D/F/A/T) and by the duration-calc fast path in
/// `Song::process_tick` (`is_calculating_duration == true`).
pub(super) fn apply_flow_control_effect(
    pattern: &Pattern,
    song_type: SongType,
    first_tick: bool,
    pattern_change: &mut PatternChange,
    speed: &mut u32,
    bpm: &mut BPM,
    rate: f32,
) -> bool {
    // Effect codes overlap across formats (XM effect 2 is Porta Down, but
    // S3M/IT effect 2 is Pattern Jump). Match on (song_type, effect) so an
    // XM Porta Down can never be misclassified as a Pattern Jump.
    let xm_or_mod = matches!(song_type, SongType::XM | SongType::MOD);
    let s3m_or_it = matches!(song_type, SongType::S3M | SongType::IT);

    match pattern.effect {
        // Pattern Jump
        0xB if xm_or_mod => {
            pattern_change.set_jump(first_tick, pattern.effect_param);
            true
        }
        2 if s3m_or_it => {
            pattern_change.set_jump(first_tick, pattern.effect_param);
            true
        }
        // Pattern Break
        0xD if xm_or_mod => {
            pattern_change.set_break(song_type, first_tick, pattern.effect_param);
            true
        }
        3 if s3m_or_it => {
            pattern_change.set_break(song_type, first_tick, pattern.effect_param);
            true
        }
        // XM/MOD Fxx: <0x20 sets speed, >=0x20 sets BPM (a Protracker quirk
        // preserved by XM and MOD).
        0xF if xm_or_mod => {
            if first_tick && pattern.effect_param > 0 {
                if pattern.effect_param < 0x20 {
                    *speed = pattern.effect_param as u32;
                } else {
                    bpm.update(pattern.effect_param as u32, rate);
                }
            }
            true
        }
        // S3M Axx / IT Axx - SetSpeed
        1 if s3m_or_it => {
            if first_tick && pattern.effect_param > 0 {
                *speed = pattern.effect_param as u32;
            }
            true
        }
        // S3M Txx / IT Txx - SetBpm
        20 if s3m_or_it => {
            if first_tick && pattern.effect_param > 0 {
                bpm.update(pattern.effect_param as u32, rate);
            }
            true
        }
        _ => false,
    }
}

/// Pre-iteration boilerplate every backend repeats at the top of its
/// per-channel loop: reset transient flags, latch `last_instrument` from
/// the row's instrument byte, and compute the gating used by every
/// downstream block.
///
/// Returns `note_delay_first_tick`: the tick at which a note-delayed row
/// (XM `EDx` / S3M/IT `SDx`) should fire its trigger; falls back to
/// `first_tick` when the row carries no note delay.
pub(super) fn init_channel_iter(
    channel: &mut ChannelState,
    pattern: &Pattern,
    instruments: &[Instrument],
    song_type: SongType,
    tick: u32,
    first_tick: bool,
) -> bool {
    channel.tremor_silenced = false;
    if pattern.instrument != 0 {
        channel.last_instrument = if (pattern.instrument as usize) < instruments.len() {
            pattern.instrument as usize
        } else {
            0
        };
    }
    if pattern.is_note_delay(song_type) {
        tick == pattern.get_y() as u32
    } else {
        first_tick
    }
}

/// On a porta-to-note row that also carries an instrument number, the
/// instrument re-reads sample volume and rewinds envelopes (no audio
/// retrigger). Must run *after* the per-format note-action block (so that
/// a Trigger arm has set the porta target first) and *before* any volume
/// column work (so vol-col can override the retrig'd volume).
///
/// `is_porta_to_note` already covers all four formats (XM `0x03/0x05` +
/// vol-col `0xF0..=0xFE`, S3M `G/L`, IT `G/L` + vol-col `193..=202`,
/// MOD `0x03/0x05`).
pub(super) fn apply_porta_retrig_if_needed(
    voices: &mut [Voice],
    channel: &ChannelState,
    pattern: &Pattern,
    i: usize,
    first_tick: bool,
    instruments: &Vec<Instrument>,
    song_type: SongType,
) {
    if !(first_tick && pattern.is_porta_to_note(song_type) && pattern.instrument != 0) {
        return;
    }
    if let Some(v_idx) = channel.voice_idx {
        if voices[v_idx].channel_idx == i {
            voices[v_idx].porta_retrig_for_instrument(instruments);
        }
    }
}

/// Bind the channel's host voice if the back-pointer agrees (the slot
/// could have been stolen by an NNA / DCT cut, in which case we treat
/// the channel as voiceless for vol-col / effect dispatch). Used by all
/// four backends after the note-action block.
pub(super) fn bind_voice_for_channel<'a>(
    voices: &'a mut [Voice],
    channel: &ChannelState,
    i: usize,
) -> Option<&'a mut Voice> {
    let idx = channel.voice_idx?;
    if voices[idx].channel_idx != i {
        return None;
    }
    Some(&mut voices[idx])
}

/// Look up the row's main effect in the per-format `EFFECT_TABLE`,
/// dispatch it through `apply_effect`, then — if it was an `Extended`
/// command — look up the high nibble in the per-format `E/S` table and
/// dispatch through `apply_extended`. The flow-control effects
/// (Pattern Jump / Break / Set Speed / Set BPM) are *not* handled here:
/// callers must `apply_flow_control_effect` first and short-circuit on
/// its return value, since flow control short-circuits the rest of the
/// channel's effect work.
pub(super) fn dispatch_main_and_extended(
    pattern: &Pattern,
    channel: &mut ChannelState,
    mut voice_ref: Option<&mut Voice>,
    ctx: &mut EffectCtx<'_>,
    effect_table: &[EffectKind; 32],
    extended_table: &[ExtendedCmdKind; 16],
) {
    let kind = if pattern.effect < 32 {
        effect_table[pattern.effect as usize]
    } else {
        EffectKind::None
    };
    let is_extended = apply_effect(kind, channel, voice_ref.as_deref_mut(), ctx, pattern);
    if is_extended {
        let ext = extended_table[pattern.get_x() as usize];
        apply_extended(ext, channel, voice_ref.as_deref_mut(), ctx, pattern.get_y());
    }
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

/// Set the context fields a fresh voice needs before `Voice::trigger_note`
/// can run (trigger_note reads `instrument` and `sample` to look up
/// envelopes / global vol / filter / etc.). The playback-state fields
/// (`on`, `sustained`, `sample_position`, `loop_started`) are owned by
/// `trigger_note` so the retrig effect path - which calls trigger_note
/// without going through init_voice_basics - resets them too.
pub(super) fn init_voice_basics(voice: &mut Voice, channel_idx: usize, instrument: usize, sample: usize) {
    voice.channel_idx = channel_idx;
    voice.instrument = instrument;
    voice.sample = sample;
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

/// Bundle of borrow-split context that the per-channel effect helpers
/// share. Constructed once per channel iteration. Lets shared helpers
/// take `(channel, voice, ctx)` instead of stretching out into 10+
/// individual arguments, and keeps the lifetime story explicit.
#[allow(dead_code)] // frequency_tables / rate / old_effects / compatible_g are
                    // part of the canonical context bundle; not all helpers
                    // read them today but having them in the bundle saves
                    // signature widening when new helpers need them.
pub(super) struct EffectCtx<'a> {
    pub pattern_change: &'a mut PatternChange,
    pub global_volume:  &'a mut GlobalVolume,
    pub instruments:    &'a Vec<Instrument>,
    pub frequency_tables: &'a AudioTables,
    pub tick:           u32,
    pub row:            usize,
    pub first_tick:     bool,
    pub first_row_tick: bool,
    pub note_delay_first_tick: bool,
    pub song_type:      SongType,
    pub rate:           f32,
    pub old_effects:    bool,
    pub compatible_g:   bool,
}

/// Apply an extended-subcommand effect. Operates on the channel/voice the
/// caller has already borrowed mutably for this iteration; everything
/// else (pattern_change, instruments, tick, etc.) comes through the
/// shared `EffectCtx`.
pub(super) fn apply_extended(
    kind: ExtendedCmdKind,
    channel: &mut ChannelState,
    mut voice: Option<&mut Voice>,
    ctx: &mut EffectCtx<'_>,
    y: u8,
) {
    // Pull frequently-read fields out so the body keeps reading like
    // the inline match did before the bundle was introduced.
    let pattern_change   = &mut *ctx.pattern_change;
    let instruments      = ctx.instruments;
    let tick             = ctx.tick;
    let row              = ctx.row;
    let first_tick       = ctx.first_tick;
    let first_row_tick   = ctx.first_row_tick;
    let song_type        = ctx.song_type;
    // ctx.rate / ctx.frequency_tables aren't read directly here; the
    // channel.fine_porta_* paths reach period tables through SongType.
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

}

// =================================================================
// Main effect-column dispatch table.
// =================================================================
//
// Each format previously inlined ~15-20 match arms for the "main"
// effect column (everything that isn't flow control or the E/S
// extended subcommand). Most arms across formats called the same
// channel methods - the only thing that varied was the effect code.
//
// `EffectKind` collapses every format's per-effect intent into a
// single enum. Per-format `[EffectKind; 32]` tables map raw effect
// bytes into the enum, and `apply_effect` is the shared dispatcher.
// Each backend's main match collapses to a single table lookup +
// dispatch call.
//
// Per-format quirks (e.g. XM uses `volume_slide_main`, IT/S3M use
// `it_volume_slide`; XM panning is 0..255, IT panning is param*4)
// are folded into apply_effect via `match ctx.song_type` branches.

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum EffectKind {
    /// Slot is unused for this format / no-op.
    None,
    Arpeggio,
    PortaUp,
    PortaDown,
    PortaToNote,
    Vibrato,
    /// XM 5 / MOD 5 / S3M L / IT 0xC: porta-to-note + volume slide.
    PortaPlusVolSlide,
    /// XM 6 / MOD 6 / S3M K / IT 0xB: vibrato + volume slide.
    VibratoPlusVolSlide,
    Tremolo,
    /// XM/MOD 8: panning byte is 0..255.
    SetPanningXm,
    /// IT 0x18: panning byte is *4 (0..63 → 0..252).
    SetPanningIt,
    /// S3M 24 (X command): panning 0..255.
    SetPanningS3m,
    SampleOffset,
    /// XM 0xA / MOD 0xA: volume_slide_main (XM nibble decode).
    VolSlideXmStyle,
    /// S3M 4 / IT 0x04: it_volume_slide (IT nibble decode).
    VolSlideItStyle,
    /// XM 0xC / MOD 0xC: set voice volume (0..64).
    SetVolume,
    /// S3M 13: set channel volume (0..64).
    SetChannelVolume,
    /// S3M 14: channel-volume slide.
    ChannelVolSlide,
    /// XM 0x19 / S3M 16: panning slide (P).
    PanningSlide,
    /// S3M Q / IT 0x11: retrig (it_retrig).
    Retrig,
    /// XM 0x1B (Rxy): multi retrig with volume change.
    XmMultiRetrig,
    /// XM 0x10 / S3M V / IT 0x16: set global volume.
    SetGlobalVolume,
    /// XM 0x11 / S3M W / IT 0x17: global volume slide.
    GlobalVolSlide,
    /// XM 0x14: Kxx, key off at tick xx.
    KeyOffAtTick,
    /// XM 0x15: Lxx, set envelope position.
    SetEnvelopePos,
    /// S3M 9 (Ixy): tremor.
    Tremor,
    /// S3M 21 (Uxy): fine vibrato.
    FineVibrato,
    /// IT 0x1A (Zxx): resonant filter cutoff/resonance.
    Filter,
    /// XM 0xE / MOD 0xE / S3M 19 / IT 0x13: extended subcommand
    /// (handled by `apply_extended` via the per-format E/S tables, not
    /// `apply_effect` - this variant is here so the table can mark
    /// those slots and the backend can route to apply_extended).
    Extended,
}

const EK: EffectKind = EffectKind::None; // shorthand for table padding

/// XM main effect table. Index = pattern.effect (0..=0x1F).
pub(super) const XM_EFFECT_TABLE: [EffectKind; 32] = {
    use EffectKind::*;
    let mut t = [EK; 32];
    t[0x00] = Arpeggio;
    t[0x01] = PortaUp;
    t[0x02] = PortaDown;
    t[0x03] = PortaToNote;
    t[0x04] = Vibrato;
    t[0x05] = PortaPlusVolSlide;
    t[0x06] = VibratoPlusVolSlide;
    t[0x07] = Tremolo;
    t[0x08] = SetPanningXm;
    t[0x09] = SampleOffset;
    t[0x0A] = VolSlideXmStyle;
    // 0x0B Pattern Jump   - apply_flow_control_effect
    t[0x0C] = SetVolume;
    // 0x0D Pattern Break  - apply_flow_control_effect
    t[0x0E] = Extended;
    // 0x0F Speed/BPM      - apply_flow_control_effect
    t[0x10] = SetGlobalVolume;
    t[0x11] = GlobalVolSlide;
    t[0x14] = KeyOffAtTick;
    t[0x15] = SetEnvelopePos;
    t[0x19] = PanningSlide;
    t[0x1B] = XmMultiRetrig;
    t
};

/// MOD effect table. Index = pattern.effect (0..=0x0F).
pub(super) const MOD_EFFECT_TABLE: [EffectKind; 32] = {
    use EffectKind::*;
    let mut t = [EK; 32];
    t[0x00] = Arpeggio;
    t[0x01] = PortaUp;
    t[0x02] = PortaDown;
    t[0x03] = PortaToNote;
    t[0x04] = Vibrato;
    t[0x05] = PortaPlusVolSlide;
    t[0x06] = VibratoPlusVolSlide;
    t[0x07] = Tremolo;
    t[0x08] = SetPanningXm;
    t[0x09] = SampleOffset;
    t[0x0A] = VolSlideXmStyle;
    // 0x0B Pattern Jump  - apply_flow_control_effect
    t[0x0C] = SetVolume;
    // 0x0D Pattern Break - apply_flow_control_effect
    t[0x0E] = Extended;
    // 0x0F Speed/BPM     - apply_flow_control_effect
    t
};

/// S3M effect table.
pub(super) const S3M_EFFECT_TABLE: [EffectKind; 32] = {
    use EffectKind::*;
    let mut t = [EK; 32];
    // 1   A  SetSpeed         - apply_flow_control_effect
    // 2   B  PatternJump      - apply_flow_control_effect
    // 3   C  PatternBreak     - apply_flow_control_effect
    t[4]  = VolSlideItStyle;       // D
    t[5]  = PortaDown;             // E
    t[6]  = PortaUp;               // F
    t[7]  = PortaToNote;           // G
    t[8]  = Vibrato;               // H
    t[9]  = Tremor;                // I
    t[10] = Arpeggio;              // J
    t[11] = VibratoPlusVolSlide;   // K
    t[12] = PortaPlusVolSlide;     // L
    t[13] = SetChannelVolume;      // M
    t[14] = ChannelVolSlide;       // N
    t[15] = SampleOffset;          // O
    t[16] = PanningSlide;          // P
    t[17] = Retrig;                // Q
    t[18] = Tremolo;               // R
    t[19] = Extended;              // S (table-driven via S3M_S_TABLE)
    // 20  T  SetBpm            - apply_flow_control_effect
    t[21] = FineVibrato;           // U
    t[22] = SetGlobalVolume;       // V
    t[23] = GlobalVolSlide;        // W
    t[24] = SetPanningS3m;         // X
    t
};

/// IT effect table.
pub(super) const IT_EFFECT_TABLE: [EffectKind; 32] = {
    use EffectKind::*;
    let mut t = [EK; 32];
    // 0x01 A  SetSpeed         - apply_flow_control_effect
    // 0x02 B  PatternJump      - apply_flow_control_effect
    // 0x03 C  PatternBreak     - apply_flow_control_effect
    t[0x04] = VolSlideItStyle;
    t[0x05] = PortaDown;
    t[0x06] = PortaUp;
    t[0x07] = PortaToNote;
    t[0x08] = Vibrato;
    t[0x0A] = Arpeggio;
    t[0x0B] = VibratoPlusVolSlide;
    t[0x0C] = PortaPlusVolSlide;
    t[0x11] = Retrig;
    t[0x13] = Extended; // S - table-driven via IT_S_TABLE
    // 0x14 T  SetBpm           - apply_flow_control_effect
    t[0x16] = SetGlobalVolume;
    t[0x17] = GlobalVolSlide;
    t[0x18] = SetPanningIt;
    t[0x1A] = Filter;
    t
};

/// Apply a main-column effect. Returns true if the effect was Extended
/// (the caller routes those to `apply_extended` via the per-format
/// E/S subcommand table).
pub(super) fn apply_effect(
    kind: EffectKind,
    channel: &mut ChannelState,
    mut voice: Option<&mut Voice>,
    ctx: &mut EffectCtx<'_>,
    pattern: &Pattern,
) -> bool {
    match kind {
        EffectKind::None => {}
        EffectKind::Extended => return true,

        EffectKind::Arpeggio => {
            // XM/MOD: arpeggio with no params clears any prior period_shift.
            // S3M/IT: arpeggio is `J` and has memory.
            let has_memory = matches!(ctx.song_type, SongType::S3M | SongType::IT);
            if pattern.effect_param != 0 || has_memory {
                channel.arpeggio(ctx.tick, pattern.get_x(), pattern.get_y(), has_memory);
            } else {
                channel.period_shift = 0;
            }
        }

        EffectKind::PortaUp => {
            channel.porta_up(ctx.song_type, ctx.first_tick, pattern.effect_param);
        }
        EffectKind::PortaDown => {
            channel.porta_down(ctx.song_type, ctx.first_tick, pattern.effect_param);
        }
        EffectKind::PortaToNote => {
            channel.porta_to_note(
                ctx.song_type, voice.as_deref_mut(), ctx.first_tick,
                pattern.effect_param, ctx.compatible_g, ctx.rate, ctx.frequency_tables,
            );
        }
        EffectKind::Vibrato => {
            channel.vibrato(
                voice.as_deref_mut(), ctx.first_tick,
                pattern.get_x(), pattern.get_y(),
                ctx.old_effects, ctx.rate, ctx.frequency_tables, ctx.song_type,
            );
        }
        EffectKind::FineVibrato => {
            channel.fine_vibrato(
                voice.as_deref_mut(), ctx.first_tick,
                pattern.get_x(), pattern.get_y(),
                ctx.old_effects, ctx.rate, ctx.frequency_tables, ctx.song_type,
            );
        }
        EffectKind::Tremolo => {
            channel.tremolo(
                voice.as_deref_mut(), ctx.first_tick,
                pattern.get_x(), pattern.get_y(), ctx.song_type,
            );
        }
        EffectKind::Tremor => {
            channel.tremor(ctx.tick, pattern.effect_param);
        }

        EffectKind::PortaPlusVolSlide => {
            channel.porta_to_note(
                ctx.song_type, voice.as_deref_mut(), ctx.first_tick, 0,
                ctx.compatible_g, ctx.rate, ctx.frequency_tables,
            );
            apply_vol_slide(channel, voice, ctx, pattern.effect_param);
        }
        EffectKind::VibratoPlusVolSlide => {
            channel.vibrato(
                voice.as_deref_mut(), ctx.first_tick, 0, 0,
                ctx.old_effects, ctx.rate, ctx.frequency_tables, ctx.song_type,
            );
            apply_vol_slide(channel, voice, ctx, pattern.effect_param);
        }

        EffectKind::SetPanningXm => {
            if ctx.first_tick {
                if let Some(v) = voice.as_deref_mut() {
                    v.panning.set_panning(pattern.effect_param as i32);
                }
            }
        }
        EffectKind::SetPanningIt => {
            if ctx.first_tick {
                if let Some(v) = voice.as_deref_mut() {
                    v.panning.set_panning((pattern.effect_param as i32 * 4).min(255));
                }
            }
        }
        EffectKind::SetPanningS3m => {
            if ctx.first_tick {
                if let Some(v) = voice.as_deref_mut() {
                    v.panning.set_panning(pattern.effect_param as i32);
                }
            }
        }
        EffectKind::SampleOffset => {
            if ctx.first_tick {
                let param = channel.recall_or_set(
                    crate::channel_state::EffectMemorySlot::SampleOffset,
                    pattern.effect_param,
                );
                if let Some(v) = voice.as_deref_mut() {
                    v.sample_position = (param as f32) * 256.0 + 4.0;
                }
            }
        }

        EffectKind::VolSlideXmStyle => {
            // Gating: XM 0xA uses note_delay_first_tick; MOD 0xA uses
            // first_tick. note_delay_first_tick == first_tick when no
            // note delay is active.
            channel.volume_slide_main(voice.as_deref_mut(), ctx.note_delay_first_tick, pattern.effect_param);
        }
        EffectKind::VolSlideItStyle => {
            channel.it_volume_slide(voice.as_deref_mut(), ctx.note_delay_first_tick, pattern.effect_param);
        }
        EffectKind::SetVolume => {
            if ctx.first_tick {
                channel.set_volume(voice.as_deref_mut(), true, pattern.effect_param);
            }
        }
        EffectKind::SetChannelVolume => {
            if ctx.first_tick {
                channel.channel_volume = pattern.effect_param.min(64);
            }
        }
        EffectKind::ChannelVolSlide => {
            channel.channel_volume_slide(ctx.first_tick, pattern.effect_param);
        }
        EffectKind::PanningSlide => {
            channel.panning_slide(voice.as_deref_mut(), ctx.note_delay_first_tick, pattern.effect_param, ctx.song_type);
        }

        EffectKind::Retrig => {
            channel.it_retrig(voice.as_deref_mut(), ctx.instruments, ctx.tick, pattern.effect_param);
        }
        EffectKind::XmMultiRetrig => {
            channel.retrig(
                voice.as_deref_mut(), ctx.instruments, ctx.tick,
                pattern.get_y(), pattern.get_x(),
            );
        }

        EffectKind::SetGlobalVolume => {
            ctx.global_volume.set_volume(ctx.note_delay_first_tick, pattern.effect_param);
        }
        EffectKind::GlobalVolSlide => {
            ctx.global_volume.volume_slide(ctx.note_delay_first_tick, pattern.effect_param);
        }

        EffectKind::KeyOffAtTick => {
            if ctx.tick == pattern.effect_param as u32 {
                if let Some(v) = voice.as_deref_mut() {
                    v.key_off(ctx.instruments, false);
                }
            }
        }
        EffectKind::SetEnvelopePos => {
            if ctx.first_tick {
                if let Some(v) = voice.as_deref_mut() {
                    let inst = &ctx.instruments[v.instrument];
                    v.volume_envelope_state.set_position(&inst.volume_envelope, pattern.effect_param);
                    v.panning_envelope_state.set_position(&inst.panning_envelope, pattern.effect_param);
                    v.pitch_envelope_state.set_position(&inst.pitch_envelope, pattern.effect_param);
                }
            }
        }
        EffectKind::Filter => {
            // IT Z: 0x00..=0x7F sets filter cutoff; 0x80..=0x8F sets
            // resonance (4 bits, scaled << 3).
            if ctx.first_tick {
                if let Some(v) = voice.as_deref_mut() {
                    if pattern.effect_param < 0x80 {
                        v.filter_cutoff = pattern.effect_param;
                    } else if (0x80..=0x8F).contains(&pattern.effect_param) {
                        v.filter_resonance = (pattern.effect_param & 0x0F) << 3;
                    }
                }
            }
        }
    }
    false
}

/// Used by the PortaPlusVolSlide / VibratoPlusVolSlide combo dispatch:
/// XM/MOD use `volume_slide_main` (XM-style nibble decode), IT/S3M use
/// `it_volume_slide` (IT-style with fine variants encoded in nibbles).
/// In every backend the combo gates on first_tick (not
/// note_delay_first_tick) - matches the pre-extraction code.
fn apply_vol_slide(
    channel: &mut ChannelState,
    voice: Option<&mut Voice>,
    ctx: &EffectCtx<'_>,
    param: u8,
) {
    match ctx.song_type {
        SongType::XM | SongType::MOD => {
            channel.volume_slide_main(voice, ctx.first_tick, param);
        }
        _ => {
            channel.it_volume_slide(voice, ctx.note_delay_first_tick, param);
        }
    }
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
