use crate::channel_state::channel_state::{EnvelopeState, Note, Panning, PortaToNoteState, TremoloState, VibratoState, Volume, VibratoEnvelopeState, WaveControl};
use std::sync::atomic::{AtomicI32, Ordering};

/// Debug context for the OUR_DUMP_CH-gated `[OUR]` trace; the OMT-side
/// counterpart lives in tools/openmpt_instrumentation.patch.
pub static DUMP_CTX_ORD: AtomicI32 = AtomicI32::new(-1);
pub static DUMP_CTX_ROW: AtomicI32 = AtomicI32::new(-1);
pub static DUMP_CTX_TICK: AtomicI32 = AtomicI32::new(-1);
use crate::instrument::Instruments;
use crate::tables::AudioTables;
use crate::module_reader::SongType;
// use crate::module_reader::is_note_valid;
// use std::num::Wrapping;
// use std::cmp::{min, max};

pub(crate) mod channel_state;

#[derive(Clone,Copy,Debug)]
pub(crate) struct SplineData {
    pub(crate) p0: f32,
    pub(crate) p1: f32,
    pub(crate) p2: f32,
    pub(crate) p3: f32,
}

impl SplineData {
    fn new() -> Self {
        Self {
            p0: 0.0,
            p1: 0.0,
            p2: 0.0,
            p3: 0.0
        }
    }
    pub fn interpolate(&self, t: f32) -> f32 {
        let p0 = self.p0;
        let p1 = self.p1;
        let p2 = self.p2;
        let p3 = self.p3;

        let c3 =      -p0 + 3.0 * p1 - 3.0 * p2 + p3;
        let c2 = 2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3;
        let c1 =      -p0                  + p2;
        let c0 =                  p1;

        0.5 * (((c3 * t + c2) * t) + c1) * t + c0
    }

    #[allow(dead_code)]
    pub fn push(&mut self, p: f32) {
        self.p0 = self.p1;
        self.p1 = self.p2;
        self.p2 = self.p3;
        self.p3 = p;
    }
}

#[derive(Clone,Copy,Debug)]
pub struct Voice {
    pub        instrument:                     usize,
    pub        sample:                         usize,
    pub        frequency:                      f32,
    pub        du:                             f32,
    pub        volume:                         Volume,
    pub        sample_position:                f32,
    pub        loop_started:                   bool,
    pub        sustained:                      bool,
    pub(crate) spline_data:                    SplineData,
    
    // Playback state moved from ChannelState
    pub volume_envelope_state:          EnvelopeState,
    pub panning_envelope_state:         EnvelopeState,
    pub pitch_envelope_state:           EnvelopeState,
    pub vibrato_envelope_state:         VibratoEnvelopeState,
    pub vibrato_state:                  VibratoState,
    pub tremolo_state:                  TremoloState,
    pub tremolo_shift:                  f32,
    pub frequency_shift:                f32,
    pub        panning:                        Panning,
    pub(crate) instrument_global_volume:       u8,
    pub(crate) sample_global_volume:           u8,
    pub(crate) filter_cutoff:                  u8,
    pub(crate) filter_resonance:               u8,
    pub(crate) filter_state:                   ResonantFilter,
    pub on:                             bool,
    pub surround:                       bool,
    pub channel_idx:                    usize,
    pub(crate) last_played_note:               u8,

    // ---- mixer instrumentation (write-only telemetry; never read by the
    // playback path) -----------------------------------------------------
    /// Global tick counter (`Song.tick_counter`-equivalent) of the last
    /// time this voice was actually rendered by `Song::output_channels`.
    /// Distinguishes "still being mixed" from "trigger fired but mixer
    /// has since cut us" — useful when reading state_dump output, which
    /// otherwise sees `voice.sample_position` frozen at the trigger value
    /// and can't tell the two states apart. 0 = never rendered.
    pub last_render_tick:               u64,
    /// Reason the mixer set `voice.on = false`, if any. Persists across
    /// the cut so post-mortem dumps can attribute silenced voices.
    pub cut_reason:                     Option<VoiceCutReason>,

    // ---- per-sample volume ramping (anti-click) ----
    /// Instantaneous L/R gain currently applied at the mixer. Updated
    /// per output sample by `left_ramp_step` / `right_ramp_step`. Held
    /// at the per-tick target once `ramp_samples_remaining` hits 0.
    pub current_left_vol:               f32,
    pub current_right_vol:              f32,
    pub left_ramp_step:                 f32,
    pub right_ramp_step:                f32,
    pub ramp_samples_remaining:         u32,
    /// True ⇒ a cut was requested but the mixer is ramping the voice's
    /// gain down to 0 first. Once `current_left_vol`/`current_right_vol`
    /// reach 0 the mixer sets `on = false`. Use the `cut_voice` helper
    /// in `song/backend.rs` to set this — it also keeps the channel-side
    /// `voice_idx` invariant intact.
    pub pending_cut:                    bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VoiceCutReason {
    /// `LoopType::NoLoop` sample reached its end and the mixer set the
    /// volume to 0 + cleared `on`. Reference: output.rs end-of-sample
    /// branch (the `OnePastTheEnd` arm).
    SampleEnd,
    /// Engine-side note action set `voice.on = false` (NoteAction::Cut,
    /// IT NNA cut, S3M Sxx note cut, key-off without sustaining envelope).
    /// Set from the trigger logic; the mixer just observes it.
    NoteCut,
    /// Voice volume reached 0 via fadeout / envelope and the mixer
    /// elected to drop the voice instead of mixing 0-gain samples.
    Faded,
}

/// IT-compatible 2-pole resonant low-pass filter (direct-form IIR biquad).
///
/// `y[n] = a0 * x[n] + b0 * y[n-1] + b1 * y[n-2]`
///
/// Coefficients are computed in [[update_filter]] using the same recurrence
/// libopenmpt uses for `kITFilterBehaviour` (`Snd_flt.cpp:SetupChannelFilter`).
/// The earlier engine used a Chamberlin state-variable filter, which under
/// the same `cutoff`/`resonance` byte produced very different (and at low
/// cutoffs unstable) responses — `1_channel_moog.it` was the smoking gun:
/// resonance feedback built up monotonically over 25 s instead of tracking
/// the song's filter sweeps.
#[derive(Clone, Copy, Debug)]
pub struct ResonantFilter {
    /// Input (feed-forward) coefficient.
    pub a0: f32,
    /// y[n-1] feedback coefficient.
    pub b0: f32,
    /// y[n-2] feedback coefficient.
    pub b1: f32,
    /// `[y[n-1], y[n-2]]`.
    pub history: [f32; 2],
}

impl ResonantFilter {
    pub(crate) fn new() -> Self {
        Self {
            a0: 1.0,
            b0: 0.0,
            b1: 0.0,
            history: [0.0; 2],
        }
    }
}

impl Voice {
    pub fn new() -> Self {
        Self {
            instrument: 0,
            sample: 0,
            frequency: 0.0,
            du: 0.0,
            volume: Volume::new(),
            sample_position: 0.0,
            loop_started: false,
            sustained: false,
            spline_data: SplineData::new(),
            volume_envelope_state: EnvelopeState::new(),
            panning_envelope_state: EnvelopeState::new(),
            pitch_envelope_state: EnvelopeState::new(),
            vibrato_envelope_state: VibratoEnvelopeState::new(),
            vibrato_state: VibratoState::new(),
            tremolo_state: TremoloState::new(),
            tremolo_shift: 0.0,
            frequency_shift: 0.0,
            panning: Panning::new(),
            instrument_global_volume: 64,
            sample_global_volume: 64,
            filter_cutoff: 127,
            filter_resonance: 0,
            filter_state: ResonantFilter::new(),
            on: false,
            surround: false,
            channel_idx: 0,
            last_played_note: 0,
            last_render_tick: 0,
            cut_reason: None,
            current_left_vol: 0.0,
            current_right_vol: 0.0,
            left_ramp_step: 0.0,
            right_ramp_step: 0.0,
            ramp_samples_remaining: 0,
            pending_cut: false,
        }
    }

    /// Reset ramp state for a fresh note trigger. Zero current_*_vol so
    /// the new voice fades in from silence over the ~5 ms ramp window.
    ///
    /// The OLD voice that previously occupied this slot (if any) has
    /// already been moved to a background slot by
    /// `spawn_background_cut_inline` (cut path) and continues to mix
    /// there with its own pending_cut ramp, so we don't lose its audio
    /// by zeroing here.
    pub fn reset_ramp_for_new_note(&mut self) {
        self.current_left_vol = 0.0;
        self.current_right_vol = 0.0;
        self.left_ramp_step = 0.0;
        self.right_ramp_step = 0.0;
        self.ramp_samples_remaining = 0;
        self.pending_cut = false;
    }


    pub fn key_off(&mut self, instruments: &Instruments, song_type: SongType) -> bool {
        let instrument = &instruments[self.instrument];
        self.sustained = false;
        self.volume_envelope_state.key_off(&instrument.volume_envelope);
        self.panning_envelope_state.key_off(&instrument.panning_envelope);
        self.pitch_envelope_state.key_off(&instrument.pitch_envelope);
        self.volume.fadeout_speed = instrument.volume_fadeout as i32;

        // S3M ^^^ silences immediately (no envelopes/fadeout in S3M).
        if song_type == SongType::S3M {
            self.on = false;
            return true;
        }

        // FT2: with no volume envelope, key-off zeros the voice volume.
        // The envelope-on case lets sustain end and fadeout decay it.
        if !instrument.volume_envelope.on {
            self.volume.retrig(0);
        }
        return true;
    }

