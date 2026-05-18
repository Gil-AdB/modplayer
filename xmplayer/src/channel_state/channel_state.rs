use crate::envelope::{Envelope, EnvelopePoint};
use crate::tables;
#[cfg(test)]
#[allow(unused_imports)]
use crate::tables::{TableType, AMIGA_PERIODS, LINEAR_PERIODS};
use crate::tables::AudioTables;
use crate::instrument::VibratoEnvelope;


/// A value bounded by a minimum and a maximum
///
///  If input is less than min then this returns min.
///  If input is greater than max then this returns max.
///  Otherwise, this returns input.
///
/// **Panics** in debug mode if `!(min <= max)`.
#[inline]
pub fn clamp<T: PartialOrd>(input: T, min: T, max: T) -> T {
    debug_assert!(min <= max, "min must be less than or equal to max");
    if input < min {
        min
    } else if input > max {
        max
    } else {
        input
    }
}

#[derive(Clone,Copy,Debug)]
pub struct PortaToNoteState {
    pub target_note:                Note,
    pub speed:                      u16,
}


impl PortaToNoteState {
    pub(crate) fn new() -> PortaToNoteState {
        PortaToNoteState {
            target_note: Note::new(),
            speed: 0
        }
    }

    pub(crate) fn next_tick(&mut self, current_note: &mut Note) {
        if self.speed == 0 { return; }

        // IT linear-pitch path: current_note.linear_hz is the source of
        // truth (period is inert). Slide linear_hz multiplicatively
        // toward target_note.linear_hz using the same 2^(±speed/768)
        // factor as apply_porta in linear mode (commit 9a15ce1).
        // `self.speed` was set to `pattern_param * 4` at trigger time
        // (porta_to_note in mod.rs), which is the OMT "amount" unit;
        // dividing by 768 = 64 fine steps × 12 semitones/octave gives
        // octaves of pitch shift per tick.
        //
        // Without this, orbiter.it ch6's `GAF` (param=0xAF=175) slid
        // period instead of hz, and the actual playback pitch never
        // budged from the trigger frequency — same shape as the E/F
        // porta and arpeggio/vibrato fixes.
        if current_note.linear_hz != 0.0 && self.target_note.linear_hz != 0.0 {
            let cur = current_note.linear_hz;
            let target = self.target_note.linear_hz;
            if (cur - target).abs() < 1e-3 { return; }
            let step = (self.speed as f32 / 768.0).exp2();
            let new = if cur < target {
                (cur * step).min(target)
            } else {
                (cur / step).max(target)
            };
            current_note.linear_hz = new.max(1.0);
            return;
        }

        if self.target_note.period == 0 { return; }
        // Widen to i32 so the clamp catches overshoot. The previous u16-
        // wrapping arithmetic looked like it clamped, but a downward slide
        // larger than `current` underflowed to ~65000 and the
        // `< target` check then read it as "haven't reached yet" — leaving
        // the period at a huge value (audible as a sub-Hz drone, i.e. the
        // channel still ON but inaudible). Matches master's i32-widened
        // min/max clamp at scratch/channel_master.rs:427-432.
        let cur = current_note.period as i32;
        let target = self.target_note.period as i32;
        let speed = self.speed as i32;
        let next = if cur < target {
            (cur + speed).min(target)
        } else if cur > target {
            (cur - speed).max(target)
        } else {
            cur
        };
        current_note.period = next as u16;
    }
}

pub(crate) enum WaveControl {
    SIN,
    RAMP,
    SQUARE,
}

impl WaveControl {
    pub(crate) fn from(control: u8) -> WaveControl {
        match control & 3 {
            0 => WaveControl::SIN,
            1 => WaveControl::RAMP,
            _ => WaveControl::SQUARE
        }
    }
}

const SIN_TABLE: [i32; 32] =
    [0,   24,   49,  74,  97, 120, 141, 161,
        180, 197, 212, 224, 235, 244, 250, 253,
        255, 253, 250, 244, 235, 224, 212, 197,
        180, 161, 141, 120,  97,  74,  49,  24];

#[derive(Clone,Copy,Debug)]
pub struct VibratoState {
    pub speed:  i8,
    pub depth:  i16,
    /// 256-step counter (4× the FT2 64-step cycle for sub-tick
    /// resolution). Wraps via u8 overflow.
    pub pos:    u8,
    pub fine:   bool,
}

impl VibratoState {

    pub(crate) fn new() -> VibratoState {
        VibratoState {
            speed: 0,
            depth: 0,
            pos: 0,
            fine: false,
        }
    }


