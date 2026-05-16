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

/// Per-format scheduling for SDx / EDx note-delay rows.
#[derive(Clone, Copy)]
pub(super) struct DelaySchedule {
    /// True: vol col fires at the trigger tick (overrides retrig vol).
    /// False: vol col fires at row tick 0 on the still-ringing previous
    /// voice; the new voice triggers later at instrument-default vol.
    pub vol_col_at_trigger: bool,
}

const S3M_DELAY: DelaySchedule = DelaySchedule { vol_col_at_trigger: true };
const IT_DELAY:  DelaySchedule = DelaySchedule { vol_col_at_trigger: true };
const XM_DELAY:  DelaySchedule = DelaySchedule { vol_col_at_trigger: true };
const MOD_DELAY: DelaySchedule = DelaySchedule { vol_col_at_trigger: true };

pub(super) fn delay_schedule(song_type: SongType) -> DelaySchedule {
    match song_type {
        SongType::S3M => S3M_DELAY,
        SongType::IT  => IT_DELAY,
        SongType::XM  => XM_DELAY,
        SongType::MOD => MOD_DELAY,
        _             => MOD_DELAY,
    }
}

/// Per-format mixer parameters for the per-voice volume loop and the
/// master-gain calculation in `output.rs`.
/// Per-format pan-law selector. Decides what mapping from the
/// per-voice `final_panning` byte (0..255) to the L/R gain pair we use
/// at the mixing stage.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PanLaw {
    /// FT2 sqrt LUT (`PANNING_TAB`): `gain = sqrt(pan/256)`. Center
    /// (pan=128) → L=R≈0.707, sum ≈ 1.414. OMT's `FT2Panning`. XM.
    Ft2Sqrt,
    /// Pure linear: `leftVol = (256-pan)/256`, `rightVol = pan/256`.
    /// Center → L=R=0.5, sum = 1.0 (3 dB quieter than `Ft2Sqrt` at
    /// center; identical at hard left/right). OMT's `NoSoftPanning`,
    /// which is the default for IT/S3M/MOD under `MixLevels::Compatible`.
    Linear,
}

#[derive(Clone, Copy)]
pub(super) struct VoiceMixFormula {
    /// XM/S3M/IT update envelopes per tick; MOD has none.
    pub update_envelopes: bool,
    /// Multiply by `channel.channel_volume / 64`. S3M-only (`Mxx`).
    pub channel_vol: bool,
    /// Multiply by `voice.instrument_global_volume / 128`. IT-only.
    pub instrument_global: bool,
    /// Multiply by `global_volume / global_vol_div`. MOD has no song
    /// global volume so this is gated by `apply_global_vol`.
    pub apply_global_vol: bool,
    pub global_vol_div: f32,
    /// Mask before computing master_gain. S3M packs `stereo on` into
    /// bit 7 (e.g. 0xB0 = stereo + master 0x30); mask 0x7F there.
    pub master_byte_mask: u8,
    /// Post-master scaling factor, empirically calibrated per format.
    pub global_scale: f32,
    /// Per-channel frequency multiplier. MOD compensates for using the
    /// FT2 Amiga clock instead of Protracker PAL (14187580/14317456 ≈
    /// 0.99093, ~16 cents). Other formats: 1.0.
    pub freq_scale: f32,
    /// L/R gain mapping at the final mix stage. See `PanLaw`.
    pub pan_law: PanLaw,
}

const XM_MIX:  VoiceMixFormula = VoiceMixFormula {
    update_envelopes: true,  channel_vol: false, instrument_global: false,
    apply_global_vol: true,  global_vol_div: 64.0,
    // Calibrated empirically: 25/34-song XM corpus inlier-median sat
    // at 0.947 with FRAC_1_SQRT_2 (0.7071). Bumping the scale by
    // 1/0.947 = 1.056 lands the bulk of the corpus at ratio ~1.0.
    // The bumped value is close to OMT's MixLevels::CompatibleFT2
    // 192/256 sample-pre-amp ratio (0.75), via 0.7071 * 1.056 ≈
    // 0.7468 ≈ sqrt(192/256) = 0.866 / sqrt(2). Coincidence or not,
    // empirical median is the source of truth for matching OMT.
    master_byte_mask: 0xFF,  global_scale: 0.7468,
    freq_scale: 1.0,
    pan_law: PanLaw::Ft2Sqrt,
};
const MOD_MIX: VoiceMixFormula = VoiceMixFormula {
    update_envelopes: false, channel_vol: false, instrument_global: false,
    apply_global_vol: false, global_vol_div: 1.0,
    // Calibrated empirically: 23/27-song MOD inlier-median sat at
    // 0.900 with FRAC_1_SQRT_2. Bump by 1/0.900 = 1.111 → 0.7857.
    // Most MODs use 4 channels at hard-pan 0/255 with the Ft2Sqrt
    // pan law — at hard pan the FT2 LUT delivers 1.0, so the master
    // calibration directly scales the audible output.
    master_byte_mask: 0xFF,  global_scale: 0.7857,
    freq_scale: 14187580.0 / 14317456.0,
    pan_law: PanLaw::Ft2Sqrt,
};
const S3M_MIX: VoiceMixFormula = VoiceMixFormula {
    update_envelopes: true,  channel_vol: true,  instrument_global: false,
    apply_global_vol: true,  global_vol_div: 64.0,
    master_byte_mask: 0x7F,  global_scale: std::f32::consts::SQRT_2,
    freq_scale: 1.0,
    pan_law: PanLaw::Ft2Sqrt,
};
const IT_MIX:  VoiceMixFormula = VoiceMixFormula {
    update_envelopes: true,  channel_vol: true, instrument_global: true,
    apply_global_vol: true,  global_vol_div: 128.0,
    // OMT loads IT with `MixLevels::Compatible` → `PanningMode::NoSoftPanning`
    // — pure linear L/R split, not the FT2 sqrt LUT. The prior 1.5
    // calibration absorbed the 1.414× over-loud contribution from
    // `Ft2Sqrt` at center pan; with the law now correct (see
    // `pan_law` below), bump global_scale by 1/0.75 ≈ 1.33 so the
    // per-channel diff_bisect ratios against OMT stay near 1.0
    // (was uniformly ~0.75 after the pan-law switch).
    master_byte_mask: 0xFF,  global_scale: 2.0,
    freq_scale: 1.0,
    // OMT loads IT with `MixLevels::Compatible` → `PanningMode::NoSoftPanning`
    // — pure linear L/R split, not the FT2 sqrt LUT. Net effect at center
    // pan: 1.414× quieter than `Ft2Sqrt`. Closes the per-channel
    // monochrome_crisis ch3/12/15 over-loud gap.
    pan_law: PanLaw::Linear,
};
const STM_MIX: VoiceMixFormula = VoiceMixFormula {
    update_envelopes: false, channel_vol: false, instrument_global: false,
    apply_global_vol: false, global_vol_div: 1.0,
    master_byte_mask: 0x7F,  global_scale: std::f32::consts::SQRT_2,
    freq_scale: 1.0,
    pan_law: PanLaw::Ft2Sqrt,
};