    pub(crate) fn set_frequency(&mut self, frequency: f32, rate: f32) {
        self.frequency = frequency;
        if rate > 0.0 {
            self.du = self.frequency / rate;
        } else {
            self.du = 0.0;
        }
    }

    pub(crate) fn update_envelopes(&mut self, instruments: &Instruments, rate: f32) {
        let instrument = &instruments[self.instrument];
        
        let envelope_volume = self.volume_envelope_state.handle(&instrument.volume_envelope, self.sustained, 64, false);
        let envelope_panning = self.panning_envelope_state.handle(&instrument.panning_envelope, self.sustained, 32, true);

        self.panning.update_envelope_panning(envelope_panning);
        self.volume.envelope_vol = envelope_volume as i32;

        let mut final_cutoff = self.filter_cutoff as i32;
        if instrument.is_filter_envelope {
            let envelope_filter = self.pitch_envelope_state.handle(&instrument.pitch_envelope, self.sustained, 32, false);
            // libopenmpt Sndmix.cpp:2349 — IT filter envelope is
            // *multiplicative* on cutoff:
            //   computedCutoff = cutoff * (envModifier + 256) / 256
            // env value range 0..16384 (= 0..64 * 256, center 8192 from
            // the OMT +32 offset). Map to envMod -256..+256.
            let env_mod = (envelope_filter as i32 - 8192) / 32;
            final_cutoff = (self.filter_cutoff as i32 * (env_mod + 256)) / 256;
        } else {
            let envelope_pitch = self.pitch_envelope_state.handle(&instrument.pitch_envelope, self.sustained, 32, false);
            // Pitch envelopes are centered around 32.
            // Each unit is approx 1/16 of a semitone.
            // We'll approximate this by adding to frequency_shift.
            // 1 semitone is approx 6% frequency change.
            // So 1/16 semitone is approx 0.375% change.
            let pitch_shift_units = (envelope_pitch as i32 - 32 * 256) / 256;
            self.frequency_shift = self.frequency * (pitch_shift_units as f32 * 0.00375);
        }

        // Auto-vibrato — FT2 parity. FT2 applies it as a period shift
        // before period→freq lookup (pmp_main.c:1314-1321):
        //   autoVibVal = VIB_SINE_TAB[pos] << 2          (range -256..+256)
        //   period_delta = (autoVibVal * autoVibAmp) >> 16
        //   final_period = real_period + period_delta
        //
        // Our `handle()` returns `(sin * amp) / 64`; FT2's equivalent
        // period delta is `(sin<<2) * amp / 65536 = sin * amp / 16384`,
        // i.e. our return divided by 256.
        //
        // For Amiga-period mode (flags bit 0 = 0, as in SHOOTING.XM)
        // the frequency derivative wrt period is:
        //   d_freq / d_period = -freq / period
        // so a small period_delta of N units shifts freq by
        // approximately `-freq * N / period` Hz.
        //
        // Prior implementation used a semitone-fraction formula
        // (`-(auto_vibrato/16384) * 0.05946 * freq`) which was about
        // 5.5× too weak vs FT2's actual pitch-modulation depth.
        // SHOOTING.XM channels using the auto-vibrato instrument 12
        // were rendering ~13% louder than ft2play because the under-
        // applied auto-vib left the sample read-rate too steady; with
        // the correct modulation depth, frequency sweeps cross more of
        // the sample's dynamic content and the average power lines up
        // with ft2play's per-channel RMS.
        let auto_vibrato = self.vibrato_envelope_state.handle(&instrument.vibrato_envelope, self.sustained);
        if auto_vibrato != 0 && self.frequency > 0.0 {
            // For Amiga-period mode (default XM): freq = K / period where
            // K = 8363 * 1712 = 14317456. So d_freq/d_period = -freq²/K,
            // and a period delta of N units yields a frequency shift of
            // approximately -freq² * N / K Hz.
            //
            // (Linear-period mode XMs would need a different formula —
            // d_freq/d_period scales differently — but the bulk of XM
            // corpus uses Amiga periods, and SHOOTING.XM is Amiga.
            // Linear-mode auto-vibrato calibration is a separate TODO.)
            const AMIGA_K: f32 = 8363.0 * 1712.0;
            let period_delta_ft2 = (auto_vibrato as f32) / 256.0;
            self.frequency_shift += -self.frequency * self.frequency * period_delta_ft2 / AMIGA_K;
        }

        self.update_filter(rate, final_cutoff.clamp(0, 127) as u8);
    }

    pub(crate) fn update_filter(&mut self, rate: f32, cutoff: u8) {
        // Bypass only when cutoff is at-or-above the fully-open ceiling
        // AND resonance is zero. Filter-envelope instruments with high
        // resonance (e.g. moog inst 3 res=120) skip this branch even
        // when envModifier pushes cutoff past 127, and IT modules without
        // any filter envelope or resonance (monochrome_crisis-style) hit
        // this branch and bypass entirely.
        if cutoff >= 127 && self.filter_resonance == 0 {
            self.filter_state.a0 = 1.0;
            self.filter_state.b0 = 0.0;
            self.filter_state.b1 = 0.0;
            return;
        }

        // libopenmpt CutOffToFrequency (Snd_flt.cpp:40), IT branch.
        // envModifier folded into `cutoff` upstream in update_envelopes.
        let computed_cutoff = cutoff as f32 * 256.0;
        let mut fc = 110.0 * 2.0f32.powf(0.25 + computed_cutoff / (24.0 * 512.0));
        fc = fc.clamp(120.0, 20_000.0).min(rate * 0.5);
        let fc_omega = fc * 2.0 * std::f32::consts::PI;

        // 2 * damping factor — IT resonance 0..127 in 0.1875 dB steps.
        let dmpfac = 10.0f32.powf(-(self.filter_resonance as f32) * (24.0 / 128.0) / 20.0);

        // IT-compatible coefficient recurrence (Snd_flt.cpp:127-143).
        let r = rate / fc_omega;
        let d = dmpfac * r + dmpfac - 1.0;
        let e = r * r;
        let denom = 1.0 + d + e;
        self.filter_state.a0 = 1.0 / denom;
        self.filter_state.b0 = (d + e + e) / denom;
        self.filter_state.b1 = -e / denom;
    }


    pub(crate) fn compute_base_volume(&self) -> f32 {
        let mut vol = (self.volume.fadeout_vol as f32 / 65536.0) * 
        (self.volume.envelope_vol as f32 / 16384.0) * 
        (self.volume.get_volume() as f32 / 64.0) + self.tremolo_shift;
        
        if vol < 0.0 { vol = 0.0; }
        if vol > 1.0 { vol = 1.0; }
        
        vol * (self.sample_global_volume as f32 / 64.0)
    }

    pub(crate) fn set_output_volume(&mut self, volume: f32) {
        self.volume.output_volume = volume;
    }

    pub(crate) fn update_fadeout(&mut self) {
        if !self.sustained {
            // FT2/OMT: fadeout subtracts speed*2 per tick.
            let step = self.volume.fadeout_speed.saturating_mul(2);
            if self.volume.fadeout_vol - step < 0 {
                self.volume.fadeout_vol = 0;
            } else {
                self.volume.fadeout_vol -= step;
            }
        }
    }

    /// Re-arm volume + envelopes for an instrument number that landed on a
    /// porta-to-note row. The note itself doesn't retrigger (no
    /// sample_position reset, no fadeout reset of `sustained`), but the
    /// instrument number causes the voice to re-read its sample's default
    /// volume and rewind its envelope phases — matching ST3/FT2/IT.
    ///
    /// FT2 `triggerInstrument` also resets `fadeoutVol` to full; without it,
    /// a key-off that landed on the previous voice keeps the fadeout running
    /// (we'd play silent through a porta-revived instrument).
    pub(crate) fn porta_retrig_for_instrument(&mut self, instruments: &Instruments) {
        let instrument = &instruments[self.instrument];
        let sample_vol = if self.sample < instrument.samples.len() {
            instrument.samples[self.sample].volume as i32
        } else {
            64
        };
        self.volume.retrig(sample_vol);
        self.volume.fadeout_vol = 65536;
        self.volume_envelope_state.reset(0, &instrument.volume_envelope);
        self.panning_envelope_state.reset(0, &instrument.panning_envelope);
        self.pitch_envelope_state.reset(0, &instrument.pitch_envelope);
        self.vibrato_envelope_state.reset(&instrument.vibrato_envelope);
    }