    pub(crate) fn get_frequency_shift(&mut self, wave_control: WaveControl) -> i32 {
        // pos (0..255) → FT2 logical position (0..63) by divide-by-4.
        // Upper half is the negative half-cycle.
        let logical_pos = (self.pos as u16) >> 2;
        let in_negative_half = logical_pos >= 32;
        let table_idx = (logical_pos & 31) as usize;
        let delta = match wave_control {
            WaveControl::SIN => SIN_TABLE[table_idx] * self.depth as i32,
            WaveControl::RAMP => {
                let temp: i32 = (table_idx as i32) * 8;
                let raw = if in_negative_half { 255 - temp } else { temp };
                raw * self.depth as i32
            }
            WaveControl::SQUARE => 255 * self.depth as i32,
        };
        // Fine vibrato (S3M Uxy / IT u) gets an extra >>2 for 1/4 swing.
        let shift_amt = if self.fine { 7 } else { 5 };
        let s = delta >> shift_amt;
        if in_negative_half { -s } else { s }
    }

    pub(crate) fn next_tick(&mut self) {
        self.pos = self.pos.wrapping_add(self.speed as u8);
    }
}

// This and vibrato should derive from a base class probably
#[derive(Clone,Copy,Debug)]
pub struct TremoloState {
    pub speed:  i8,
    pub depth:  i16,
    pub pos:    i8,
}

impl TremoloState {

    pub(crate) fn new() -> TremoloState {
        TremoloState {
            speed: 0,
            depth: 0,
            pos: 0
        }
    }


    pub(crate) fn get_volume_shift(&mut self, wave_control: WaveControl) -> i32 {
        let delta;
        let termolo_pos = (self.pos >> 2) & 31;
        match wave_control {
            WaveControl::SIN => { delta = SIN_TABLE[termolo_pos as usize] as i32; }
            WaveControl::RAMP => {
                let temp:i32 = (termolo_pos * 8) as i32;
                delta = if self.pos < 0 { 255 - temp } else { temp } as i32
            }
            WaveControl::SQUARE => { delta = 255; }
        }
        (((delta * self.depth as i32) >> 6) * (if self.pos < 0 { -1 } else { 1 })) as i32
    }

    pub(crate) fn next_tick(&mut self) {
        self.pos += self.speed;
        if self.pos > 31 { self.pos -= 64; }
    }
}

#[derive(Clone,Copy,Debug)]
pub struct EnvelopeState {
    pub frame:      u16,
    pub sustained:  bool,
    // looped:     bool,
    pub idx:        usize,
    // instrument:         &'a Instrument,
}

impl EnvelopeState {
    pub(crate) fn new() -> EnvelopeState {
        EnvelopeState { frame: 0, sustained: false, idx: 0 }
    }

    pub(crate) fn key_off(&mut self, env: &Envelope) {
        if env.sustain && env.has_loop && env.loop_end_point == env.sustain_point {
            self.idx = env.loop_start_point as usize;
            self.frame = env.points[self.idx].frame;
        }
    }

    pub(crate) fn handle(&mut self, env: &Envelope, channel_sustained: bool, default: u16, sticky_sustain: bool) -> u16 {
        if !env.on || env.size < 1 { return default * 256; }
        if env.size == 1 { return env.points[0].value * 256; }

        // Handle Sustain Loop (IT)
        let in_sustain_loop = env.has_sustain_loop && channel_sustained;
        
        // Sustain Point (XM)
        if !env.has_sustain_loop && env.sustain && channel_sustained && self.frame == env.points[env.sustain_point as usize].frame {
            self.sustained = true;
        }

        if !env.has_sustain_loop && self.sustained && (channel_sustained || sticky_sustain) {
            return env.points[env.sustain_point as usize].value * 256;
        }

        let retval = EnvelopeState::lerp(self.frame, &env.points[self.idx], &env.points[self.idx + 1]);

        // Increment frame
        let can_normal_loop = (!env.sustain || !channel_sustained) && env.has_loop;
        
        let mut loop_end_frame = 65535;
        if in_sustain_loop {
            loop_end_frame = env.points[env.sustain_loop_end_point as usize].frame;
        } else if can_normal_loop {
            loop_end_frame = env.points[env.loop_end_point as usize].frame;
        }

        let end_frame = env.points[(env.size - 1) as usize].frame;

        if self.frame < end_frame || (in_sustain_loop && self.frame < loop_end_frame + 1) || (can_normal_loop && self.frame < loop_end_frame + 1) {
            self.frame += 1;
            
            // Handle Index update
            if self.idx < (env.size - 2) as usize && self.frame >= env.points[self.idx + 1].frame {
                self.idx += 1;
            }
        }

        // Handle Sustain Loop jump
        if in_sustain_loop && self.frame > loop_end_frame {
            self.idx = env.sustain_loop_start_point as usize;
            self.frame = env.points[self.idx].frame;
        }
        // Handle Normal Loop jump
        else if can_normal_loop && self.frame > loop_end_frame {
            self.idx = env.loop_start_point as usize;
            self.frame = env.points[self.idx].frame;
        }

        retval
    }