pub(super) fn voice_mix(song_type: SongType) -> &'static VoiceMixFormula {
    match song_type {
        SongType::XM  => &XM_MIX,
        SongType::S3M => &S3M_MIX,
        SongType::IT  => &IT_MIX,
        SongType::STM => &STM_MIX,
        _             => &MOD_MIX,
    }
}

/// Run the per-voice update + output_volume computation for every active
/// voice. Replaces the four near-duplicate "Process all active voices"
/// loops the backends used to inline. The format-specific axis (which
/// factors apply, with what divisors) lives entirely in the `mix` table.
pub(super) fn process_voices(
    voices: &mut [crate::channel_state::Voice],
    channels: &[crate::channel_state::ChannelState],
    instruments: &Vec<crate::instrument::Instrument>,
    rate: f32,
    global_volume: u32,
    mix: &VoiceMixFormula,
) {
    for voice in voices.iter_mut() {
        if !voice.on { continue; }
        let channel = &channels[voice.channel_idx];
        let silenced = channel.force_off || channel.tremor_silenced;

        if mix.update_envelopes {
            voice.update_envelopes(instruments, rate);
        }
        voice.update_fadeout();

        let mut v = voice.compute_base_volume();
        if mix.channel_vol {
            v *= channel.channel_volume as f32 / 64.0;
        }
        if mix.instrument_global {
            v *= voice.instrument_global_volume as f32 / 128.0;
        }
        if mix.apply_global_vol {
            v *= global_volume as f32 / mix.global_vol_div;
        }
        voice.set_output_volume(if silenced { 0.0 } else { v });
    }
}

/// Per-channel timing for the row currently being processed: the tick
/// (within the row) at which each per-row event fires. For most rows
/// everything lives on tick 0; SDx/EDx note-delay shifts the trigger
/// (and per the format, possibly the vol col) to `delay = pattern.get_y()`.
#[derive(Clone, Copy, Debug)]
pub(super) struct RowTiming {
    pub trigger_tick: u32,
    pub vol_col_tick: u32,
}

impl RowTiming {
    pub(super) fn for_row(pattern: &Pattern, song_type: SongType) -> Self {
        let trigger_tick = if pattern.is_note_delay(song_type) {
            pattern.get_y() as u32
        } else {
            0
        };
        let vol_col_tick = if delay_schedule(song_type).vol_col_at_trigger {
            trigger_tick
        } else {
            0
        };
        Self { trigger_tick, vol_col_tick }
    }
}

