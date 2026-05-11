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

#[derive(Clone, Copy, Debug)]
pub struct ResonantFilter {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub history: [f32; 2],
}

impl ResonantFilter {
    pub(crate) fn new() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
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
        }
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
            // IT filter envelopes are centered around 32 (range 0..64)
            // They add/subtract from the current cutoff.
            final_cutoff += (envelope_filter as i32 - 32 * 256) / 256;
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

        // Auto-vibrato. Negated because VIB_SINE_TAB is -sin; the FT2
        // effective wave is +sin (rising at pos=64).
        let auto_vibrato = self.vibrato_envelope_state.handle(&instrument.vibrato_envelope, self.sustained);
        if auto_vibrato != 0 {
            let vib_shift = (-(auto_vibrato as f32) / 16384.0) * 0.05946;
            self.frequency_shift += self.frequency * vib_shift;
        }

        self.update_filter(rate, final_cutoff.clamp(0, 127) as u8);
    }

    pub(crate) fn update_filter(&mut self, rate: f32, cutoff: u8) {
        if cutoff >= 127 && self.filter_resonance == 0 {
            self.filter_state.a = 1.0;
            self.filter_state.b = 0.0;
            self.filter_state.c = 0.0;
            return;
        }

        // IT cutoff scale mapping: 0..127 -> roughly 100Hz .. 15kHz
        // Using a logarithmic scale
        let cutoff_freq = 100.0 * (150.0f32).powf(cutoff as f32 / 127.0);
        
        // SVF p coefficient: 2 * sin(pi * f / rate)
        let p = 2.0 * (std::f32::consts::PI * cutoff_freq / rate).sin();
        
        // SVF r coefficient (damping): 2.0 * 10^(-resonance_db / 20)
        // IT resonance 0..127 maps to roughly 0..24dB
        let resonance_db = (self.filter_resonance as f32 / 127.0) * 24.0;
        let r = 2.0 * 10.0f32.powf(-resonance_db / 20.0);

        self.filter_state.a = p.min(1.99); // Stability limit
        self.filter_state.b = r;
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
    pub(crate) fn porta_retrig_for_instrument(&mut self, instruments: &Instruments) {
        let instrument = &instruments[self.instrument];
        let sample_vol = if self.sample < instrument.samples.len() {
            instrument.samples[self.sample].volume as i32
        } else {
            64
        };
        self.volume.retrig(sample_vol);
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
            self.volume_envelope_state.reset(0, &instrument.volume_envelope);
            self.panning_envelope_state.reset(0, &instrument.panning_envelope);
            self.pitch_envelope_state.reset(0, &instrument.pitch_envelope);
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
            effect_memory: [0u8; EFFECT_MEMORY_LEN],
            glissando: false,
            tremor: 0,
            tremor_count: 0,
            tremor_silenced: false,
            vibrato_active_this_row: false,
            period_shift: 0,
            last_played_note: 0,
            vibrato_waveform:       0,
            tremolo_waveform: 0,
            vibrato_retrig:         true,
            tremolo_retrig:         true,
            last_samples: [0.0; 512],
            last_samples_pos: 0,
            loop_row: 0,
            loop_count: 0,
            frequency_scale: 1.0,
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
            eprintln!("[OUR] ord={} row={} tick={} ch={} note={} period={} vib_shift={} period_shift={} freq={} vibpos={} vibdep={} vibspd={} vibwf={} fine={} vraw={} pos={:.0}",
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
                voice.sample_position);
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
        // FT2: speed and depth are independent memory lanes — only the
        // non-zero nibble of param updates its respective field.
        let _ = song_type;
        let _ = old_effects;
        if speed != 0 { self.set_mem(EffectMemorySlot::VibratoSpeed, speed); }
        if depth != 0 { self.set_mem(EffectMemorySlot::VibratoDepth, depth); }
        let cur_speed = self.mem(EffectMemorySlot::VibratoSpeed);
        let cur_depth = self.mem(EffectMemorySlot::VibratoDepth);

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
        let (cur_speed, cur_depth) = if song_type == SongType::XM || song_type == SongType::MOD {
            let packed = self.recall_or_set(EffectMemorySlot::TremoloParam, (speed << 4) | depth);
            (packed >> 4, packed & 0x0F)
        } else {
            if speed != 0 { self.set_mem(EffectMemorySlot::TremoloSpeed, speed); }
            if depth != 0 { self.set_mem(EffectMemorySlot::TremoloDepth, depth); }
            (self.mem(EffectMemorySlot::TremoloSpeed), self.mem(EffectMemorySlot::TremoloDepth))
        };

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

    pub(crate) fn porta_up(&mut self, song_type: SongType, first_tick: bool, amount: u8) {
        let actual_amount = match song_type {
            SongType::IT  => self.recall_or_set(EffectMemorySlot::ItPortaUp, amount),
            SongType::S3M => self.recall_or_set_shared(
                EffectMemorySlot::ItPortaUp, EffectMemorySlot::ItPortaDown, amount,
            ),
            _ => amount,
        };

        if song_type == SongType::IT || (song_type == SongType::S3M && actual_amount >= 0xE0) {
            if actual_amount >= 0xF0 { // Extra Fine
                if first_tick {
                    let val = (actual_amount & 0x0F) as u16;
                    self.note.period = self.note.period.saturating_sub(val);
                }
            } else if actual_amount >= 0xE0 { // Fine
                if first_tick {
                    let val = ((actual_amount & 0x0F) as u16) << 2;
                    self.note.period = self.note.period.saturating_sub(val);
                }
            } else { // Normal
                if !first_tick {
                    let val = (actual_amount as u16) << 2;
                    self.note.period = self.note.period.saturating_sub(val);
                }
            }
        } else {
            // XM/MOD: store the raw byte; scale by 4 at the apply site.
            if first_tick {
                if actual_amount != 0 {
                    self.set_mem(EffectMemorySlot::PortaUp, actual_amount);
                }
            } else {
                let val = (self.mem(EffectMemorySlot::PortaUp) as u16) * 4;
                self.note.period = (std::num::Wrapping(self.note.period) - std::num::Wrapping(val)).0;
            }
        }

        let min_period = if song_type == SongType::S3M || song_type == SongType::IT { 113 } else { 1 };
        if (self.note.period as i16) < min_period {
            self.note.period = min_period as u16;
        }
    }

    /// S3M Ixy / similar: alternate the channel between audible (x ticks)
    /// and silent (y ticks). Persistent state lives in `tremor` (param
    /// memory) and `tremor_count` (running on/off counter encoded as
    /// sign bit + remaining-ticks-in-current-phase). Per-tick state goes
    /// into `tremor_silenced`, which the volume post-loop honors.
    pub(crate) fn tremor(&mut self, tick: u32, param: u8) {
        if tick == 0 && param != 0 {
            self.tremor = param;
        }

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
        let actual_amount = match song_type {
            SongType::IT  => self.recall_or_set(EffectMemorySlot::ItPortaDown, amount),
            SongType::S3M => self.recall_or_set_shared(
                EffectMemorySlot::ItPortaDown, EffectMemorySlot::ItPortaUp, amount,
            ),
            _ => amount,
        };

        if song_type == SongType::IT || (song_type == SongType::S3M && actual_amount >= 0xE0) {
            if actual_amount >= 0xF0 { // Extra Fine
                if first_tick {
                    let val = (actual_amount & 0x0F) as u16;
                    self.note.period = self.note.period.saturating_add(val);
                }
            } else if actual_amount >= 0xE0 { // Fine
                if first_tick {
                    let val = ((actual_amount & 0x0F) as u16) << 2;
                    self.note.period = self.note.period.saturating_add(val);
                }
            } else { // Normal
                if !first_tick {
                    let val = (actual_amount as u16) << 2;
                    self.note.period = self.note.period.saturating_add(val);
                }
            }
        } else {
            if first_tick {
                if actual_amount != 0 {
                    self.set_mem(EffectMemorySlot::PortaDown, actual_amount);
                }
            } else {
                let val = (self.mem(EffectMemorySlot::PortaDown) as u16) * 4;
                self.note.period += val;
            }
        }

        let max_period = if song_type == SongType::S3M || song_type == SongType::IT { 27392 } else { 31999 };
        if self.note.period > max_period {
            self.note.period = max_period;
        }
    }

    pub(crate) fn fine_porta_up(&mut self, song_type: SongType, first_tick: bool, amount: u8) {
        if first_tick {
            if amount != 0 {
                self.set_mem(EffectMemorySlot::FinePortaUp, amount);
            }
            let val = (self.mem(EffectMemorySlot::FinePortaUp) as u16) * 4;
            self.note.period = (std::num::Wrapping(self.note.period) - std::num::Wrapping(val)).0;
            let min_period = if song_type == SongType::S3M || song_type == SongType::IT { 113 } else { 1 };
            if (self.note.period as i16) < min_period {
                self.note.period = min_period as u16;
            }
        }
    }

    pub(crate) fn set_volume(&mut self, voice: Option<&mut Voice>, first_tick: bool, vol: u8) {
        if first_tick {
            if let Some(v) = voice {
                v.volume.set_volume(vol as i32);
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

    pub(crate) fn fine_volume_slide(&mut self, voice: Option<&mut Voice>, first_tick: bool, amount: i8) {
        if first_tick {
            if let Some(v) = voice {
                v.volume.set_volume(v.volume.volume as i32 + amount as i32);
            }
        }
    }

    pub(crate) fn fine_porta_down(&mut self, song_type: SongType, first_tick: bool, amount: u8) {
        if first_tick {
            if amount != 0 {
                self.set_mem(EffectMemorySlot::FinePortaDown, amount);
            }
            let val = (self.mem(EffectMemorySlot::FinePortaDown) as u16) * 4;
            self.note.period += val;
            let max_period = if song_type == SongType::S3M || song_type == SongType::IT { 27392 } else { 31999 };
            if self.note.period > max_period {
                self.note.period = max_period;
            }
        }
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

    pub(crate) fn it_vol_col_volume_slide(&mut self, voice: Option<&mut Voice>, first_tick: bool, mut amount: i8) {
        // Memory stores signed amount as u8 (bit-pattern round-trip)
        // to preserve direction across the param=0 recall.
        if amount == 0 {
            amount = self.mem(EffectMemorySlot::ItVolColVolSlide) as i8;
        } else {
            self.set_mem(EffectMemorySlot::ItVolColVolSlide, amount as u8);
        }
        self.volume_slide(voice, first_tick, amount);
    }

    pub(crate) fn it_vol_col_fine_volume_slide(&mut self, voice: Option<&mut Voice>, first_tick: bool, mut amount: i8) {
        if amount == 0 {
            amount = self.mem(EffectMemorySlot::ItVolColFineVolSlide) as i8;
        } else {
            self.set_mem(EffectMemorySlot::ItVolColFineVolSlide, amount as u8);
        }
        self.fine_volume_slide(voice, first_tick, amount);
    }

    pub(crate) fn it_vol_col_porta_to_note(&mut self, voice: Option<&mut Voice>, first_tick: bool, speed: u8, compatible_g: bool, rate: f32, tables: &AudioTables) {
        let speed = self.recall_or_set(EffectMemorySlot::ItVolColPorta, speed);
        self.porta_to_note(SongType::IT, voice, first_tick, speed, compatible_g, rate, tables);
    }

    pub(crate) fn it_volume_slide(&mut self, voice: Option<&mut Voice>, first_tick: bool, param: u8, fast_volume_slides: bool) {
        let param = self.recall_or_set(EffectMemorySlot::ItVolSlide, param);

        let x = param >> 4;
        let y = param & 0x0F;

        // IT/S3M Dxy: upper nibble wins; DFy/DxF only fine when one is F.
        // `fast_volume_slides` extends non-fine slides to tick 0 (ST3 v3.00
        // quirk) — we forward it by spoofing first_tick.
        if x == 0x0F && y != 0 {        // DFy: Fine Down
            self.fine_volume_slide(voice, first_tick, -(y as i8));
        } else if y == 0x0F && x != 0 { // DxF: Fine Up
            self.fine_volume_slide(voice, first_tick, x as i8);
        } else if x != 0 && y == 0 {    // Dx0: Up by x
            self.volume_slide(voice, first_tick && !fast_volume_slides, x as i8);
        } else if x == 0 && y != 0 {    // D0y: Down by y
            self.volume_slide(voice, first_tick && !fast_volume_slides, -(y as i8));
        } else if x != 0 {              // Dxy with both non-zero: low nibble ignored, slide up
            self.volume_slide(voice, first_tick && !fast_volume_slides, x as i8);
        }
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
        let param = self.recall_or_set(EffectMemorySlot::VolSlide, param);
        if first_tick { return; }
        let x = param >> 4;
        let y = param & 0x0F;

        if x != 0 {
            self.volume_slide(voice, first_tick, x as i8);
        } else if y != 0 {
            self.volume_slide(voice, first_tick, -(y as i8));
        }
    }

    pub(crate) fn channel_volume_slide(&mut self, first_tick: bool, param: u8) {
        if first_tick {
            // Fine slides handled in first tick if needed
            let up = (param >> 4) as i32;
            let down = (param & 0xf) as i32;
            if up == 0xf && down != 0 {
                self.channel_volume = (self.channel_volume as i32 + down).min(64) as u8;
            } else if down == 0xf && up != 0 {
                self.channel_volume = (self.channel_volume as i32 - up).max(0) as u8;
            }
        } else {
            let up = (param >> 4) as i32;
            let down = (param & 0xf) as i32;
            if up != 0x0 && up != 0xf && down == 0 {
                self.channel_volume = (self.channel_volume as i32 + up).min(64) as u8;
            } else if down != 0x0 && down != 0xf && up == 0 {
                self.channel_volume = (self.channel_volume as i32 - down).max(0) as u8;
            }
        }
    }

    pub(crate) fn panning_slide(&mut self, voice: Option<&mut Voice>, first_tick: bool, param: u8, song_type: SongType) {
        let actual_param = self.recall_or_set(EffectMemorySlot::PanningSlide, param);

        let right = (actual_param >> 4) as i32;
        let left = (actual_param & 0xf) as i32;
        let is_xm = matches!(song_type, SongType::XM | SongType::MOD);
        let (r_shift, l_shift) = if is_xm { (right, left) } else { (right << 2, left << 2) };

        // FT2 (XM/MOD): no fine variant — every-tick slide, hi nibble wins
        // when both are non-zero. IT/S3M PFx / PxF are fine slides (tick 0 only).
        if is_xm {
            if !first_tick {
                if let Some(v) = voice {
                    if right != 0 {
                        v.panning.set_panning(v.panning.panning as i32 + r_shift);
                    } else if left != 0 {
                        v.panning.set_panning(v.panning.panning as i32 - l_shift);
                    }
                }
            }
            return;
        }

        if first_tick {
            if right == 0xf && left != 0 {
                if let Some(v) = voice {
                    v.panning.set_panning(v.panning.panning as i32 - l_shift);
                }
            } else if left == 0xf && right != 0 {
                if let Some(v) = voice {
                    v.panning.set_panning(v.panning.panning as i32 + r_shift);
                }
            }
        } else {
            if let Some(v) = voice {
                if right != 0 && right != 0xf && left == 0 {
                    v.panning.set_panning(v.panning.panning as i32 + r_shift);
                } else if left != 0 && left != 0xf && right == 0 {
                    v.panning.set_panning(v.panning.panning as i32 - l_shift);
                }
            }
        }
    }
}