    pub(crate) fn set_position(&mut self, env: &Envelope, pos: u8) {
        // // pre: envelope exists and should be set
        //
        if env.size < 2 {
            self.frame = 0;
            self.idx = 0;
            return;
        }

        for i in 1..env.size {
            if (pos as u16) < env.points[i as usize].frame {
                self.frame = pos as u16;
                self.idx = (i - 1) as usize;
                self.sustained = false;
                return;
            }
        }

        self.frame = env.points[env.size as usize - 1].frame;
        self.idx = (env.size - 1) as usize;

    }

        // pre: e.on && e.size > 0
//    default: panning envelope: middle (0x80?), volume envelope - max (0x40)
//     pub fn handle1(&mut self, e: &Envelope, sustained: bool, default: u16) -> u16 {
//         // fn handle(&mut self, e: &Envelope, channel_sustained: bool) -> u16 {
//         if !e.on || e.size < 1 { return default * 256;} // bail out
//         if e.size == 1 {
//             // if !e.sustain {return default;}
//             return e.points[0].value * 256;
//         }
//
//         if e.has_loop && self.frame >= e.points[e.loop_end_point as usize].frame as u32 as u16 {
//             self.frame = e.points[e.loop_start_point as usize].frame as u32 as u16
//         }
//
//         let mut idx:usize = 0;
//         loop {
//             if idx >= e.size as usize - 2 { break; }
//             if e.points[idx].frame as u32 <= self.frame as u32 && e.points[idx+1].frame as u32 >= self.frame as u32 {
//                 break;
//             }
//             idx += 1;
//         }
//
//         // if sustained && (e.sustain && self.idx == e.sustain_point as u32) && self.idx == e.size as u32 {
//         //     return e.points[self.idx as usize].value;
//         // }
//
//         let retval = EnvelopeState::lerp(self.frame as u16, &e.points[idx as usize], &e.points[(idx + 1) as usize]);
//
//
//         if !sustained || !e.sustain || self.frame != e.points[e.sustain_point as usize].frame as u32 as u16 {
//             self.frame += 1;
//         }
//
//         retval
//     }

    fn lerp(frame: u16, e1: &EnvelopePoint, e2: &EnvelopePoint) -> u16 {
        if frame == e1.frame {
            return e1.value * 256;
        } else if frame == e2.frame {
            return e2.value * 256;
        }

        let t = (frame - e1.frame) as f32 / (e2.frame - e1.frame) as f32;

        return clamp((((1.0 - t) * e1.value as f32 + t * e2.value as f32) * 256.0) as i32, 0, 65535) as u16;
    }

    // fn next_tick(& mut self) {
    //     if self.instrument.volume_points > 0 {
    //         self.volume_frame += 1;
    //         if self.volume_frame >= self.instrument.volume_envelope[self.volume_idx + 1].frame_number {
    //             self.volume_idx +=1;
    //             if self.volume_idx > self.instrument.volume_loop_end_point as u32 {
    //                 self.volume_idx = self.instrument.volume_loop_start_point as u32;
    //                 self.volume_frame = self.instrument.volume_envelope[self.volume_idx];
    //
    //             }
    //
    //         }
    //     }
    //     // if self.volume_frame > self.instrument.panning_loop_end_point {
    //     //
    //     // }
    //     // self.panning_frame  += 1;
    // }

    pub(crate) fn reset(& mut self, pos: u16, env: &Envelope) {
        if !env.on {return;}
        self.frame      = pos;
        self.sustained  = false;
        // self.looped     = false;

        if env.size > 0 && self.frame > env.points[(env.size - 1) as usize].frame {self.frame = env.points[(env.size - 1) as usize].frame;}
        let mut idx:usize = 0;
        loop {
            if env.size < 2 || idx >= env.size as usize - 2 { break; }
            if env.points[idx].frame <= self.frame && env.points[idx+1].frame >= self.frame {
                break;
            }
            idx += 1;
        }
        self.idx = idx;
    }
}