    pub(crate) fn trigger_note(&mut self, instruments: &Instruments, reset_envelopes: bool, vibrato_retrig: bool, tremolo_retrig: bool) {
        self.sample_position = 4.0;
        self.loop_started = false;
        self.sustained = true;
        self.on = true;
        // Auto-vibrato / pitch-envelope accumulator. Without this,
        // a freshly-triggered note inherits the previous note's last
        // tick's frequency_shift on its own tick 0 (e.g. SHOOTING.XM
        // ch0 trace showed ord=3 row=39 tick=2 freq_shift=-36 →
        // ord=4 row=0 tick=0 freq_shift=189 before update_envelopes
        // recomputes — leaked state that wouldn't matter if the mixer
        // didn't ever sample the channel between trigger and the next
        // update_envelopes pass).
        self.frequency_shift = 0.0;

        self.volume.fadeout_vol = 65536;
        self.volume.fadeout_speed = instruments[self.instrument].volume_fadeout as i32;

        let instrument = &instruments[self.instrument];
        self.instrument_global_volume = instrument.global_volume;
        self.filter_cutoff = instrument.initial_filter_cutoff;
        self.filter_resonance = instrument.initial_filter_resonance;
        if self.sample < instrument.samples.len() {
            self.sample_global_volume = instrument.samples[self.sample].global_volume;
        }

        if reset_envelopes {
            // IT envCarry (envelope flag bit 3): preserve the envelope's
            // current position across a fresh trigger of the same
            // instrument. Required for 1_channel_moog.it inst 1's filter
            // sweep, which only reaches its peak after ~140 ticks and
            // would never get there if every retrigger reset it to 0.
            // Caller is expected to have copied the prior voice's
            // envelope state into `self` for the carry-enabled
            // envelopes before calling trigger_note (see the IT NNA
            // alloc path in `song/backend/it.rs`).
            if !instrument.volume_envelope.carry {
                self.volume_envelope_state.reset(0, &instrument.volume_envelope);
            }
            if !instrument.panning_envelope.carry {
                self.panning_envelope_state.reset(0, &instrument.panning_envelope);
            }
            if !instrument.pitch_envelope.carry {
                self.pitch_envelope_state.reset(0, &instrument.pitch_envelope);
            }
            self.vibrato_envelope_state.reset(&instrument.vibrato_envelope);
        }

        if vibrato_retrig { self.vibrato_state.pos = 0; }
        if tremolo_retrig { self.tremolo_state.pos = 0; }
    }
}

/// Slots for the per-channel effect-memory table.
///
/// Each tracker effect that has "param=0 means recall last param" semantics
/// stores its raw byte parameter in `ChannelState::effect_memory[slot]`.
/// Two effects can share a slot (e.g. S3M shares E and F porta memory) and
/// pre-multiplication is done at the use-site, not at storage time.

// --- Data-driven slide spec ---------------------------------------------
//
// XM/MOD/S3M/IT slides share a uniform shape:
//   1) optional memory recall (param==0 means "use the last non-zero")
//   2) decode the byte into an (i32 step, gating tick)
//   3) apply the step to a clamped numeric field
// The decoders below capture the per-format quirk (XM-A vs IT-D vs panning P),
// and `apply_slide` is the one engine that walks the spec. Add a new slide
// effect by adding one `SlideSpec` row in the dispatcher — no new handler.

/// Numeric field a slide writes to, with its built-in 0..=N clamp.
#[derive(Copy, Clone, Debug)]
pub enum SlideField {
    /// `voice.volume.volume`, 0..=64. XM A / IT D / vol-col vol-slide.
    VoiceVolume,
    /// `voice.panning.panning`, 0..=255. XM P (vol-col) / IT P / S3M P.
    VoicePanning,
    /// `channel.channel_volume`, 0..=64. IT/S3M M command.
    ChannelVolume,
}

/// Tick on which a decoded step actually fires.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SlideTiming {
    /// Fine slides (S3M/IT DFy/DxF, XM `E[AB]`).
    FirstTickOnly,
    /// Running slides (XM A after tick 0, S3M/IT D without fast_volume_slides).
    AfterFirstTick,
    /// Running slides under ST3 `fast_volume_slides` (tick-0 included).
    EveryTick,
    /// No-op for this row.
    Never,
}

/// How to turn the raw `u8` param into a signed step + timing.
#[derive(Copy, Clone, Debug)]
pub enum SlideDecode {
    /// XM `Axy`: hi nibble = up, low = down, hi wins on conflict.
    /// Running slide → fires after the first tick.
    XmPacked,
    /// IT/S3M `Dxy` / `Pxy`: F-nibble triggers a fine sub-variant.
    /// `scale` is the byte-magnitude multiplier (1 for vol slides on a
    /// 0..=64 field, 4 for panning slides on a 0..=255 field).
    /// `fast_volume_slides` (ST3 v3.00 quirk) promotes running slides to
    /// every-tick.
    ItPacked { scale: u8, fast: bool },
    /// Direct signed step. Caller has already decoded direction/magnitude
    /// (XM vol-col 0x60..0x9f / IT vol-col `Cx`..`Fx`). The raw byte passed
    /// to `apply_slide` is the *bit-pattern* of the i8 step, so the
    /// memory recall round-trips cleanly. `fine` selects the timing slot.
    SignedDirect { fine: bool },
    /// IT/S3M `Mxy` channel volume slide — like `ItPacked` but with the
    /// F-nibble polarity swapped (MFy is fine *up*, MxF is fine *down*),
    /// and parameters where one nibble is F and the other is 0 (`M_F0`,
    /// `M_0F`) are no-ops rather than running slides.
    MStyle { scale: u8 },
}

/// One slide effect, materialized as a row in the dispatcher.
#[derive(Copy, Clone, Debug)]
pub struct SlideSpec {
    pub field: SlideField,
    pub decode: SlideDecode,
    /// `None` skips the recall-or-set; otherwise the raw param is funneled
    /// through this slot before decoding.
    pub memory: Option<EffectMemorySlot>,
}

// --- Data-driven porta spec ---------------------------------------------
//
// Porta-up/down has the same recall → decode → apply → clamp shape as the
// slide family, but on `channel.note.period` instead of a volume/panning
// field. The extra wrinkle is IT/S3M's magic-nibble encoding (xF0+ extra
// fine, xE0–xEF fine, else normal) and S3M's shared memory between E and F.

/// Sign of the period delta (Up shrinks period = raises pitch).
#[derive(Copy, Clone, Debug)]
pub enum PortaDir { Up, Down }

/// Memory layout per format.
#[derive(Copy, Clone, Debug)]
pub enum PortaMemory {
    /// XM/MOD 1xx/2xx + E1x/E2x: dedicated slot per effect.
    Separate(EffectMemorySlot),
    /// S3M E/F: a write hits both slots; reads come from `primary`.
    Shared { primary: EffectMemorySlot, secondary: EffectMemorySlot },
}

/// How to turn the recalled byte into a magnitude + timing.
#[derive(Copy, Clone, Debug)]
pub enum PortaDecode {
    /// XM/MOD 1xx/2xx: `param << 2`, runs after the first tick.
    XmNormal,
    /// XM/MOD E1x/E2x: `param << 2`, runs on the first tick only.
    XmFine,
    /// IT/S3M E/F magic-nibble encoding:
    ///   xF0..xFF → extra-fine (param & 0xF, ×1, first tick)
    ///   xE0..xEF → fine       (param & 0xF, ×4, first tick)
    ///   else     → normal     (param, ×4, after first tick)
    ItMagicNibble,
}

/// Period clamp range (FT2 vs ST3/IT vs PT/MOD use different floors).
/// MOD is the strictest: real Amiga Paula has period range [113, 856];
/// internal storage is period * 4 so the clamp is [452, 3424]. Without
/// this, fast porta-up on MOD files runs the period below 113 (i.e.
/// real-Amiga-impossible high pitches), and high-note positive-finetune
/// table lookups land below 113 (AmigaLimitsFinetune.mod is the canonical
/// reproducer: B-3 ft+4 hits internal period 440 = real 110 in our
/// FT2-derived table; pt2-clone and OMT both clamp it at 113).
#[derive(Copy, Clone, Debug)]
pub enum PortaClamp { Xm, It, Mod }

#[derive(Copy, Clone, Debug)]
pub struct PortaSpec {
    pub direction: PortaDir,
    pub memory: PortaMemory,
    pub decode: PortaDecode,
    pub clamp: PortaClamp,
}

// --- Data-driven LFO memory layout --------------------------------------
//
// Vibrato/tremolo come in two memory flavours:
//   * Split (FT2 / IT / S3M): speed and depth live in independent slots,
//     and a 0 nibble *preserves* that slot's previous value. Crucial for
//     XM vol-col 0xa0/0xb0 (set-speed / set-depth on their own row) and
//     for IT vol-col 203-212 (set-depth).
//   * Packed (XM/MOD tremolo): both nibbles share one slot via
//     `recall_or_set` (param=0 recalls).
//
// The two-fn pattern below captures that quirk in data; the call site
// chooses the layout per format and gets `(cur_speed, cur_depth)` back.

#[derive(Copy, Clone, Debug)]
pub enum LfoMemory {
    Split { speed: EffectMemorySlot, depth: EffectMemorySlot },
    Packed(EffectMemorySlot),
}