/// Pre-iteration boilerplate every backend repeats at the top of its
/// per-channel loop: reset transient flags, latch `last_instrument` from
/// the row's instrument byte, and compute the gating used by every
/// downstream block.
///
/// Returns `note_delay_first_tick`: the tick at which a note-delayed row
/// (XM `EDx` / S3M/IT `SDx`) should fire its trigger; falls back to
/// `first_tick` when the row carries no note delay. New code should
/// prefer `RowTiming::for_row` for per-event timing — this bool is kept
/// for the existing call sites that haven't migrated yet.
pub(super) fn init_channel_iter(
    channel: &mut ChannelState,
    pattern: &Pattern,
    instruments: &[Instrument],
    song_type: SongType,
    tick: u32,
    first_tick: bool,
) -> bool {
    channel.tremor_silenced = false;
    channel.vibrato_active_this_row = pattern.has_vibrato(song_type);
    if pattern.instrument != 0 {
        channel.last_instrument = if (pattern.instrument as usize) < instruments.len() {
            pattern.instrument as usize
        } else {
            0
        };
        // FT2: an instrument number resets the tremor counter (even on
        // a row without a note) so the next Txx restarts a fresh
        // on-phase. OpenMPT test cases: TremorInstr.xm, TremorRecover.xm.
        // S3M / IT don't have this rule documented in tests; gate to
        // formats that use the FT2 tremor mixer behaviour.
        if matches!(song_type, SongType::XM | SongType::MOD) {
            channel.tremor_count = 0;
            channel.tremor_silenced = false;
        }
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

/// Cut `voices[v_idx]` and invalidate the owning channel's `voice_idx` if it
/// pointed here. Use this for every "voice.on = false" path so the voice-pool
/// invariant (host channel ↔ voice ownership) is maintained.
///
/// The cut is *deferred* — we set `pending_cut` and the mixer ramps the
/// voice's gain to 0 over ~5 ms before flipping `on` to false. The
/// channel ownership pointer is cleared immediately so subsequent note
/// triggers don't try to interact with the dying voice (they'll get a
/// fresh slot from `alloc_voice`, leaving this one to ramp out in
/// peace). Without the ramp, hard-cutting a voice mid-cycle produces
/// the step-discontinuity pops that are obvious in orbiter and present
/// quietly in every other module.
pub(super) fn cut_voice(
    voices: &mut [Voice],
    channels: &mut [ChannelState],
    v_idx: usize,
    reason: crate::channel_state::VoiceCutReason,
) {
    if !voices[v_idx].on || voices[v_idx].pending_cut { return; }
    let ci = voices[v_idx].channel_idx;
    // Short-circuit: an already-silent voice doesn't need a ramp.
    let already_silent =
        voices[v_idx].current_left_vol.abs() < 1e-5
        && voices[v_idx].current_right_vol.abs() < 1e-5
        && voices[v_idx].volume.output_volume < 1e-5;
    if already_silent {
        voices[v_idx].on = false;
        voices[v_idx].cut_reason = Some(reason);
        if ci < channels.len() && channels[ci].voice_idx == Some(v_idx) {
            channels[ci].voice_idx = None;
        }
        return;
    }
    // Mark in-place for ramp-out. The host slot stays occupied until
    // the mixer finishes the ramp; the channel pointer is cleared so
    // future events can't act on the dying voice.
    voices[v_idx].pending_cut = true;
    voices[v_idx].cut_reason = Some(reason);
    if ci < channels.len() && channels[ci].voice_idx == Some(v_idx) {
        channels[ci].voice_idx = None;
    }
}

/// Move the contents of an actively-playing voice into a background
/// slot and mark THAT copy with `pending_cut`. The mixer will continue
/// to play the snapshot's sample (at its current sample_position)
/// while ramping its gain to 0 over the next ~5 ms; meanwhile the
/// source slot is freed (`on = false`) so the upcoming `alloc_voice`
/// for the new note can take it. During the 5 ms overlap window both
/// voices contribute — background fades out, new voice fades in,
/// summing to a smooth crossfade. Without this snapshot the old
/// voice's audio disappears the instant the new trigger lands, and
/// the new voice's 0-to-target fade-in leaves a brief amplitude gap
/// (the residual pop the gain-only ramp couldn't fix).
///
/// Caller-side contract: this function only touches `voices`. Any
/// `channels[ci].voice_idx` that pointed to `src_idx` is left in
/// place — `bind_voice_for_channel` resets it during the upcoming
/// new-note allocation. Pass through this entry point from every
/// "abrupt cut" site (NNA Cut, NoteAction::Cut, DCA Cut) so the
/// snapshot semantics stay consistent.
///
/// Falls back to in-place `pending_cut` on the source slot when no
/// idle slot is free — the 256-slot pool makes that unlikely, but the
/// fallback keeps behaviour deterministic if it ever happens.
pub(super) fn spawn_background_cut_inline(
    voices: &mut [Voice],
    src_idx: usize,
    reason: crate::channel_state::VoiceCutReason,
) {
    if !voices[src_idx].on || voices[src_idx].pending_cut {
        return;
    }
    if let Some(bg) = voices.iter().position(|v| !v.on) {
        if bg != src_idx {
            voices[bg] = voices[src_idx];
            voices[bg].pending_cut = true;
            voices[bg].cut_reason = Some(reason);
        }
        voices[src_idx].on = false;
        voices[src_idx].cut_reason = Some(reason);
    } else {
        voices[src_idx].pending_cut = true;
        voices[src_idx].cut_reason = Some(reason);
    }
}


pub(super) fn alloc_voice(voices: &mut [Voice]) -> usize {
    // Prefer truly idle slots (on==false). A `pending_cut` voice is
    // technically still on (mixer is ramping it out) — skip those so
    // we don't yank away the ramp; pick another slot instead.
    for (vi, v) in voices.iter().enumerate() {
        if !v.on { return vi; }
    }
    // Fall back to slots ramping out — better than evicting a live
    // voice if every slot is busy. spx-shuttledeparture hits this on
    // long sustained NNA tails; the steal-quietest path below is the
    // last resort.
    //
    // NOTE: both fallback paths return a slot whose `voice.channel_idx`
    // still points at its previous owner. The caller must clear
    // `channels[prev_owner].voice_idx` after init_voice_basics
    // overwrites the channel_idx, otherwise a stale host pointer
    // persists (voice-pool invariant warnings). See the cleanup at
    // the trigger sites in xm/s3m/mod_/it.rs.
    for (vi, v) in voices.iter().enumerate() {
        if v.pending_cut { return vi; }
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

/// Per-tick voice-pool invariant check. A channel's `voice_idx` may legitimately
/// point at a quiescent slot (`voice.on == false`) that's been silenced or
/// reassigned — the next trigger on that channel will overwrite it. The bug
/// state we want to surface is when a channel's pointer references an *active*
/// voice currently owned by a *different* channel: that means the other
/// channel reused the slot without invalidating the original pointer, and the
/// stale pointer will silently corrupt audio (the original NoteAction::Off
/// regression on mview ch8 was exactly this).
///
/// Debug builds panic with the channel indices + slot number; release builds
/// log once per occurrence so corpus rendering doesn't get gated on it.
pub(super) fn validate_voice_pool(voices: &[Voice], channels: &[ChannelState]) {
    // Observer only — never touches state. If a stale pointer survives
    // to validation, that means the proactive cleanup at the trigger
    // site (the `stale_clears` Vec applied after each per-channel
    // loop in xm/s3m/mod_/it.rs) missed a case. Repair-in-validator
    // would hide that regression behind a self-healing veneer; we'd
    // rather see the warning. Debug builds panic so the offending
    // path is easy to pinpoint; release builds log each distinct
    // (channel, slot, host) triple once.
    for (ci, ch) in channels.iter().enumerate() {
        let Some(vi) = ch.voice_idx else { continue };
        if vi >= voices.len() { continue; }
        let v = &voices[vi];
        if v.on && v.channel_idx != ci {
            let msg = format!(
                "voice-pool invariant violated: channel {} → voice_idx=Some({}), \
                 but voice {} is active and owned by channel {}",
                ci, vi, vi, v.channel_idx,
            );
            #[cfg(debug_assertions)]
            panic!("{}", msg);
            #[cfg(not(debug_assertions))]
            {
                use std::sync::Mutex;
                use std::collections::HashSet;
                static SEEN: Mutex<Option<HashSet<(usize, usize, usize)>>> = Mutex::new(None);
                let key = (ci, vi, v.channel_idx);
                let mut guard = SEEN.lock().unwrap();
                let set = guard.get_or_insert_with(HashSet::new);
                if set.insert(key) {
                    eprintln!("[voice-pool] {}", msg);
                }
            }
        }
    }
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
    // Fresh note triggers fade in from silence — otherwise reusing a
    // slot that still holds the previous voice's instantaneous gain
    // would re-introduce the step-discontinuity click we're trying to
    // avoid.
    voice.reset_ramp_for_new_note();
}

/// Apply the previous-voice-on-channel handling that runs before alloc_voice
/// in XM, MOD, and S3M backends. XM and MOD always cut; S3M dispatches via
/// the instrument's NNA. Cmwt with IT's DCT/DCA logic isn't covered here -
/// that block is more involved and stays inline in ItBackend.
pub(super) fn cut_or_nna_existing_voice(
    voices: &mut [Voice],
    channel: &mut ChannelState,
    instruments: &Vec<Instrument>,
    song_type: SongType,
    channel_idx: usize,
    prev_voice_idx: usize,
) {
    let v = &voices[prev_voice_idx];
    if !(v.on && v.channel_idx == channel_idx) { return; }
    // Helper: snapshot the voice into a background slot, then clear the
    // host channel's voice_idx if it was pointing at this slot. Without
    // the clear, a later alloc_voice call may hand the freed slot to a
    // different channel and the old pointer becomes a stale reference.
    let cut_and_clear = |voices: &mut [Voice], ch: &mut ChannelState, idx: usize| {
        spawn_background_cut_inline(voices, idx,
            crate::channel_state::VoiceCutReason::NoteCut);
        if ch.voice_idx == Some(idx) { ch.voice_idx = None; }
    };
    match song_type {
        SongType::XM | SongType::MOD => {
            cut_and_clear(voices, channel, prev_voice_idx);
        }
        _ => {
            let nna = instruments[voices[prev_voice_idx].instrument].nna;
            match nna {
                0 => { cut_and_clear(voices, channel, prev_voice_idx); }
                1 => { /* Continue */ }
                2 => { voices[prev_voice_idx].key_off(instruments, song_type); } // Note Off
                3 => { voices[prev_voice_idx].sustained = false; } // Fade
                _ => { cut_and_clear(voices, channel, prev_voice_idx); }
            }
        }
    }
}

/// End-of-tick mute pass: zero out a voice's output and mark it inactive
/// when it has finished fading or has dropped below the audibility floor.
/// Repeated identically in all four backends after their volume formula.
pub(super) fn mute_silent_voices(voices: &mut [Voice], channels: &mut [ChannelState]) {
    const SILENCE_FLOOR: f32 = 0.00001;
    let mut to_cut: Vec<usize> = Vec::new();
    for (v_idx, voice) in voices.iter().enumerate() {
        if !voice.on { continue; }
        let is_host_voice = channels[voice.channel_idx].voice_idx == Some(v_idx);
        // Host voice: cut only when the fadeout has fully completed. Don't
        // cut on a momentarily-low output_volume — that ends a voice the
        // pattern still references (mview.xm ch11/ch12 ~80s: porta+instrument
        // rows revive a previously key-off'd voice; cutting on the
        // SILENCE_FLOOR pass loses the voice before the revive fires).
        // Stranded non-host voices (NNA leftovers) can be reclaimed once
        // their audible contribution drops below the floor.
        let cut = (!voice.sustained && voice.volume.fadeout_vol == 0)
            || (!is_host_voice && voice.volume.output_volume < SILENCE_FLOOR);
        if cut { to_cut.push(v_idx); }
    }
    for v_idx in to_cut {
        cut_voice(voices, channels, v_idx, crate::channel_state::VoiceCutReason::Faded);
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
    SetFinetuneS3m,     // S3M / IT S2 — channel c5_speed override via S3M_FINETUNE_TABLE
    SetSurround,        // IT S9 / S3M S9 — y=0 disables, y=1 enables (right-channel phase invert)
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
        None,             // E0  set filter (Amiga LED) - not implemented
        FinePortaUp,      // E1
        FinePortaDown,    // E2
        Glissando,        // E3
        VibratoWaveform,  // E4
        SetFinetune,      // E5
        PatternLoop,      // E6
        TremoloWaveform,  // E7
        None,             // E8  (Sync — unused)
        NoteRetrig,       // E9
        FineVolSlideUp,   // EA
        FineVolSlideDown, // EB
        NoteCutAtTick,    // EC
        None,             // ED  note delay (handled in trigger path)
        PatternDelay,     // EE
        None,             // EF
    ]
};

/// S3M's Sxy table.
pub(super) const S3M_S_TABLE: [ExtendedCmdKind; 16] = {
    use ExtendedCmdKind::*;
    [
        None,             // S0  Amiga LED filter (n/a for digital pipeline)
        Glissando,        // S1  set glissando (param != 0 enables semitone-snap porta)
        SetFinetuneS3m,   // S2  set finetune (channel c5_speed via S3M_FINETUNE_TABLE)
        VibratoWaveform,  // S3  vibrato waveform (low 2 bits = sine/sawtooth/square)
        TremoloWaveform,  // S4  tremolo waveform
        None,             // S5  panbrello waveform (IT-only)
        None,             // S6  frame delay (rarely-used IT-era extension)
        None,             // S7  NNA controls (IT-only)
        SetExtraPanning,  // S8  panning (param * 17)
        SetSurround,      // S9  surround on/off (y=0 off, y=1 on; right channel phase-inverted)
        None,             // SA  high sample offset (TODO: combine with O for >65535-byte samples)
        PatternLoop,      // SB
        NoteCutAtTick,    // SC
        None,             // SD  note delay (handled at note-trigger time)
        PatternDelay,     // SE
        None,             // SF  MIDI macro (IT-only)
    ]
};

/// IT's Sxy table — only the subcommands the engine currently honors.
pub(super) const IT_S_TABLE: [ExtendedCmdKind; 16] = {
    use ExtendedCmdKind::*;
    [
        None,             // S0
        Glissando,        // S1
        SetFinetuneS3m,   // S2  (same handler as S3M's; offset chosen from song_type)
        VibratoWaveform,  // S3
        TremoloWaveform,  // S4
        None,             // S5  panbrello waveform (TODO if any IT corpus needs it)
        None,             // S6  frame delay (uncommon)
        None,             // S7  NNA / instr controls (handled at note-trigger logic)
        SetItPanning,     // S8 (param << 4 - 16-step coarse panning)
        SetSurround,      // S9  surround on/off (y=0 off, y=1 on; right channel phase-inverted)
        None,             // SA  high sample offset (TODO)
        PatternLoop,      // SB  pattern loop — same handler as S3M; 1_channel_moog.it
                          // uses 10–12 of these per pattern to build out its
                          // structure within a 4 kB compo budget.
        NoteCutAtTick,    // SC
        None,             // SD  note delay (gated via `is_note_delay` in dispatch)
        PatternDelay,     // SE
        None,             // SF  MIDI macro
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
    /// S3M ST3 fast-volume-slides quirk (cwtv 0x1300 or fastVolSlides
    /// flag bit). When true, vol slides apply on tick 0 too — not just
    /// non-first-ticks. Songs from buggy ST3 v3.00 (e.g. 2ND_PM.S3M)
    /// rely on this; without it slides accumulate 33% less per row and
    /// voices stay audible past their intended cutoff.
    pub fast_volume_slides: bool,
    /// True when the song uses Amiga-period mode (false = linear). S3M is
    /// always Amiga; IT/XM read the flag from their headers. Used by the
    /// S3M/IT arpeggio formula override — in linear mode the existing
    /// period_shift = -(x*64) is exact (LUT is 64 units per semitone), so
    /// the override only kicks in for amiga where it's audibly wrong.
    pub use_amiga:      bool,
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
                // XM/MOD: bit 2 = no-retrig flag. S3M/IT mask to & 3.
                if matches!(song_type, SongType::XM | SongType::MOD) {
                    channel.vibrato_retrig = (y & 4) == 0;
                }
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
                if matches!(song_type, SongType::XM | SongType::MOD) {
                    channel.tremolo_retrig = (y & 4) == 0;
                }
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
                if let Some(v) = voice.as_deref_mut() {
                    v.on = false;
                    v.cut_reason = Some(crate::channel_state::VoiceCutReason::NoteCut);
                }
                channel.voice_idx = None;
            }
        }
        ExtendedCmdKind::PatternDelay => {
            // XM uses first_row_tick gating; S3M/IT use first_tick.
            // Both end up at "set once at the row's first tick".
            //
            // Per OpenMPT PatternDelays.xm: "If there are multiple
            // pattern delays (EEx), only the one on the rightmost
            // channel is considered (even if the EEx parameter is 0)."
            // The per-channel loop processes left-to-right so the last
            // (rightmost) channel's EE write wins naturally — drop the
            // `!delay_processed` early-out that was preserving the
            // leftmost value.
            let gate = match song_type { SongType::XM => first_row_tick, _ => first_tick };
            if gate {
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
        ExtendedCmdKind::SetFinetuneS3m => {
            // S3M / IT S2x: set channel c5_speed and recompute the period.
            if first_tick {
                channel.note.c5_speed = S3M_FINETUNE_TABLE[(y as usize) & 0xF] as u32;
                if channel.note.original_note != 0 {
                    let offset = if song_type == SongType::S3M { 11i8 } else { -1i8 };
                    let p = crate::channel_state::channel_state::Note::note_to_period_s3m(
                        channel.note.original_note, offset, channel.note.c5_speed,
                    );
                    channel.note.period = p;
                    channel.note.base_period = p;
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
        ExtendedCmdKind::SetSurround => {
            // IT/S3M S9x. Only S90/S91 are well-defined here:
            //   y == 0 → surround off
            //   y == 1 → surround on; pan snaps to center (128)
            // Surround playback inverts the right-channel sign so the
            // signal cancels in mono and stereo speakers hear a
            // phase-spread image. (OMT also defines S98..S9F for
            // ModPlug reverb / mixing extensions — ignore those.)
            if first_tick {
                match y {
                    0 => { channel.surround = false; }
                    1 => {
                        channel.surround = true;
                        if let Some(v) = voice.as_deref_mut() {
                            v.panning.set_panning(128);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

}

// Main effect-column dispatch. Each format has a `[EffectKind; 32]`
// table mapping raw effect bytes into a shared enum; `apply_effect`
// dispatches off the enum with `match song_type` for per-format quirks.

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

/// S3M S2x finetune → c5_speed. Geometric: 8363 * 2^((i-8) / 96).
pub(super) const S3M_FINETUNE_TABLE: [u16; 16] = [
    7895, 7941, 7985, 8046, 8107, 8169, 8232, 8280,
    8363, 8413, 8463, 8529, 8581, 8651, 8723, 8757,
];

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
    t[0x1D] = Tremor;  // T (Tremor) — was missing; OpenMPT test cases
                       // Tremor.xm, TremorInstr.xm, TremorRecover.xm
                       // all relied on it.
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
    // IT effect numbering is alphabet-direct: A=1, B=2, ..., Z=26. Same
    // mapping as S3M (see S3M_EFFECT_TABLE above). Keep the two tables
    // in sync — any IT-handled effect that has an S3M analogue should
    // appear at the same index in both.
    // 0x01 A  SetSpeed         - apply_flow_control_effect
    // 0x02 B  PatternJump      - apply_flow_control_effect
    // 0x03 C  PatternBreak     - apply_flow_control_effect
    t[0x04] = VolSlideItStyle;     // D
    t[0x05] = PortaDown;           // E
    t[0x06] = PortaUp;             // F
    t[0x07] = PortaToNote;         // G
    t[0x08] = Vibrato;             // H
    t[0x09] = Tremor;              // I
    t[0x0A] = Arpeggio;            // J
    t[0x0B] = VibratoPlusVolSlide; // K
    t[0x0C] = PortaPlusVolSlide;   // L
    t[0x0D] = SetChannelVolume;    // M
    t[0x0E] = ChannelVolSlide;     // N
    t[0x0F] = SampleOffset;        // O — was missing; ch4..9 of
                                   // monochrome_crisis.it use this to
                                   // start the sample at offset 768
    t[0x10] = PanningSlide;        // P
    t[0x11] = Retrig;              // Q
    t[0x12] = Tremolo;             // R
    t[0x13] = Extended;            // S - table-driven via IT_S_TABLE
    // 0x14 T  SetBpm           - apply_flow_control_effect
    t[0x15] = FineVibrato;         // U
    t[0x16] = SetGlobalVolume;     // V
    t[0x17] = GlobalVolSlide;      // W
    t[0x18] = SetPanningIt;        // X
    t[0x1A] = Filter;              // Z
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
                // S3M/IT amiga: recompute period_shift via the c5_speed
                // formula so arp steps land on exact semitones.
                let arpeggio_via_formula = matches!(ctx.song_type, SongType::S3M | SongType::IT)
                    && ctx.use_amiga;
                if arpeggio_via_formula
                    && channel.note.c5_speed != 0
                    && channel.note.original_note != 0
                {
                    let offset = if ctx.song_type == SongType::S3M { 11i8 } else { -1i8 };
                    let arp_step = match ctx.tick % 3 {
                        1 => pattern.get_x() as i8,
                        2 => pattern.get_y() as i8,
                        _ => 0,
                    };
                    if arp_step != 0 {
                        let base = crate::channel_state::channel_state::Note::note_to_period_s3m(
                            channel.note.original_note, offset, channel.note.c5_speed,
                        ) as i32;
                        let stepped = crate::channel_state::channel_state::Note::note_to_period_s3m(
                            channel.note.original_note, offset + arp_step, channel.note.c5_speed,
                        ) as i32;
                        channel.period_shift = (stepped - base) as i16;
                    }
                } else if ctx.song_type == SongType::MOD && channel.note.original_note != 0 {
                    // MOD arpeggio walks the Amiga period table by note
                    // index, not by a fixed -64 per semitone (that's an
                    // FT2-linear convention). Amiga semitone sizes vary
                    // with octave, so a 7-semitone arp step from period
                    // 113 would otherwise shift period to a negative
                    // value, get clamped to 0, and play at a wildly
                    // wrong rate that exhausts looping samples in a
                    // single buffer (ArpWraparound.mod ch0 row 12+).
                    let arp_step = match ctx.tick % 3 {
                        1 => pattern.get_x() as i8,
                        2 => pattern.get_y() as i8,
                        _ => 0,
                    };
                    if arp_step != 0 {
                        let base_note = channel.note.original_note as i32;
                        let target_note = (base_note + arp_step as i32).clamp(1, 120) as u8;
                        let base = channel.note.note_to_period(
                            base_note as u8, channel.note.finetune, ctx.frequency_tables,
                        ) as i32;
                        let stepped = channel.note.note_to_period(
                            target_note, channel.note.finetune, ctx.frequency_tables,
                        ) as i32;
                        channel.period_shift = (stepped - base) as i16;
                    }
                }
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
            // ST3 quirk (OMT kS3MIgnoreCombinedFineSlides): combined
            // slide commands Kxy and Lxy don't run on the first tick.
            // Fine slides written in the K/L low nibble are
            // effectively no-ops because they'd only fire on tick 0,
            // which we're skipping here. OpenMPT test cases:
            // NoCombinedSlidesOnFirstTick-{Fast,Normal}.s3m.
            let skip_first_tick = ctx.song_type == SongType::S3M && ctx.first_tick;
            if !skip_first_tick {
                apply_vol_slide(channel, voice, ctx, pattern.effect_param);
            }
        }
        EffectKind::VibratoPlusVolSlide => {
            channel.vibrato(
                voice.as_deref_mut(), ctx.first_tick, 0, 0,
                ctx.old_effects, ctx.rate, ctx.frequency_tables, ctx.song_type,
            );
            let skip_first_tick = ctx.song_type == SongType::S3M && ctx.first_tick;
            if !skip_first_tick {
                apply_vol_slide(channel, voice, ctx, pattern.effect_param);
            }
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
                // Always memorize the param so a later note row using
                // bare Oxx (param 00) picks up the most recent value
                // (OMT Snd_fx.cpp:CMD_OFFSET via the effect memory).
                let param = channel.recall_or_set(
                    crate::channel_state::EffectMemorySlot::SampleOffset,
                    pattern.effect_param,
                );
                // Only apply the offset to a voice when the row also
                // triggers a fresh note. Bare Oxx (no note) just sets
                // memory — applying it would scrub the playhead on a
                // sustaining voice and produce phantom audio (OpenMPT
                // S3M test case OxxMemory.s3m: "Should remain silent",
                // 300× too loud before this).
                //
                // FT2 quirk (XM only): porta-to-note + offset on the
                // same row ignores the offset entirely — see OMT's
                // porta-offset.xm test note.
                let triggers = matches!(
                    pattern.note_action(ctx.song_type),
                    crate::pattern::NoteAction::Trigger(_),
                );
                let xm_porta_offset_skip = ctx.song_type == SongType::XM
                    && pattern.is_porta_to_note(ctx.song_type);
                if triggers && !xm_porta_offset_skip {
                    if let Some(v) = voice.as_deref_mut() {
                        let mut target = (param as u32) * 256;
                        // S3M Oxx loop wrap (GUS-mode quirk): if the
                        // offset lands past the end of a looping
                        // sample's loop, wrap back into the loop.
                        // SoundBlaster stops instead, but OpenMPT
                        // emulates the GUS variant. Test case:
                        // OffsetLoopWraparound.s3m.
                        if ctx.song_type == SongType::S3M {
                            let sample = &ctx.instruments[v.instrument]
                                .samples
                                .get(v.sample);
                            if let Some(sample) = sample {
                                if sample.loop_type != crate::instrument::LoopType::NoLoop
                                    && sample.loop_end > 0
                                    && target >= sample.loop_end
                                {
                                    let loop_span = sample.loop_end - sample.loop_start;
                                    if loop_span > 0 {
                                        target = (target - sample.loop_start) % loop_span
                                            + sample.loop_start;
                                    }
                                }
                            }
                        }
                        v.sample_position = target as f32 + 4.0;
                    }
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
            channel.it_volume_slide(voice.as_deref_mut(), ctx.note_delay_first_tick, pattern.effect_param, ctx.fast_volume_slides);
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
            channel.panning_slide(voice.as_deref_mut(), ctx.first_tick, pattern.effect_param, ctx.song_type);
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
                    v.key_off(ctx.instruments, ctx.song_type);
                }
            }
        }
        EffectKind::SetEnvelopePos => {
            if ctx.first_tick {
                if let Some(v) = voice.as_deref_mut() {
                    let inst = &ctx.instruments[v.instrument];
                    let is_xm = matches!(ctx.song_type, SongType::XM | SongType::MOD);
                    let set_vol = if is_xm { inst.volume_envelope.on } else { true };
                    if set_vol {
                        v.volume_envelope_state.set_position(&inst.volume_envelope, pattern.effect_param);
                    }
                    // FT2 logic bug: pan-env (and pitch) position gates on
                    // vol-env's sustain flag, not its own.
                    let set_pan = if is_xm { inst.volume_envelope.sustain } else { true };
                    if set_pan {
                        v.panning_envelope_state.set_position(&inst.panning_envelope, pattern.effect_param);
                        v.pitch_envelope_state.set_position(&inst.pitch_envelope, pattern.effect_param);
                    }
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
            channel.it_volume_slide(voice, ctx.first_tick, param, ctx.fast_volume_slides);
        }
    }
}

// --- Table-driven vol-col dispatch ---------------------------------------
//
// Each format's volume-column layout is a sorted list of (byte_range →
// action) rows that read like the FT2 / IT spec sheet. `dispatch_vol_col`
// is the one engine: find the matching row, compute the magnitude
// (`byte - row.start`), and route it to the right channel method.
// Adding a new vol-col mapping is a single `VolColEntry` row.

#[derive(Copy, Clone, Debug)]
pub(super) enum Sign { Up, Down }

#[derive(Copy, Clone, Debug)]
pub(super) enum VolColAction {
    /// `mag` is the new volume (0..=64). Tick gate: note_delay_first_tick.
    SetVolume,
    /// `mag` is the signed step magnitude (signed by `sign`).
    /// `fine` selects fine-vs-running; `with_memory` selects the IT
    /// memory-bearing variant. Tick gate is encoded by the four
    /// combinations:
    ///   running, no memory  (XM 0x60/0x70)    → first_tick
    ///   running, with memory (IT 85-94/95-104) → first_tick
    ///   fine, no memory     (XM 0x80/0x90)    → first_tick
    ///   fine, with memory   (IT 65-74/75-84)  → note_delay_first_tick
    VolSlide { sign: Sign, fine: bool, with_memory: bool },
    /// `(mag << shift)` → `porta_up`. IT vol-col 105-114 uses shift=2.
    PortaUp { shift: u8 },
    /// `(mag << shift)` → `porta_down`. IT vol-col 115-124 uses shift=2.
    PortaDown { shift: u8 },
    /// New panning = `mag * scale` clamped to 0..=255. No tick gate (the
    /// inline match also fires every tick; semantically idempotent).
    /// XM 0xc0-0xcf uses scale=17 (0..15 → 0..255). IT 128-192 uses scale=4.
    SetPanning { scale: u8 },
    /// XM vol-col D (low nibble) or E (high nibble) → `panning_slide`.
    PanningSlide { hi_nibble: bool },
    /// XM A (set speed: depth=0) / XM B + IT 203-212 (set depth: speed=0).
    Vibrato { set_speed: bool },
    /// Tone porta. `with_memory=true` routes to `it_vol_col_porta_to_note`.
    PortaToNote { with_memory: bool },
}

#[derive(Copy, Clone, Debug)]
pub(super) struct VolColEntry {
    pub start: u8,                // inclusive
    pub end: u8,                  // inclusive
    pub action: VolColAction,
}

/// One row per byte-range. Walked in order; first match wins. Bytes that
/// don't fall in any row (including `0xFF` "no volume" sentinel) are
/// silently ignored — same as the previous inline `_ => {}` arm.
pub(super) fn dispatch_vol_col(
    table: &[VolColEntry],
    raw_byte: u8,
    channel: &mut ChannelState,
    mut voice: Option<&mut crate::channel_state::Voice>,
    ctx: &EffectCtx<'_>,
) {
    for entry in table {
        if raw_byte < entry.start || raw_byte > entry.end { continue; }
        let mag = raw_byte - entry.start;
        match entry.action {
            VolColAction::SetVolume => {
                channel.set_volume(voice.as_deref_mut(), ctx.note_delay_first_tick, mag);
            }
            VolColAction::VolSlide { sign, fine, with_memory } => {
                let signed = match sign {
                    Sign::Up => mag as i8,
                    Sign::Down => -(mag as i8),
                };
                match (fine, with_memory) {
                    (false, false) => channel.volume_slide(voice.as_deref_mut(), ctx.first_tick, signed),
                    (true,  false) => channel.fine_volume_slide(voice.as_deref_mut(), ctx.first_tick, signed),
                    (false, true)  => channel.it_vol_col_volume_slide(voice.as_deref_mut(), ctx.first_tick, signed),
                    (true,  true)  => channel.it_vol_col_fine_volume_slide(voice.as_deref_mut(), ctx.note_delay_first_tick, signed),
                }
            }
            VolColAction::PortaUp { shift } => {
                channel.porta_up(ctx.song_type, ctx.first_tick, mag << shift);
            }
            VolColAction::PortaDown { shift } => {
                channel.porta_down(ctx.song_type, ctx.first_tick, mag << shift);
            }
            VolColAction::SetPanning { scale } => {
                if let Some(v) = voice.as_deref_mut() {
                    v.panning.set_panning(((mag as i32) * (scale as i32)).min(255));
                }
            }
            VolColAction::PanningSlide { hi_nibble } => {
                let p = if hi_nibble { mag << 4 } else { mag };
                channel.panning_slide(voice.as_deref_mut(), ctx.first_tick, p, ctx.song_type);
            }
            VolColAction::Vibrato { set_speed } => {
                let (s, d) = if set_speed { (mag, 0) } else { (0, mag) };
                channel.vibrato(
                    voice.as_deref_mut(), ctx.first_tick,
                    s, d, ctx.old_effects, ctx.rate, ctx.frequency_tables, ctx.song_type,
                );
            }
            VolColAction::PortaToNote { with_memory } => {
                if with_memory {
                    channel.it_vol_col_porta_to_note(
                        voice.as_deref_mut(), ctx.note_delay_first_tick,
                        mag, ctx.compatible_g, ctx.rate, ctx.frequency_tables,
                    );
                } else {
                    channel.porta_to_note(
                        ctx.song_type, voice.as_deref_mut(), ctx.note_delay_first_tick,
                        mag, ctx.compatible_g, ctx.rate, ctx.frequency_tables,
                    );
                }
            }
        }
        return;
    }
}

/// XM volume-column layout. Reads like the FT2 spec sheet:
/// `0x10..0x50` set volume, `0x60..0x6f` vol-slide down (running), etc.
pub(super) const XM_VOL_COL: &[VolColEntry] = &[
    VolColEntry { start: 0x10, end: 0x50, action: VolColAction::SetVolume },
    VolColEntry { start: 0x60, end: 0x6f, action: VolColAction::VolSlide { sign: Sign::Down, fine: false, with_memory: false } },
    VolColEntry { start: 0x70, end: 0x7f, action: VolColAction::VolSlide { sign: Sign::Up,   fine: false, with_memory: false } },
    VolColEntry { start: 0x80, end: 0x8f, action: VolColAction::VolSlide { sign: Sign::Down, fine: true,  with_memory: false } },
    VolColEntry { start: 0x90, end: 0x9f, action: VolColAction::VolSlide { sign: Sign::Up,   fine: true,  with_memory: false } },
    // FT2 vol-col: A = set vibrato speed, B = set depth + apply.
    VolColEntry { start: 0xa0, end: 0xaf, action: VolColAction::Vibrato { set_speed: true } },
    VolColEntry { start: 0xb0, end: 0xbf, action: VolColAction::Vibrato { set_speed: false } },
    VolColEntry { start: 0xc0, end: 0xcf, action: VolColAction::SetPanning { scale: 17 } },
    // FT2 vol-col: D = pan-slide left (lo nibble), E = pan-slide right (hi nibble).
    VolColEntry { start: 0xd0, end: 0xdf, action: VolColAction::PanningSlide { hi_nibble: false } },
    VolColEntry { start: 0xe0, end: 0xef, action: VolColAction::PanningSlide { hi_nibble: true  } },
    VolColEntry { start: 0xf0, end: 0xfe, action: VolColAction::PortaToNote { with_memory: false } },
];

/// IT volume-column layout (decimal range encoding; ranges of 10 except
/// the 0..64 set-volume head and the 128..192 set-panning span).
pub(super) const IT_VOL_COL: &[VolColEntry] = &[
    VolColEntry { start: 0,   end: 64,  action: VolColAction::SetVolume },
    VolColEntry { start: 65,  end: 74,  action: VolColAction::VolSlide { sign: Sign::Up,   fine: true,  with_memory: true } },
    VolColEntry { start: 75,  end: 84,  action: VolColAction::VolSlide { sign: Sign::Down, fine: true,  with_memory: true } },
    VolColEntry { start: 85,  end: 94,  action: VolColAction::VolSlide { sign: Sign::Up,   fine: false, with_memory: true } },
    VolColEntry { start: 95,  end: 104, action: VolColAction::VolSlide { sign: Sign::Down, fine: false, with_memory: true } },
    VolColEntry { start: 105, end: 114, action: VolColAction::PortaUp   { shift: 2 } },
    VolColEntry { start: 115, end: 124, action: VolColAction::PortaDown { shift: 2 } },
    VolColEntry { start: 128, end: 192, action: VolColAction::SetPanning { scale: 4 } },
    VolColEntry { start: 193, end: 202, action: VolColAction::PortaToNote { with_memory: true } },
    VolColEntry { start: 203, end: 212, action: VolColAction::Vibrato { set_speed: false } },
];

/// S3M volume-column: just plain set-volume in 0..=64.
pub(super) const S3M_VOL_COL: &[VolColEntry] = &[
    VolColEntry { start: 0, end: 64, action: VolColAction::SetVolume },
];

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
    channel.period_shift = 0;
    channel.tremor_count = 0;
    channel.update_frequency_voice(voice, rate, false, frequency_tables);
}