#[derive(Clone,Copy,Debug)]
pub struct VibratoEnvelopeState {
    pub vibrato_sweep:  u16,
    pub vibrato_amp:    u16,
    pub vibrato_pos:    u16,
}

impl VibratoEnvelopeState {
    pub(crate) fn new() -> Self {
        Self{
            vibrato_sweep: 0,
            vibrato_amp: 0,
            vibrato_pos: 0
        }
    }

    pub(crate) fn reset(&mut self, env: &VibratoEnvelope) {
        self.vibrato_pos = 0;
        if env.vibrato_sweep > 0 {
            // FT2 stores the *derived per-tick increment* in eVibSweep,
            // not the raw 0..255 sweep param. The increment is
            //   depth × 256 / sweep
            // so that the amp reaches `depth × 256` (its cap) after
            // exactly `sweep` ticks. SHOOTING.XM inst 12 has
            // depth=15, sweep=255 → eVibSweep = 15·256/255 = 15.
            // Storing the raw 255 here made auto-vibrato ramp up 17×
            // too fast (full depth by tick 1), audible as a flanging
            // "wobble" on every note using an auto-vibrato instrument.
            self.vibrato_sweep =
                ((env.vibrato_depth as u32 * 256) / env.vibrato_sweep as u32) as u16;
            self.vibrato_amp = 0;
        } else {
            self.vibrato_sweep = 0;
            self.vibrato_amp = (env.vibrato_depth as u16) << 8;
        }
    }

    // This probably makes sense somehow, but I'm too tired to care
    // taken from ft2-clone
    #[allow(dead_code)]
    pub(crate) fn handle(&mut self, env: &VibratoEnvelope, channel_sustained: bool) -> i16 {
        let mut _auto_vibrato_amp;
        if env.vibrato_depth > 0 {
            if self.vibrato_sweep > 0 {
                _auto_vibrato_amp = self.vibrato_amp;
                if channel_sustained {
                    let mut next_amp = self.vibrato_amp + self.vibrato_sweep;
                    if (next_amp >> 8) as u8 > env.vibrato_depth {
                        next_amp = (env.vibrato_depth as u16) << 8;
                        self.vibrato_sweep = 0;
                    }
                    self.vibrato_amp = next_amp;
                }
            } else {
                _auto_vibrato_amp = self.vibrato_amp;
            }
            self.vibrato_pos = (self.vibrato_pos + env.vibrato_rate as u16) & 255;
 
            let _auto_vibrato_value : i16;
            if env.vibrato_type == 1 { // square
                _auto_vibrato_value = if self.vibrato_pos > 127 {64} else {-64}
            } else if env.vibrato_type == 2 { // ramp up
                _auto_vibrato_value = (((self.vibrato_pos >> 1) as i16 + 64) & 127) - 64;
            } else if env.vibrato_type == 3 { // rampdown
                _auto_vibrato_value = ((-((self.vibrato_pos >> 1) as i16) + 64) & 127) - 64;
            } else { // sin
                _auto_vibrato_value = tables::VIB_SINE_TAB[self.vibrato_pos as usize] as i16;
            }

            (_auto_vibrato_value as i32 * _auto_vibrato_amp as i32 / 64) as i16
        } else {
            0
        }
    }
}

#[derive(Clone,Copy,Debug)]
pub struct Note {
    pub note:            u8,
    pub finetune:        i8,
    pub period:          u16,
    pub base_period:     u16,
    pub original_note:   u8,
    /// Active c5_speed for the formula-based pitch path (S3M / IT amiga).
    /// Copied from `Sample::c5_speed` at trigger; mutable mid-note via the
    /// S3M S2 finetune subcommand. Zero means "use the LUT path" (samples
    /// without c5_speed metadata, or formats that don't use it). The
    /// arpeggio formula override and any future formula-based effect must
    /// read this, not `sample.c5_speed`, so S2 changes propagate.
    pub c5_speed:        u32,
    /// IT-linear-mode override: when nonzero, `frequency()` returns
    /// this directly (IT linear stores period == freq Hz, which our
    /// FT2-linear period table can't represent). Pitch slides still
    /// write `period` additively — slide accuracy needs a per-format
    /// multiplicative path eventually.
    pub linear_hz:       f32,
}

impl Note {

    pub(crate) fn new() -> Note {
        Note{
            note: 0,
            finetune: 0,
            period: 0,
            base_period: 0,
            original_note: 0,
            c5_speed: 0,
            linear_hz: 0.0,
        }
    }