/// Decode an effect-byte into a signed step magnitude + tick gating, per
/// the format-specific rules captured by `SlideDecode`. Pure function — no
/// channel state involved, easy to unit-test.
pub fn decode_slide(param: u8, decode: SlideDecode) -> (i32, SlideTiming) {
    match decode {
        SlideDecode::XmPacked => {
            // XM A: hi-nibble up, low-nibble down, hi wins. Running slide.
            let hi = (param >> 4) as i32;
            let lo = (param & 0x0F) as i32;
            let step = if hi != 0 { hi } else if lo != 0 { -lo } else { 0 };
            (step, SlideTiming::AfterFirstTick)
        }
        SlideDecode::ItPacked { scale, fast } => {
            // IT/S3M D (vol) and P (pan) share this decode.
            //   DFy / PFy → fine down by y (first tick)
            //   DxF / PxF → fine up by x (first tick)
            //   Dx0 / Px0 → running up by x
            //   D0y / P0y → running down by y
            //   Dxy (both nonzero, neither F) → ST3/IT pick hi
            // `fast_volume_slides` promotes running slides to every-tick.
            let hi = (param >> 4) as i32;
            let lo = (param & 0x0F) as i32;
            let s = scale as i32;
            let running = if fast { SlideTiming::EveryTick } else { SlideTiming::AfterFirstTick };
            if hi == 0x0F && lo != 0 {
                (-lo * s, SlideTiming::FirstTickOnly)
            } else if lo == 0x0F && hi != 0 {
                (hi * s, SlideTiming::FirstTickOnly)
            } else if hi != 0 && lo == 0 {
                (hi * s, running)
            } else if hi == 0 && lo != 0 {
                (-lo * s, running)
            } else if hi != 0 {
                (hi * s, running)
            } else {
                (0, SlideTiming::Never)
            }
        }
        SlideDecode::SignedDirect { fine } => {
            // Param byte is the bit-pattern of an i8 step (caller cast).
            let step = (param as i8) as i32;
            let timing = if fine { SlideTiming::FirstTickOnly } else { SlideTiming::AfterFirstTick };
            (step, timing)
        }
        SlideDecode::MStyle { scale } => {
            // IT/S3M Mxy:
            //   MFy (hi=F, lo!=0,!=F) → fine UP   by lo (first tick)
            //   MxF (hi!=0,!=F, lo=F) → fine DOWN by hi (first tick)
            //   Mx0 (hi!=0,!=F, lo=0) → run  UP   by hi (after first tick)
            //   M0y (hi=0, lo!=0,!=F) → run  DOWN by lo (after first tick)
            //   anything else (incl. MF0 / M0F / MFF) is a no-op.
            let hi = (param >> 4) as i32;
            let lo = (param & 0x0F) as i32;
            let s = scale as i32;
            if hi == 0xF && lo != 0 && lo != 0xF {
                (lo * s, SlideTiming::FirstTickOnly)
            } else if lo == 0xF && hi != 0 && hi != 0xF {
                (-hi * s, SlideTiming::FirstTickOnly)
            } else if hi != 0 && hi != 0xF && lo == 0 {
                (hi * s, SlideTiming::AfterFirstTick)
            } else if hi == 0 && lo != 0 && lo != 0xF {
                (-lo * s, SlideTiming::AfterFirstTick)
            } else {
                (0, SlideTiming::Never)
            }
        }
    }
}