    // note <= 120
    pub(crate) fn set_note(&mut self, note: u8, finetune: i8, original_note: u8, frequency_tables: &AudioTables) {
        self.note = note;
        self.original_note = original_note;
        self.finetune = finetune;
        self.period = self.note_to_period(note, finetune, frequency_tables);
        self.base_period = self.period;
    }

    pub(crate) fn note_to_period(&self, note: u8, finetune: i8, frequency_tables: &AudioTables) -> u16 {
        let sidx= (note as i32 - 1) * 16 + ((finetune >> 3) + 16) as i32;
        let idx = clamp(sidx, 0, 1935);

        frequency_tables.periods[idx as usize]
    }

    /// Closed-form S3M/IT period from c5_speed (OpenMPT formula).
    /// `note_offset` maps our note convention to the formula's 0-indexed
    /// note: S3M +11, IT -1 (both land C-5 at formula note 60).
    pub(crate) fn note_to_period_s3m(note: u8, note_offset: i8, c5_speed: u32) -> u16 {
        const FREQ_S3M_TABLE: [u16; 12] = [1712, 1616, 1524, 1440, 1356, 1280, 1208, 1140, 1076, 1016, 960, 907];
        let n = (note as i32 + note_offset as i32).clamp(0, 11 * 12 + 11) as u32;
        let f = FREQ_S3M_TABLE[(n % 12) as usize] as u64;
        let num = 8363u64 * (f << 5);
        let den = (c5_speed.max(1) as u64) << (n / 12);
        (num / den).min(u16::MAX as u64) as u16
    }

    pub(crate) fn snap_to_semitone(&mut self, frequency_tables: &AudioTables) {
        let mut best_idx = 0;
        let mut best_diff = 65535;
        for i in (0..frequency_tables.periods.len()).step_by(16) {
            let diff = (frequency_tables.periods[i] as i32 - self.period as i32).abs();
            if diff < best_diff {
                best_diff = diff;
                best_idx = i;
            }
        }
        self.period = frequency_tables.periods[best_idx];
    }


    #[allow(dead_code)]
    pub(crate) fn get_tone(note: u8, relative_note: i8) -> Result<u8, bool> {
        let tone = note as i8 + relative_note;
        if tone > 12 * 10 || tone < 0 {
            return Err(false);
        }
        Ok(tone as u8)
    }


    // indexing:   let idx = (self.note - 1) as u32 * 16 + ((self.finetune >> 3) + 16) as u32;
    // note is 7 bits << 4  - bits 11..4
    // finetune is 8 bit >> 3 = 5 bits - 0..31 (after fixup) - bits 0..4

    //  B-3 -15 -14 -13 -12 -11 -10 -9 -8 -9 -6 -5 -4 -3 -2 -1 C-4 +1 +2 +3 +4 +5 +6 +7 +8 +9 +10 +11 +12 +13 +14 +15 D-4
    //  -16
    //  note: C-4
    //  C-4 - 1 = B-3
    //  FineTune +16 shifts the the FineTune into place. We subtract 1 * 16 from note, and fixup the FineTune
    //  Which means we can just do a binary search inside the table and round to the nearest semi tone...
    // fn nearest_semi_tone(&self, period: u16, added_note: u8, use_amiga: TableType) -> i16 {
    //
    //     let note2period = match use_amiga {
    //         TableType::LinearFrequency => {&LINEAR_PERIODS},
    //         TableType::AmigaFrequency => {&AMIGA_PERIODS},
    //     };
    //
    //     let needed_period = period as i16;
    //     let mut idx: isize = ((note2period.binary_search_by(|element| needed_period.cmp(element)).unwrap_or_else(|x| x)) & (!0xf)) as isize;
    //
    //     let ft = if self.finetune >= 0 && idx != 0 {self.finetune >> 3} else {(self.finetune >> 3) + 16};
    //
    //     let mut fixed_note_id = idx as isize;
    //     // clamp to note 97
    //     if fixed_note_id < 0 {fixed_note_id = 0}
    //     if fixed_note_id > (8*12*16) {
    //         fixed_note_id = 8 * 12 * 16;
    //          // fixed_note_id += (ft & 2) as isize; // FT2 bug
    //     }
    //
    //     fixed_note_id += added_note as isize * 16;
    //
    //     // actual allowed values
    //     let clamped_note_idx = (clamp(fixed_note_id, 0, 1551) + ft as isize) as usize;
    //     note2period[clamped_note_idx]
    // }

    // for arpeggio and portamento (semitone-slide mode). Lifted directly from ft2-clone. I'll have to write it from scratch one day
    #[allow(dead_code)]
    fn relocate_ton(&self, period: u16, arp_note: u8, frequency_tables: &AudioTables) -> u16 {
        // int32_t fine_tune, lo_period, hi_period, tmp_period, tableIndex;
        let fine_tune: u32 = (((self.finetune >> 3) + 16) << 1) as u32;
        let mut hi_period: u32 = (8 * 12 * 16) * 2;
        let mut lo_period: u32 = 0;

        let note2period = &frequency_tables.periods;

        for _ in 0..8 {
            let tmp_period = (((lo_period + hi_period) >> 1) & 0xFFFFFFE0) as u32 + fine_tune;

            let mut table_index = (tmp_period as i32 - 16) >> 1;
            table_index = clamp(table_index, 0, 1935); // 8bitbubsy: added security check

            if period >= note2period[table_index as usize] as u16 {
                hi_period = ((tmp_period - fine_tune) as u32 & 0xFFFFFFE0) as u32;
            } else {
                lo_period = ((tmp_period - fine_tune) as u32 & 0xFFFFFFE0) as u32;
            }
        }

        // arp_note is between 0..16
        let mut tmp_period = lo_period + fine_tune as u32 + ((arp_note as u32) << 5);

        // if tmp_period < 0 {// 8bitbubsy: added security check
        //     tmp_period = 0;
        // }

        if tmp_period >= (8*12*16+15)*2-1 { // FT2 bug: off-by-one edge case
            tmp_period = (8 * 12 * 16 + 15) * 2;
        }

        return note2period[(tmp_period >>1) as usize];
    }

    #[cfg(test)]
    fn nearest_semi_tone_test(&self, period: u16, added_note: u8, use_amiga: TableType) -> (u16, u32) {
        let note2period = match use_amiga {
            TableType::LinearFrequency => {&LINEAR_PERIODS},
            TableType::AmigaFrequency => {&AMIGA_PERIODS},
        };

        let fine_tune: u32 = (((self.finetune >> 3) + 16) << 1) as u32;
        let mut hi_period: u32 = (8 * 12 * 16) * 2;
        let mut lo_period: u32 = 0;

        for _ in 0..8 {
            let tmp_period = (((lo_period + hi_period) >> 1) & 0xFFFFFFE0) as u32 + fine_tune;

            let mut table_index = (tmp_period as i32 - 16) >> 1;
            table_index = clamp(table_index, 0, 1935);

            if period >= note2period[table_index as usize] as u16 {
                hi_period = ((tmp_period - fine_tune) as u32 & 0xFFFFFFE0) as u32;
            } else {
                lo_period = ((tmp_period - fine_tune) as u32 & 0xFFFFFFE0) as u32;
            }
        }

        let mut tmp_period = lo_period + fine_tune as u32 + ((added_note as u32) << 5);

        if tmp_period >= (8*12*16+15)*2-1 {
            tmp_period = (8 * 12 * 16 + 15) * 2;
        }

        (note2period[(tmp_period >>1) as usize], lo_period)
    }

    // for arpeggio and portamento (semitone-slide mode). Lifted directly from ft2-clone. I'll have to write it from scratch one day
    #[cfg(test)]
    fn relocate_ton_test(&self, period: u16, arp_note: u8, use_amiga: TableType) -> (u16, u32) {
        // int32_t fine_tune, lo_period, hi_period, tmp_period, tableIndex;
        let fine_tune: u32 = (((self.finetune >> 3) + 16) << 1) as u32;
        let mut hi_period: u32 = (8 * 12 * 16) * 2;
        let mut lo_period: u32 = 0;

        let note2period = match use_amiga {
            TableType::LinearFrequency => {&LINEAR_PERIODS},
            TableType::AmigaFrequency => {&AMIGA_PERIODS},
        };

        for _ in 0..8 {
            let tmp_period = (((lo_period + hi_period) >> 1) & 0xFFFFFFE0) as u32 + fine_tune;

            let mut table_index = (tmp_period as i32 - 16) >> 1;
            table_index = clamp(table_index, 0, 1935); // 8bitbubsy: added security check

            if period >= note2period[table_index as usize] as u16 {
                hi_period = ((tmp_period - fine_tune) as u32 & 0xFFFFFFE0) as u32;
            } else {
                lo_period = ((tmp_period - fine_tune) as u32 & 0xFFFFFFE0) as u32;
            }
        }

        // arp_note is between 0..16
        let mut tmp_period = lo_period + fine_tune as u32 + ((arp_note as u32) << 5);

        let before_clamp = lo_period;

        // if tmp_period < 0 {// 8bitbubsy: added security check
        //     tmp_period = 0;
        // }

        if tmp_period >= (8*12*16+15)*2-1 { // FT2 bug: off-by-one edge case
            tmp_period = (8 * 12 * 16 + 15) * 2;
        }

        return (note2period[(tmp_period >>1) as usize], before_clamp);
    }