/// Decode a porta byte into an unsigned magnitude + tick gate. Sign is
/// applied later via `PortaDir`; this function is direction-agnostic and
/// pure, so the magic-nibble decode for IT/S3M can be unit-tested in
/// isolation.
pub fn decode_porta(param: u8, decode: PortaDecode) -> (u16, SlideTiming) {
    match decode {
        PortaDecode::XmNormal => ((param as u16) << 2, SlideTiming::AfterFirstTick),
        PortaDecode::XmFine   => ((param as u16) << 2, SlideTiming::FirstTickOnly),
        PortaDecode::ItMagicNibble => {
            if param >= 0xF0 {
                // Extra-fine: low nibble, ×1, first-tick only.
                ((param & 0x0F) as u16, SlideTiming::FirstTickOnly)
            } else if param >= 0xE0 {
                // Fine: low nibble, ×4, first-tick only.
                (((param & 0x0F) as u16) << 2, SlideTiming::FirstTickOnly)
            } else {
                // Normal: whole byte, ×4, after first tick.
                ((param as u16) << 2, SlideTiming::AfterFirstTick)
            }
        }
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(usize)]
pub enum EffectMemorySlot {
    PortaUp = 0,             // XM/MOD E1xx, also fine via Exy. Raw byte; use-site does *4.
    PortaDown,               // XM/MOD E2xx
    FinePortaUp,             // XM E1x
    FinePortaDown,           // XM E2x
    VolSlide,                // XM/MOD A
    FineVolSlideUp,          // XM EA
    FineVolSlideDown,        // XM EB
    SampleOffset,            // 9xx
    ItPortaUp,               // IT E / S3M E (S3M shares with PortaDown via shared write)
    ItPortaDown,             // IT F / S3M F
    ItVolColVolSlide,        // IT vol-col Cx/Dx (running)
    ItVolColFineVolSlide,    // IT vol-col Ax/Bx (fine)
    ItVolColPorta,           // IT vol-col Gx
    ItVolSlide,              // IT D
    VibratoParam,            // XM/MOD H/4 packed nibbles
    TremoloParam,            // XM/MOD R/7 packed nibbles
    VibratoSpeed,            // IT/S3M H speed (split storage)
    VibratoDepth,            // IT/S3M H depth (split storage)
    TremoloSpeed,            // IT/S3M R speed
    TremoloDepth,            // IT/S3M R depth
    PanningSlide,            // XM P / IT P / S3M P
    Arpeggio,                // XM/IT/S3M arpeggio packed nibbles
    Count,                   // sentinel for table size
}

const EFFECT_MEMORY_LEN: usize = EffectMemorySlot::Count as usize;

/// FT2's arpeggio table (mirrors `arpTab` in ft2play/tables.c). The
/// first 16 entries are the clean `0,1,2` cycle Protracker uses; the
/// rest are the actual byte values that FT2.08/.09 read from out-of-
/// bounds memory when the song's speed exceeded 16. Reproducing the
/// overflow is what lets Arpeggio.xm (speed=19) match ft2play tick-
/// for-tick — without it the cycle becomes plain `tick % 3`.
const FT2_ARP_TAB: [u8; 256] = [
    0, 1, 2, 0, 1, 2, 0, 1, 2, 0, 1, 2, 0, 1, 2, 0,
    0x00, 0x18, 0x31, 0x4A, 0x61, 0x78, 0x8D, 0xA1, 0xB4, 0xC5, 0xD4, 0xE0, 0xEB, 0xF4, 0xFA, 0xFD,
    0xFF, 0xFD, 0xFA, 0xF4, 0xEB, 0xE0, 0xD4, 0xC5, 0xB4, 0xA1, 0x8D, 0x78, 0x61, 0x4A, 0x31, 0x18,
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x03, 0x00, 0x02, 0x00, 0x04, 0x00, 0x00,
    0x00, 0x05, 0x06, 0x00, 0x00, 0x07, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x04, 0x05, 0x00, 0x00,
    0x0B, 0x00, 0x0A, 0x02, 0x01, 0x03, 0x04, 0x07, 0x00, 0x05, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x79, 0x02, 0x00, 0x00, 0x8F, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46, 0x4F, 0x52, 0x4D, 0x49, 0x4C, 0x42, 0x4D, 0x42, 0x4D,
];

pub struct ChannelState {
    pub voice_idx:                      Option<usize>, // Which voice is currently "active" for this channel
    pub last_instrument:                usize,
    pub last_sample:                    usize,
    pub note:                           Note,
    pub frequency:                      f32,
    pub volume:                         Volume,
    pub panning:                        Panning,
    pub on:                             bool,
    pub channel_volume:                 u8,
    pub porta_to_note:                  PortaToNoteState,
    pub force_off:                      bool,
    /// IT/S3M S91 surround flag. Persists across notes on this channel
    /// — that's why it lives here rather than on `Voice`. Each render
    /// tick the output stage negates the right-channel gain when this
    /// is set so the signal cancels in mono and stereo speakers hear
    /// a wide / phase-spread image.
    pub surround:                       bool,
    pub effect_memory:                  [u8; EFFECT_MEMORY_LEN],
    pub(crate) glissando:                      bool,
    pub(crate) tremor:                         u8,
    pub(crate) tremor_count:                   u32,
    /// Set by the Tremor (S3M Ixy) handler each tick. When true, the
    /// volume post-loop forces voice output to 0 for this channel's voice.
    /// Reset to false at the top of every channel iteration so it only
    /// persists for the tick on which Tremor was dispatched.
    pub(crate) tremor_silenced:                bool,
    /// True when the row currently being processed carries a vibrato or
    /// vibrato-combo effect (or vol-col vibrato for XM/MOD). Set in
    /// `init_channel_iter` from `pattern.has_vibrato`. Gates the
    /// vibrato wave's frequency contribution inside
    /// `update_frequency_voice` — without it the persisted
    /// `vibrato_state.pos` keeps biasing pitch on every tick of every
    /// subsequent row, even ones that don't ask for vibrato. Mirrors
    /// master's `if pattern.has_vibrato()` gate.
    pub(crate) vibrato_active_this_row:        bool,
    pub(crate) period_shift:                   i16,
    pub(crate) last_played_note:               u8,
    pub(crate) vibrato_waveform:               u8,
    pub(crate) tremolo_waveform:               u8,
    pub(crate) vibrato_retrig:                 bool,
    pub(crate) tremolo_retrig:                 bool,
    pub(crate) last_samples:                   [f32; 512], // Standardized to 512 for UI
    pub(crate) last_samples_pos:               usize,
    pub(crate) loop_row:                       u8,
    pub(crate) loop_count:                     u8,
    /// Multiplier applied in `update_frequency_voice` to compensate for
    /// per-format reference-clock differences between our period→Hz table
    /// (8363*1712/period, the FT2/XM Amiga clock) and the format's
    /// authentic clock. MOD uses Protracker PAL clock 3546895*4/period
    /// which is ~0.91% lower (16 cents flatter), so MOD channels need
    /// `frequency_scale = 14187580/14317456 ≈ 0.99093`. XM/S3M/IT use
    /// the same clock as our table → scale = 1.0. Set once at Song
    /// construction from `VoiceMixFormula::freq_scale`.
    pub(crate) frequency_scale:                f32,
    /// MOD/PT: a Cxx (SetVolume) effect on a row with no active voice
    /// stashes the param here. The next note trigger on this channel
    /// consumes it instead of resetting to sample default. Mirrors PT
    /// behaviour where the volume command "sticks" until the next
    /// trigger. OpenMPT test case PTInstrVolume.mod relies on this:
    /// row 0 sets vol=0 on an empty channel, row 1's note triggers
    /// and must keep vol=0 instead of reverting to sample default.
    pub(crate) pending_set_volume:             Option<u8>,
    /// IT: set when the channel's last note action was Cut / Off /
    /// Fade. Used to suppress bare-instrument-row retriggers per
    /// OpenMPT's "OffsetWithInstr.it" rule: an offset effect can be
    /// triggered if only an instrument number and no note is next to
    /// it, *if the last triggered note was not a note cut, note off
    /// or note fade*. Cleared on every fresh note trigger.
    pub(crate) note_stopped:                   bool,
    /// XM/MOD: a Tremor (Txx) effect that fires sets this; subsequent
    /// rows continue the alternating on/off cycle from `tremor_count`
    /// even without a fresh Txx on the row, until interrupted by a
    /// new volume-affecting command (per OpenMPT TremorRecover.xm:
    /// "volume commands should be able to override this effect").
    pub(crate) tremor_active:                  bool,
    /// XM: mirror of the live voice's `volume.volume` (FT2's `ch->realVol`).
    /// Survives voice death so that a note-without-instrument trigger
    /// at row N can pick up the running channel vol even when the
    /// previous voice has fully faded out and been returned to the pool
    /// (`channel.voice_idx = None`). Updated at end-of-tick by the XM
    /// backend after vol-col / effect dispatch.
    pub(crate) real_vol:                       u8,
    /// XM: set by vol-col Fx (porta-to-note in volume column) on a row,
    /// checked by eff-col 3xx/5xx so its "set portaSpeed" update is
    /// suppressed. FT2's `getNewNote` returns early after vol-col Fx,
    /// so eff-col 3 never overwrites `portaSpeed`. Without this gate,
    /// row `vol=F1, eff=31` ends up with portaSpeed=4 (from eff-col 31)
    /// instead of 64 (from vol-col F1). Reset at the start of every
    /// channel iter in `init_channel_iter`.
    pub(crate) xm_vol_col_porta_this_row:      bool,
}

impl ChannelState {
    pub fn new() -> Self {
        Self {
            voice_idx: None,
            last_instrument: 0,
            last_sample: 0,
            note: Note::new(),
            frequency: 0.0,
            volume: Volume::new(),
            panning: Panning::new(),
            on: false,
            channel_volume: 64,
            porta_to_note: PortaToNoteState::new(),
            force_off: false,
            surround: false,
            effect_memory: [0u8; EFFECT_MEMORY_LEN],
            glissando: false,
            tremor: 0,
            tremor_count: 0,
            tremor_silenced: false,
            vibrato_active_this_row: false,
            period_shift: 0,
            last_played_note: 0,
            pending_set_volume: None,
            note_stopped: false,
            tremor_active: false,
            vibrato_waveform:       0,
            tremolo_waveform: 0,
            vibrato_retrig:         true,
            tremolo_retrig:         true,
            last_samples: [0.0; 512],
            last_samples_pos: 0,
            loop_row: 0,
            loop_count: 0,
            frequency_scale: 1.0,
            real_vol: 64,
            xm_vol_col_porta_this_row: false,
        }
    }

    /// Read an effect-memory slot.
    #[inline]
    pub fn mem(&self, slot: EffectMemorySlot) -> u8 {
        self.effect_memory[slot as usize]
    }

    /// Write an effect-memory slot.
    #[inline]
    pub fn set_mem(&mut self, slot: EffectMemorySlot, v: u8) {
        self.effect_memory[slot as usize] = v;
    }

    /// Standard "param=0 means recall, otherwise update" pattern.
    #[inline]
    pub fn recall_or_set(&mut self, slot: EffectMemorySlot, param: u8) -> u8 {
        if param == 0 {
            self.mem(slot)
        } else {
            self.set_mem(slot, param);
            param
        }
    }

    /// Two-slot variant: writes update both slots (e.g. S3M's E and F porta
    /// share memory). Recall reads from `primary`.
    #[inline]
    pub fn recall_or_set_shared(
        &mut self,
        primary: EffectMemorySlot,
        secondary: EffectMemorySlot,
        param: u8,
    ) -> u8 {
        if param == 0 {
            self.mem(primary)
        } else {
            self.set_mem(primary, param);
            self.set_mem(secondary, param);
            param
        }
    }


    pub(crate) fn update_frequency_voice(&mut self, voice: &mut Voice, rate: f32, semitone: bool, frequency_tables: &AudioTables) {
        let vib_shift = if self.vibrato_active_this_row {
            voice.vibrato_state.get_frequency_shift(WaveControl::from(self.vibrato_waveform))
        } else { 0 };
        self.frequency = self.note.frequency(self.period_shift, vib_shift, semitone, frequency_tables) * self.frequency_scale + voice.frequency_shift;
        voice.set_frequency(self.frequency, rate);
        // Diagnostic dump — `OUR_DUMP_CH=<idx>` env var pairs with
        // tools/openmpt_instrumentation.patch for tick-by-tick diffs.
        let ours_dump_ch_env = std::env::var("OUR_DUMP_CH").ok();
        let ours_dump_ch: i32 = ours_dump_ch_env.as_deref().and_then(|s| s.parse().ok()).unwrap_or(-1);
        if ours_dump_ch >= 0 && voice.channel_idx as i32 == ours_dump_ch && !semitone && voice.on {
            let ord = DUMP_CTX_ORD.load(Ordering::Relaxed);
            let row = DUMP_CTX_ROW.load(Ordering::Relaxed);
            let tick = DUMP_CTX_TICK.load(Ordering::Relaxed);
            eprintln!("[OUR] ord={} row={} tick={} ch={} note={} period={} vib_shift={} period_shift={} freq={} vibpos={} vibdep={} vibspd={} vibwf={} fine={} vraw={} pos={:.0} aVibPos={} aVibAmp={} aVibSwp={} envVPos={} envPPos={} envVAmp={} fadeOut={} outVol={:.4} freq_shift={:.1}",
                ord, row, tick,
                voice.channel_idx, voice.last_played_note,
                self.note.period as i32 + self.period_shift as i32 + vib_shift,
                vib_shift, self.period_shift,
                self.frequency as i32,
                voice.vibrato_state.pos,
                voice.vibrato_state.depth,
                voice.vibrato_state.speed,
                self.vibrato_waveform,
                voice.vibrato_state.fine,
                voice.volume.volume,
                voice.sample_position,
                voice.vibrato_envelope_state.vibrato_pos,
                voice.vibrato_envelope_state.vibrato_amp,
                voice.vibrato_envelope_state.vibrato_sweep,
                voice.volume_envelope_state.frame,
                voice.panning_envelope_state.frame,
                voice.volume.envelope_vol,
                voice.volume.fadeout_vol,
                voice.volume.output_volume,
                voice.frequency_shift);
        }
    }

    /// Update the LFO memory slots for an `Hxy` / `Rxy` style parameter
    /// and return the *current* (speed, depth) after the recall. See
    /// `LfoMemory` for the per-format quirk this captures.
    pub(crate) fn recall_lfo(&mut self, speed_in: u8, depth_in: u8, mem: LfoMemory) -> (u8, u8) {
        match mem {
            LfoMemory::Split { speed, depth } => {
                if speed_in != 0 { self.set_mem(speed, speed_in); }
                if depth_in != 0 { self.set_mem(depth, depth_in); }
                (self.mem(speed), self.mem(depth))
            }
            LfoMemory::Packed(slot) => {
                let packed = self.recall_or_set(slot, (speed_in << 4) | depth_in);
                (packed >> 4, packed & 0x0F)
            }
        }
    }

    pub(crate) fn vibrato(&mut self, voice: Option<&mut Voice>, first_tick: bool, speed: u8, depth: u8, old_effects: bool, rate: f32, tables: &AudioTables, song_type: SongType) {
        self.vibrato_inner(voice, first_tick, speed, depth, old_effects, false, rate, tables, song_type);
    }

    /// Post-increment vibrato wave pos (FT2 semantic). Called by the
    /// backend after end-of-tick update_frequency_voice.
    pub(crate) fn advance_vibrato_pos(&mut self, voice: &mut Voice) {
        voice.vibrato_state.next_tick();
    }

    /// S3M U / IT u command: like vibrato but the depth multiplier is 1
    /// instead of 4 (1/4 the swing for the same depth byte). Memory is
    /// shared with regular vibrato per S3M spec.
    pub(crate) fn fine_vibrato(&mut self, voice: Option<&mut Voice>, first_tick: bool, speed: u8, depth: u8, old_effects: bool, rate: f32, tables: &AudioTables, song_type: SongType) {
        self.vibrato_inner(voice, first_tick, speed, depth, old_effects, true, rate, tables, song_type);
    }

    fn vibrato_inner(&mut self, voice: Option<&mut Voice>, first_tick: bool, speed: u8, depth: u8, old_effects: bool, fine: bool, rate: f32, tables: &AudioTables, song_type: SongType) {
        let _ = song_type;
        let _ = old_effects;
        // Vibrato uses split memory for *all* formats (FT2 quirk): the
        // XM vol-col `Axy` / `Bxy` rows set speed and depth on separate
        // rows by leaving one nibble at 0, and packed memory would clobber
        // the silent nibble.
        let (cur_speed, cur_depth) = self.recall_lfo(speed, depth, LfoMemory::Split {
            speed: EffectMemorySlot::VibratoSpeed,
            depth: EffectMemorySlot::VibratoDepth,
        });

        if let Some(v) = voice {
            if first_tick {
                // Speed ×4 for sub-tick wave resolution.
                v.vibrato_state.speed = (cur_speed as u16 * 4) as i8;
                v.vibrato_state.depth = cur_depth as i16;
                v.vibrato_state.fine = fine;
            }
            // Apply shift with current pos; the backend post-increments
            // pos after end-of-tick update_frequency_voice.
            self.update_frequency_voice(v, rate, true, tables);
        }
    }

    pub(crate) fn tremolo(&mut self, voice: Option<&mut Voice>, first_tick: bool, speed: u8, depth: u8, song_type: SongType) {
        // XM/MOD `7xy` packs both nibbles into one memory slot. IT/S3M
        // `Rxy` keeps them independent (matches their independent-nibble
        // memory model for vibrato/panbrello).
        let mem = if matches!(song_type, SongType::XM | SongType::MOD) {
            LfoMemory::Packed(EffectMemorySlot::TremoloParam)
        } else {
            LfoMemory::Split {
                speed: EffectMemorySlot::TremoloSpeed,
                depth: EffectMemorySlot::TremoloDepth,
            }
        };
        let (cur_speed, cur_depth) = self.recall_lfo(speed, depth, mem);

        if let Some(v) = voice {
            if first_tick {
                v.tremolo_state.speed = (cur_speed as u16 * 4) as i8;
                v.tremolo_state.depth = cur_depth as i16;
            } else {
                v.tremolo_state.next_tick();
            }
            let shift = v.tremolo_state.get_volume_shift(WaveControl::from(self.tremolo_waveform));
            v.tremolo_shift = shift as f32 / 64.0;
        }
    }

    pub(crate) fn arpeggio(&mut self, tick: u32, x: u8, y: u8, has_memory: bool) {
        if has_memory {
            let packed = self.recall_or_set(EffectMemorySlot::Arpeggio, (x << 4) | y);
            let actual_x = packed >> 4;
            let actual_y = packed & 0x0F;
            match tick % 3 {
                0 => { self.period_shift = 0; }
                1 => { self.period_shift = -(actual_x as i16 * 64); }
                2 => { self.period_shift = -(actual_y as i16 * 64); }
                _ => {}
            }
        } else {
            match tick % 3 {
                0 => { self.period_shift = 0; }
                1 => { self.period_shift = -(x as i16 * 64); }
                2 => { self.period_shift = -(y as i16 * 64); }
                _ => {}
            }
        }
    }

    /// FT2's quirky arpeggio: instead of a clean `tick % 3` cycle,
    /// indexes a 256-byte table by the falling timer (`speed - tick`).
    /// At small speeds (<=15) the first 15 entries `0,1,2,0,1,2,...`
    /// reproduce the PT-style cycle, but the table's "buggy overflow"
    /// region (added in FT2.08/.09 from out-of-bounds reads of the
    /// playback binary itself) is what plays back at speeds > 15.
    /// Without this, Arpeggio.xm (which sets speed=19 via F13) shows a
    /// completely different per-tick pattern from ft2play. See
    /// `/tmp/ft2play/tables.c::arpTab` and `pmp_main.c::arp`.
    ///
    /// arpTab entry semantics: 0 → base, 1 → high nibble (param>>4),
    /// anything else → low nibble (param & 0x0F).
    pub(crate) fn arpeggio_xm(&mut self, tick: u32, speed: u32, first_row_tick: bool, x: u8, y: u8) {
        // FT2 only skips arp at the *readNewNote* tick — the literal
        // first tick of a fresh row, where getNewNote sets state but
        // doEffects doesn't run. On a pattern-delay repeat's first
        // tick, tickZero is still true but readNewNote is false (FT2:
        // `readNewNote = tickZero && pattDelTime2 == 0`); doEffects
        // fires with `song.timer == tempo`, so arpTab[tempo] picks the
        // first value. Gating on `first_row_tick` mirrors this. Test:
        // Arpeggio.xm row 3 (EEx pattern-delay) at speed=17 — without
        // this our period reset to base at the repeat's tick=0 while
        // ft2play applied the low-nibble offset.
        if first_row_tick {
            self.period_shift = 0;
            return;
        }
        // FT2 indexes arpTab with the FALLING timer value. Internal tick
        // (0..speed-1 ascending) maps to timer (speed..1 descending) as
        // timer = speed - internal_tick. With our `tick` numbering
        // matching FT2's `tempo - timer`, arpTab[timer] = arpTab[speed - tick].
        let idx = (speed.saturating_sub(tick) & 0xFF) as usize;
        let entry = FT2_ARP_TAB[idx];
        let note = match entry {
            0 => { self.period_shift = 0; return; }
            1 => x,
            _ => y,
        };
        // Linear-period semitone shift: -note * 64. Amiga-mode XM uses
        // the same convention here since FT2's relocateTon for linear
        // resolves to a constant -64 per semitone, and Amiga XM is rare
        // enough that the same approximation is acceptable until a
        // real Amiga module surfaces an audible deviation.
        self.period_shift = -((note as i16) * 64);
    }

    pub(crate) fn porta_up(&mut self, song_type: SongType, first_tick: bool, amount: u8) {
        let spec = match song_type {
            SongType::IT => PortaSpec {
                direction: PortaDir::Up,
                memory: PortaMemory::Separate(EffectMemorySlot::ItPortaUp),
                decode: PortaDecode::ItMagicNibble,
                clamp: PortaClamp::It,
            },
            SongType::S3M => PortaSpec {
                direction: PortaDir::Up,
                memory: PortaMemory::Shared {
                    primary: EffectMemorySlot::ItPortaUp,
                    secondary: EffectMemorySlot::ItPortaDown,
                },
                decode: PortaDecode::ItMagicNibble,
                clamp: PortaClamp::It,
            },
            _ => PortaSpec {
                direction: PortaDir::Up,
                memory: PortaMemory::Separate(EffectMemorySlot::PortaUp),
                decode: PortaDecode::XmNormal,
                clamp: if song_type == SongType::MOD { PortaClamp::Mod } else { PortaClamp::Xm },
            },
        };
        self.apply_porta(first_tick, amount, spec);
    }

    /// XM Txx / S3M Ixy / IT Ixy: alternate the channel between audible
    /// (x ticks) and silent (y ticks). Persistent state lives in
    /// `tremor` (param memory) and `tremor_count` (running on/off
    /// counter encoded as sign bit + remaining-ticks-in-current-phase).
    /// Per-tick state goes into `tremor_silenced`, which the volume
    /// post-loop honors.
    ///
    /// OpenMPT test Tremor.xm: "The tremor counter is not updated on
    /// the first tick, and the counter is only ever reset after a phase
    /// switch (from on to off or vice versa)." On a fresh Txx row (tick
    /// 0) we just stash the parameter and mark the on-phase active —
    /// the actual countdown advances only from tick 1 onwards.
    pub(crate) fn tremor(&mut self, tick: u32, param: u8) {
        if tick == 0 {
            if param != 0 {
                self.tremor = param;
            }
            // Activate the audible phase on row entry; let tick 1+
            // start the countdown without changing the counter.
            self.tremor_count |= 0x80;
            self.tremor_silenced = false;
            self.tremor_active = true;
            return;
        }
        self.tremor_advance();
    }

    /// Advance the tremor counter by one tick (the actual per-tick
    /// state machine). Pulled out so the per-tick post-loop can keep
    /// the cycle running on rows that have no explicit Txx command —
    /// XM tremor persists until a new volume-affecting command
    /// interrupts it. Skipped when `tremor_active` is false.
    pub(crate) fn tremor_advance(&mut self) {
        if !self.tremor_active { return; }
        let mut tremor_sign = self.tremor_count & 0x80;
        let mut tremor_data = (self.tremor_count & 0x7F) as i8;

        tremor_data -= 1;
        if tremor_data < 0 {
            if tremor_sign == 0x80 {
                // Switching from "on" to "off" phase, load the y nibble.
                tremor_sign = 0x00;
                tremor_data = (self.tremor & 0xf) as i8;
            } else {
                // Switching from "off" to "on" phase, load the x nibble.
                tremor_sign = 0x80;
                tremor_data = (self.tremor >> 4) as i8;
            }
        }

        self.tremor_count = tremor_sign | tremor_data as u32;
        // sign == 0x80 means "in the on phase, audible". Silenced is the
        // negation; volume post-loop zeros output when this is true.
        self.tremor_silenced = tremor_sign != 0x80;
    }

    pub(crate) fn porta_down(&mut self, song_type: SongType, first_tick: bool, amount: u8) {
        let spec = match song_type {
            SongType::IT => PortaSpec {
                direction: PortaDir::Down,
                memory: PortaMemory::Separate(EffectMemorySlot::ItPortaDown),
                decode: PortaDecode::ItMagicNibble,
                clamp: PortaClamp::It,
            },
            SongType::S3M => PortaSpec {
                direction: PortaDir::Down,
                memory: PortaMemory::Shared {
                    primary: EffectMemorySlot::ItPortaDown,
                    secondary: EffectMemorySlot::ItPortaUp,
                },
                decode: PortaDecode::ItMagicNibble,
                clamp: PortaClamp::It,
            },
            _ => PortaSpec {
                direction: PortaDir::Down,
                memory: PortaMemory::Separate(EffectMemorySlot::PortaDown),
                decode: PortaDecode::XmNormal,
                clamp: if song_type == SongType::MOD { PortaClamp::Mod } else { PortaClamp::Xm },
            },
        };
        self.apply_porta(first_tick, amount, spec);
    }

    pub(crate) fn fine_porta_up(&mut self, song_type: SongType, first_tick: bool, amount: u8) {
        // XM/MOD E1x. (IT/S3M handle fine via the magic nibble inside porta_up.)
        let clamp = if matches!(song_type, SongType::S3M | SongType::IT) {
            PortaClamp::It
        } else if song_type == SongType::MOD {
            PortaClamp::Mod
        } else {
            PortaClamp::Xm
        };
        self.apply_porta(first_tick, amount, PortaSpec {
            direction: PortaDir::Up,
            memory: PortaMemory::Separate(EffectMemorySlot::FinePortaUp),
            decode: PortaDecode::XmFine,
            clamp,
        });
    }

    pub(crate) fn set_volume(&mut self, voice: Option<&mut Voice>, first_tick: bool, vol: u8) {
        if first_tick {
            if let Some(v) = voice {
                v.volume.set_volume(vol as i32);
            } else {
                // No live voice: stash for the next trigger to consume.
                // PT/MOD test case PTInstrVolume.mod: Cxx on an empty
                // channel must persist until the next note row.
                self.pending_set_volume = Some(vol);
            }
        }
    }

    pub(crate) fn volume_slide(&mut self, voice: Option<&mut Voice>, first_tick: bool, amount: i8) {
        if !first_tick {
            if let Some(v) = voice {
                v.volume.set_volume(v.volume.volume as i32 + amount as i32);
            }
        }
    }

    /// Data-driven slide engine. Walks the spec to:
    ///   * recall-or-set the memory slot (if any),
    ///   * decode the byte into a signed step + timing,
    ///   * apply the step (clamped) on the right tick.
    /// One method drives every XM/MOD/S3M/IT volume / panning slide — see
    /// the `SlideSpec` doc above for the per-format quirks captured.
    pub(crate) fn apply_slide(
        &mut self,
        voice: Option<&mut Voice>,
        first_tick: bool,
        raw_param: u8,
        spec: SlideSpec,
    ) {
        let param = match spec.memory {
            Some(slot) => self.recall_or_set(slot, raw_param),
            None => raw_param,
        };
        let (step, timing) = decode_slide(param, spec.decode);
        let fires = match timing {
            SlideTiming::FirstTickOnly => first_tick,
            SlideTiming::AfterFirstTick => !first_tick,
            SlideTiming::EveryTick => true,
            SlideTiming::Never => false,
        };
        if !fires || step == 0 { return; }
        match spec.field {
            SlideField::VoiceVolume => {
                if let Some(v) = voice {
                    v.volume.set_volume(v.volume.volume as i32 + step);
                }
            }
            SlideField::VoicePanning => {
                if let Some(v) = voice {
                    v.panning.set_panning(v.panning.panning as i32 + step);
                }
            }
            SlideField::ChannelVolume => {
                let new = (self.channel_volume as i32 + step).clamp(0, 64);
                self.channel_volume = new as u8;
            }
        }
    }

    /// Data-driven porta engine. Walks a `PortaSpec` to:
    ///   * recall-or-set memory (separate or S3M-shared),
    ///   * decode the byte into (magnitude, timing),
    ///   * gate on the right tick,
    ///   * apply the signed delta to `note.period`, clamped per format.
    pub(crate) fn apply_porta(&mut self, first_tick: bool, raw_param: u8, spec: PortaSpec) {
        let param = match spec.memory {
            PortaMemory::Separate(slot) => self.recall_or_set(slot, raw_param),
            PortaMemory::Shared { primary, secondary } => {
                self.recall_or_set_shared(primary, secondary, raw_param)
            }
        };
        let (mag, timing) = decode_porta(param, spec.decode);
        let fires = match timing {
            SlideTiming::FirstTickOnly => first_tick,
            SlideTiming::AfterFirstTick => !first_tick,
            SlideTiming::EveryTick => true,
            SlideTiming::Never => false,
        };
        if !fires || mag == 0 { return; }
        let signed = match spec.direction {
            PortaDir::Up   => -(mag as i32),
            PortaDir::Down =>  mag as i32,
        };
        let (min_p, max_p) = match spec.clamp {
            PortaClamp::Xm  => (1i32, 31999i32),
            PortaClamp::It  => (113i32, 27392i32),
            PortaClamp::Mod => (452i32, 3424i32),  // PT period 113..856, ×4
        };
        let new = (self.note.period as i32 + signed).clamp(min_p, max_p);
        self.note.period = new as u16;
        // IT linear pitch: the active pitch lives in `note.linear_hz`,
        // not in `period`. Mirror OMT's `DoFreqSlide` linear branch:
        // every fine step is 1/(64×12) of an octave so the per-tick
        // multiplier is 2^(-signed / 768). `signed` is already in
        // OMT's "amount" units (decode_porta ×4'd the pattern param
        // for normal/fine, ×1 for extra-fine), so the same formula
        // covers all three cases. Without this, IT's E/F slides were
        // a no-op in linear-pitch mode (orbiter.it ch3 was sample-
        // locked at the trigger freq, looping audibly).
        if self.note.linear_hz != 0.0 {
            let factor = (-signed as f32 / 768.0).exp2();
            self.note.linear_hz *= factor;
            // Match OMT's saturation: freq ≥ ~1 Hz, no negative.
            if self.note.linear_hz < 1.0 { self.note.linear_hz = 1.0; }
        }
    }

    pub(crate) fn fine_volume_slide(&mut self, voice: Option<&mut Voice>, first_tick: bool, amount: i8) {
        if first_tick {
            if let Some(v) = voice {
                v.volume.set_volume(v.volume.volume as i32 + amount as i32);
            }
        }
    }

    pub(crate) fn fine_porta_down(&mut self, song_type: SongType, first_tick: bool, amount: u8) {
        // XM/MOD E2x.
        let clamp = if matches!(song_type, SongType::S3M | SongType::IT) {
            PortaClamp::It
        } else if song_type == SongType::MOD {
            PortaClamp::Mod
        } else {
            PortaClamp::Xm
        };
        self.apply_porta(first_tick, amount, PortaSpec {
            direction: PortaDir::Down,
            memory: PortaMemory::Separate(EffectMemorySlot::FinePortaDown),
            decode: PortaDecode::XmFine,
            clamp,
        });
    }

    pub(crate) fn porta_to_note(&mut self, _song_type: SongType, voice: Option<&mut Voice>, first_tick: bool, speed: u8, _compatible_g: bool, rate: f32, tables: &AudioTables) {
        if first_tick {
            if speed != 0 {
                self.porta_to_note.speed = (speed as u16) * 4;
            }
        } else {
            self.porta_to_note.next_tick(&mut self.note);
            if self.glissando {
                self.note.snap_to_semitone(tables);
            }
        }
        if let Some(v) = voice {
            self.update_frequency_voice(v, rate, self.glissando, tables);
        }
    }

    pub(crate) fn it_vol_col_volume_slide(&mut self, voice: Option<&mut Voice>, first_tick: bool, amount: i8) {
        // IT vol-col Cx/Dx (running). Per OMT VolColMemory.it:
        // "Volume column commands a, b, c and d (volume slide) share
        // one effect memory, but it should not be shared with Dxy in
        // the effect column." All four vol-col slide variants use the
        // same memory slot (ItVolColVolSlide), distinct from the
        // effect-column ItVolSlide. Sign is encoded by the caller;
        // the memory slot round-trips the i8 bit-pattern through u8
        // so param==0 cleanly recalls the last signed step.
        self.apply_slide(voice, first_tick, amount as u8, SlideSpec {
            field: SlideField::VoiceVolume,
            decode: SlideDecode::SignedDirect { fine: false },
            memory: Some(EffectMemorySlot::ItVolColVolSlide),
        });
    }

    pub(crate) fn it_vol_col_fine_volume_slide(&mut self, voice: Option<&mut Voice>, first_tick: bool, amount: i8) {
        // IT vol-col Ax/Bx (fine). Shares memory with Cx/Dx — see the
        // VolColMemory.it note above.
        self.apply_slide(voice, first_tick, amount as u8, SlideSpec {
            field: SlideField::VoiceVolume,
            decode: SlideDecode::SignedDirect { fine: true },
            memory: Some(EffectMemorySlot::ItVolColVolSlide),
        });
    }

    pub(crate) fn it_vol_col_porta_to_note(&mut self, voice: Option<&mut Voice>, first_tick: bool, speed: u8, compatible_g: bool, rate: f32, tables: &AudioTables) {
        let speed = self.recall_or_set(EffectMemorySlot::ItVolColPorta, speed);
        self.porta_to_note(SongType::IT, voice, first_tick, speed, compatible_g, rate, tables);
    }

    pub(crate) fn it_volume_slide(&mut self, voice: Option<&mut Voice>, first_tick: bool, param: u8, fast_volume_slides: bool) {
        // IT D / S3M D. ItPacked handles the DFy/DxF fine vs Dx0/D0y running
        // split inline; `fast_volume_slides` (ST3 v3.00) promotes running
        // slides to every-tick.
        self.apply_slide(voice, first_tick, param, SlideSpec {
            field: SlideField::VoiceVolume,
            decode: SlideDecode::ItPacked { scale: 1, fast: fast_volume_slides },
            memory: Some(EffectMemorySlot::ItVolSlide),
        });
    }

    pub(crate) fn it_retrig(&mut self, voice: Option<&mut Voice>, instruments: &Instruments, tick: u32, param: u8) {
        let x = param >> 4;
        let y = param & 0x0F;
        if y == 0 { return; }
        if tick % (y as u32) == 0 {
            if tick > 0 {
                self.retrig(voice, instruments, tick, y, x);
            }
        }
    }


    pub(crate) fn retrig(&mut self, voice: Option<&mut Voice>, instruments: &Instruments, tick: u32, amount: u8, volume_change: u8) {
        if amount == 0 { return; }
        if tick % (amount as u32) == 0 {
            if let Some(v) = voice {
                v.trigger_note(instruments, true, self.vibrato_retrig, self.tremolo_retrig);
                match volume_change {
                    1 => { v.volume.set_volume(v.volume.get_volume() as i32 - 1); }
                    2 => { v.volume.set_volume(v.volume.get_volume() as i32 - 2); }
                    3 => { v.volume.set_volume(v.volume.get_volume() as i32 - 4); }
                    4 => { v.volume.set_volume(v.volume.get_volume() as i32 - 8); }
                    5 => { v.volume.set_volume(v.volume.get_volume() as i32 - 16); }
                    6 => { v.volume.set_volume((v.volume.get_volume() as f32 * 2.0 / 3.0) as i32); }
                    7 => { v.volume.set_volume((v.volume.get_volume() as f32 * 0.5) as i32); }
                    9 => { v.volume.set_volume(v.volume.get_volume() as i32 + 1); }
                    10 => { v.volume.set_volume(v.volume.get_volume() as i32 + 2); }
                    11 => { v.volume.set_volume(v.volume.get_volume() as i32 + 4); }
                    12 => { v.volume.set_volume(v.volume.get_volume() as i32 + 8); }
                    13 => { v.volume.set_volume(v.volume.get_volume() as i32 + 16); }
                    14 => { v.volume.set_volume((v.volume.get_volume() as f32 * 1.5) as i32); }
                    15 => { v.volume.set_volume((v.volume.get_volume() as f32 * 2.0) as i32); }
                    _ => {}
                }
            }
        }
    }

    pub(crate) fn volume_slide_main(&mut self, voice: Option<&mut Voice>, first_tick: bool, param: u8) {
        // XM A: param != 0 stores memory; param == 0 recalls.
        self.apply_slide(voice, first_tick, param, SlideSpec {
            field: SlideField::VoiceVolume,
            decode: SlideDecode::XmPacked,
            memory: Some(EffectMemorySlot::VolSlide),
        });
    }

    pub(crate) fn channel_volume_slide(&mut self, first_tick: bool, param: u8) {
        // IT/S3M Mxy: like D but with swapped F-nibble polarity, no memory,
        // writes to channel.channel_volume.
        self.apply_slide(None, first_tick, param, SlideSpec {
            field: SlideField::ChannelVolume,
            decode: SlideDecode::MStyle { scale: 1 },
            memory: None,
        });
    }

    pub(crate) fn panning_slide(&mut self, voice: Option<&mut Voice>, first_tick: bool, param: u8, song_type: SongType) {
        // XM/MOD P: no fine variant (XmPacked, scale=1 on a 0..=255 field).
        // IT/S3M P: PFx / PxF fine, else running, byte magnitude ×4.
        let decode = if matches!(song_type, SongType::XM | SongType::MOD) {
            SlideDecode::XmPacked
        } else {
            SlideDecode::ItPacked { scale: 4, fast: false }
        };
        self.apply_slide(voice, first_tick, param, SlideSpec {
            field: SlideField::VoicePanning,
            decode,
            memory: Some(EffectMemorySlot::PanningSlide),
        });
    }
}

#[cfg(test)]
mod slide_decode_tests {
    use super::{decode_slide, SlideDecode, SlideTiming};

    #[test]
    fn xm_packed_hi_nibble_wins_after_first_tick() {
        // Axy: hi up, lo down, hi wins on conflict, never on tick 0.
        assert_eq!(decode_slide(0x30, SlideDecode::XmPacked), (3, SlideTiming::AfterFirstTick));
        assert_eq!(decode_slide(0x05, SlideDecode::XmPacked), (-5, SlideTiming::AfterFirstTick));
        assert_eq!(decode_slide(0x23, SlideDecode::XmPacked), (2, SlideTiming::AfterFirstTick));
        assert_eq!(decode_slide(0x00, SlideDecode::XmPacked), (0, SlideTiming::AfterFirstTick));
    }

    #[test]
    fn it_packed_fine_via_f_nibble() {
        let d = SlideDecode::ItPacked { scale: 1, fast: false };
        assert_eq!(decode_slide(0xF3, d), (-3, SlideTiming::FirstTickOnly));   // DFy
        assert_eq!(decode_slide(0x3F, d), (3, SlideTiming::FirstTickOnly));    // DxF
        assert_eq!(decode_slide(0x30, d), (3, SlideTiming::AfterFirstTick));   // Dx0
        assert_eq!(decode_slide(0x05, d), (-5, SlideTiming::AfterFirstTick));  // D0y
    }

    #[test]
    fn it_packed_fast_promotes_running_to_every_tick() {
        let d = SlideDecode::ItPacked { scale: 1, fast: true };
        assert_eq!(decode_slide(0x30, d), (3, SlideTiming::EveryTick));
        assert_eq!(decode_slide(0xF3, d), (-3, SlideTiming::FirstTickOnly)); // fine still tick-0
    }

    #[test]
    fn it_packed_panning_scale4() {
        let d = SlideDecode::ItPacked { scale: 4, fast: false };
        // Pxy = right/left in 0..15 units, scale ×4 so panning shifts 0..60.
        assert_eq!(decode_slide(0x30, d), (12, SlideTiming::AfterFirstTick));
        assert_eq!(decode_slide(0x05, d), (-20, SlideTiming::AfterFirstTick));
        assert_eq!(decode_slide(0xF3, d), (-12, SlideTiming::FirstTickOnly));
    }

    #[test]
    fn signed_direct_passes_param_through_as_i8() {
        let normal = SlideDecode::SignedDirect { fine: false };
        assert_eq!(decode_slide(0x07, normal), (7, SlideTiming::AfterFirstTick));
        assert_eq!(decode_slide(0xFD, normal), (-3, SlideTiming::AfterFirstTick));
        let fine = SlideDecode::SignedDirect { fine: true };
        assert_eq!(decode_slide(0x05, fine), (5, SlideTiming::FirstTickOnly));
        assert_eq!(decode_slide(0xFE, fine), (-2, SlideTiming::FirstTickOnly));
    }

    #[test]
    fn m_style_swaps_f_nibble_polarity_vs_d() {
        let m = SlideDecode::MStyle { scale: 1 };
        // MFy (hi=F, lo!=0,!=F) → fine UP by lo
        assert_eq!(decode_slide(0xF3, m), (3, SlideTiming::FirstTickOnly));
        // MxF (lo=F, hi!=0,!=F) → fine DOWN by hi
        assert_eq!(decode_slide(0x3F, m), (-3, SlideTiming::FirstTickOnly));
        // Mx0 → running up by hi
        assert_eq!(decode_slide(0x50, m), (5, SlideTiming::AfterFirstTick));
        // M0y → running down by lo
        assert_eq!(decode_slide(0x05, m), (-5, SlideTiming::AfterFirstTick));
        // Edge no-ops: M_F0, M_0F, M_FF
        assert_eq!(decode_slide(0xF0, m), (0, SlideTiming::Never));
        assert_eq!(decode_slide(0x0F, m), (0, SlideTiming::Never));
        assert_eq!(decode_slide(0xFF, m), (0, SlideTiming::Never));
        assert_eq!(decode_slide(0x00, m), (0, SlideTiming::Never));
    }
}

#[cfg(test)]
mod porta_decode_tests {
    use super::{decode_porta, PortaDecode, SlideTiming};

    #[test]
    fn xm_normal_scales_by_4_runs_after_first_tick() {
        assert_eq!(decode_porta(0x05, PortaDecode::XmNormal), (20, SlideTiming::AfterFirstTick));
        assert_eq!(decode_porta(0xFF, PortaDecode::XmNormal), (0x3FC, SlideTiming::AfterFirstTick));
        assert_eq!(decode_porta(0x00, PortaDecode::XmNormal), (0, SlideTiming::AfterFirstTick));
    }

    #[test]
    fn xm_fine_scales_by_4_runs_on_first_tick() {
        assert_eq!(decode_porta(0x03, PortaDecode::XmFine), (12, SlideTiming::FirstTickOnly));
    }

    #[test]
    fn it_magic_nibble_extra_fine_above_f0() {
        // xF0..xFF → extra-fine, low nibble ×1, first tick.
        assert_eq!(decode_porta(0xF0, PortaDecode::ItMagicNibble), (0, SlideTiming::FirstTickOnly));
        assert_eq!(decode_porta(0xF7, PortaDecode::ItMagicNibble), (7, SlideTiming::FirstTickOnly));
        assert_eq!(decode_porta(0xFF, PortaDecode::ItMagicNibble), (15, SlideTiming::FirstTickOnly));
    }

    #[test]
    fn it_magic_nibble_fine_e0_to_ef() {
        // xE0..xEF → fine, low nibble ×4, first tick.
        assert_eq!(decode_porta(0xE0, PortaDecode::ItMagicNibble), (0, SlideTiming::FirstTickOnly));
        assert_eq!(decode_porta(0xE3, PortaDecode::ItMagicNibble), (12, SlideTiming::FirstTickOnly));
        assert_eq!(decode_porta(0xEF, PortaDecode::ItMagicNibble), (60, SlideTiming::FirstTickOnly));
    }

    #[test]
    fn it_magic_nibble_normal_below_e0() {
        // 0x00..0xDF → normal, whole byte ×4, after first tick.
        assert_eq!(decode_porta(0x01, PortaDecode::ItMagicNibble), (4, SlideTiming::AfterFirstTick));
        assert_eq!(decode_porta(0x80, PortaDecode::ItMagicNibble), (512, SlideTiming::AfterFirstTick));
        assert_eq!(decode_porta(0xDF, PortaDecode::ItMagicNibble), (0xDF * 4, SlideTiming::AfterFirstTick));
    }
}