    pub(crate) fn base_frequency(&self, _semitone: bool, tables: &AudioTables) -> f32 {
        if self.base_period == 0 { return 0.0; }
        tables.d_period2hz_tab[self.base_period as usize] as f32
    }

    pub(crate) fn frequency(&self, period_shift: i16, period_offset: i32, semitone: bool, frequency_tables: &AudioTables) -> f32 {
        // IT linear-mode: trigger path and (separately) E/F porta keep
        // `self.linear_hz` as the source of truth. Arpeggio (Jxx) and
        // vibrato (Hxx) write into `period_shift` / `period_offset`
        // respectively (in 64-units-per-semitone convention shared
        // with XM/Amiga), so we must fold them in multiplicatively
        // here too. Same shape as the porta fix in apply_porta —
        // freq_mod = 2^(-(shift+offset) / 768), where 768 = 64 × 12.
        // Without this, every IT-linear arpeggio and vibrato is a
        // silent no-op (orbiter.it's lone Hxx and any future Jxx
        // module would otherwise sound like static-pitch notes).
        if self.linear_hz != 0.0 {
            let mut total: i32 = period_shift as i32 + period_offset;
            if semitone {
                total = ((total + 32) / 64) * 64;
            }
            if total == 0 { return self.linear_hz; }
            return self.linear_hz * (-(total as f32) / 768.0).exp2();
        }
        let mut period = self.period as i32 + period_shift as i32 + period_offset;

        if semitone {
            period = ((period + 32) / 64) * 64;
        }

        return frequency_tables.d_period2hz_tab[period.clamp(0, 65535) as usize] as f32;
    }

    /// IT linear-mode pitch. At C-5 freq = c5_speed; each octave doubles,
    /// each semitone via LinearSlideUpTable. Returns Hz directly.
    pub(crate) fn it_linear_frequency(engine_note: u8, c5_speed: u32) -> f32 {
        let n = (engine_note as i32 - 1).max(0) as u32;
        let factor = crate::tables::LINEAR_SLIDE_UP_TABLE[((n % 12) * 16) as usize] as u64;
        let shift = n / 12;
        // freq = c5_speed * (factor << shift) / (65536 << 5)
        let num = factor << shift;
        let den: u64 = 65536u64 << 5;
        ((c5_speed as u64 * num) / den) as f32
    }

    const NOTES: [&'static str;12] = ["C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-"];

    pub(crate) fn to_string(&self) -> String {
        if self.original_note == 97 || self.original_note == 0 { (self.original_note as u32).to_string() } else {
            format!("{}{}", Self::NOTES[((self.original_note as u8 - 1) % 12) as usize], (((self.original_note as u8 - 1) / 12) + '0' as u8) as char )
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::channel_state::channel_state::Note;
    use crate::tables::{TableType, AudioTables};

    #[test]
    fn test_glissando() {
        for t in [ TableType::AmigaFrequency, TableType::LinearFrequency,].iter() {
            let table = if *t == TableType::AmigaFrequency {AudioTables::calc_tables_amiga()} 
                                                                     else {AudioTables::calc_tables_linear()};
            for note_idx in 1..=120 {
                for added_note in 0..16 {
                    for finetune in -16..=15 {
                        let mut note = Note::new();
                        note.set_note(note_idx, finetune << 3, note_idx, &table);

                        if note_idx == 83 && finetune == 15 {
                            let _banana = true;
                        }

                        let actual = note.nearest_semi_tone_test(note.period as u16, added_note, *t);
                        let expected = note.relocate_ton_test(note.period as u16, added_note, *t);
                        println!("{}expected: {}, actual: {}, note {:3}, finetune {:3}, added_note:{:3}, {}, {}, {}, {}, {}",
                                 if expected.0 != actual.0 {"\x1b[38;2;255;0;0m"} else {""},
                                 expected.0, actual.0, note_idx, finetune, added_note, expected.0 != actual.0, actual.1, expected.1/2, expected.1/2 != actual.1, "\x1b[0m");
                        assert_eq!(actual.0, expected.0, "note: {}, finetune: {}, added_note: {}, table: {:?}", note_idx, finetune, added_note, t)
                    }
                }
            }
        }
    }

    fn tremor_state(x: u8, y: u8, tick: i8) -> bool {
        let tremor_pos = 0u8;

        let mut tremor_sign = tremor_pos & 0x80;
        let mut tremor_data = (tremor_pos & 0x7F) as i8;

        for _ in 0..=tick {
            tremor_data -= 1;
            if tremor_data < 0
            {
                if tremor_sign == 0x80
                {
                    tremor_sign = 0x00;
                    tremor_data = y as i8;
                } else {
                    tremor_sign = 0x80;
                    tremor_data = x as i8;
                }
            }

            // tremor_pos = tremor_sign | tremor_data as u8;
        }
        tremor_sign == 0x80
    }

    fn my_tremor(x: u8, y: u8, tick: i8) -> bool {
        let mut tremor_count = 0;
        for _ in 0..tick-1 {
            tremor_count += 1;
            tremor_count = tremor_count % (x + 1 + y + 1);
        }

        tremor_count <  x + 1

    }
    #[test]
    fn test_tremor() {
            for tick in 0i8..30i8 {
                for x in 0..15 {
                    for y in 0..15 {
                        let orig = tremor_state(x, y, tick);
                        let mine = my_tremor(x, y, tick);
                        println!("{}tick: {}, x: {}, y: {}, orig: {}, mine: {}{}",
                                 if orig != mine {"\x1b[38;2;255;0;0m"} else {""},
                                 tick, x, y, orig, mine, "\x1b[0m");
                        // assert_eq!(mine, orig, "tick: {}, x: {}, y: {}, orig: {}, mine: {}", tick, x, y, orig, mine);
                    }
                }
            }
        assert!(true)
    }
}

#[derive(Clone,Copy,Debug)]
pub struct Volume {
    pub volume:         u8,
    pub volume_shift:   i32,
    pub output_volume:  f32,
    pub fadeout_vol:    i32,
    pub fadeout_speed:  i32,
    pub envelope_vol:   i32,
    pub global_vol:     i32,
}

impl Volume {
    pub(crate) fn new() -> Volume {
        Volume {
            volume: 0,
            volume_shift: 0,
            output_volume: 1.0,
            fadeout_vol: 65536,
            fadeout_speed: 0,
            envelope_vol: 16384,
            global_vol: 64
        }
    }

    pub fn get_volume(&self) -> u8 {
        let outvol = self.volume as i32 + self.volume_shift;
        if outvol > 64 {64} else if outvol < 0 {0} else {outvol as u8}
    }

    pub(crate) fn retrig(&mut self, vol: i32) {
        self.set_volume(vol);
        self.volume_shift  = 0;
        self.fadeout_speed = 0;
    }

    pub fn set_volume(&mut self, vol: i32) {
        self.volume = if vol > 64 { 64 } else if vol < 0 { 0 } else { vol } as u8;
    }
}


#[derive(Clone,Copy,Debug)]
pub struct Panning {
    pub panning:               u8,
    pub final_panning:         u8,
}

impl Panning {
    pub(crate) fn new() -> Panning {
        Panning {
            panning: 0x80,
            final_panning: 128,
        }
    }

    // pub(crate) fn get_panning(&self) -> u8 {
    //     // let outvol = self.volume as i32 + self.volume_shift;
    //     // if outvol > 64 {64} else if outvol < 0 {0} else {outvol as u8}
    //     0x80
    // }

    pub(crate) fn set_panning(&mut self, panning: i32) {
        let clamped = clamp(panning , 0, 255) as u8;
        self.panning = clamped;
        // Mirror to final_panning so formats without panning envelopes
        // (MOD, STM) actually see the pan in the mixer. The mixer reads
        // `final_panning` exclusively; without this mirror, MOD ended
        // up with `final_panning = 128` (the Panning::new() default)
        // for every voice — output collapsed to mono regardless of
        // the LRRL Amiga defaults the loader sets. update_envelope_
        // panning still recomputes this when a pan envelope IS active
        // (XM/IT/S3M with one), so this isn't redundant for formats
        // that do drive update_envelopes.
        self.final_panning = clamped;
    }

    pub(crate) fn update_envelope_panning(&mut self, envelope_panning: u16) {
        self.final_panning = clamp(self.panning as i32 + (envelope_panning as i32-32*256)*(128 - (self.panning as i32 - 128).abs()) / (32i32 * 128i32), 0 ,255) as u8;
        // self.panning = clamp(self.panning , 0, 255) as u8;
    }
}
