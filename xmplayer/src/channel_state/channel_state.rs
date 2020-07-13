use crate::envelope::{Envelope, EnvelopePoint};
use crate::tables;
use crate::tables::{LINEAR_PERIODS, AMIGA_PERIODS};
use std::sync::atomic::Ordering::{Acquire, Relaxed};
use array_const_fn_init::array_const_fn_init;
use std::sync::Arc;
use crate::channel_state::channel_state::TableType::AmigaFrequency;
use std::num::Wrapping;

pub static mut USE_AMIGA : std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);


/// A value bounded by a minimum and a maximum
///
///  If input is less than min then this returns min.
///  If input is greater than max then this returns max.
///  Otherwise this returns input.
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

fn log(i: usize) -> f64 {
    (i as f64 / 768.0).exp2() * (8363.0 * 256.0)
}

fn log_table() -> [f64; 768] {
    let mut result = [0.0f64; 768];
    for i in 0..768 {
        result[i] = log(i);
    }

    result
}

lazy_static! {
static ref D_LOG_TAB: [f64; 768] = log_table();
}

#[derive(Eq, PartialEq)]
enum TableType {
    LinearFrequency,
    AmigaFrequency
}

struct AudioTables {
    Periods:        [u16; 1936],
    dPeriod2HzTab:  [f64; 65536],
}

impl AudioTables {
    fn calcTablesLinear() -> Self // taken directly from ft2clone
    {
        let mut result = Self { Periods: [0u16; 1936], dPeriod2HzTab: [0.0f64; 65536] };
        result.dPeriod2HzTab[0] = 0.0; // in FT2, a period of 0 yields 0Hz

        result.Periods = LINEAR_PERIODS;
        // linear periods
        for i in 1..65536 {
            let invPeriod = (12 * 192 * 4 as u16).wrapping_sub(i as u16); // this intentionally overflows uint16_t to be accurate to FT2
            let octave = invPeriod as u32 / 768;
            let period = invPeriod as u32 % 768;
            let bitshift = (14u32.wrapping_sub(octave)) & 0x1F; // 100% accurate to FT2

            result.dPeriod2HzTab[i] = D_LOG_TAB[period as usize] / (1 << bitshift) as f64;
        }
        result
    }

    fn calcTablesAmiga() -> AudioTables // taken directly from ft2clone
    {
        let mut result = Self { Periods: [0u16; 1936], dPeriod2HzTab: [0.0f64; 65536] };
        result.dPeriod2HzTab[0] = 0.0; // in FT2, a period of 0 yields 0Hz
        result.Periods = AMIGA_PERIODS;
        // Amiga periods
        for i in 1..65536 {
            result.dPeriod2HzTab[i] = (8363.0 * 1712.0) / i as f64;
        }
        result
    }
}

lazy_static! {
static ref AmigaTables:   AudioTables   = AudioTables::calcTablesAmiga();
static ref LinearTables:  AudioTables   = AudioTables::calcTablesLinear();
}

#[derive(Clone,Copy,Debug)]
pub(crate) struct PortaToNoteState {
    pub(crate) target_note:                Note,
    pub(crate) speed:                      u8,
}


impl PortaToNoteState {
    pub(crate) fn new() -> PortaToNoteState {
        PortaToNoteState {
            target_note: Note{
                note: 0,
                finetune: 0,
                period: 0
            },
            speed: 0
        }
    }
}

pub(crate) enum WaveControl {
    SIN,
    RAMP,
    SQUARE,
}

const SIN_TABLE: [i32; 32] =
    [0,   24,   49,  74,  97, 120, 141, 161,
        180, 197, 212, 224, 235, 244, 250, 253,
        255, 253, 250, 244, 235, 224, 212, 197,
        180, 161, 141, 120,  97,  74,  49,  24];

#[derive(Clone,Copy,Debug)]
pub(crate) struct VibratoState {
    speed:  i8,
    depth:  i8,
    pos:    i8,
}

impl VibratoState {

    pub(crate) fn new() -> VibratoState {
        VibratoState {
            speed: 0,
            depth: 0,
            pos: 0
        }
    }

    pub(crate) fn set_speed(&mut self, speed: i8) {
        if speed != 0 {
            self.speed = speed;
        }
    }

    pub(crate) fn set_depth(&mut self, depth: i8) {
        if depth != 0 {
            self.depth = depth;
        }
    }

    pub(crate) fn get_frequency_shift(&mut self, wave_control: WaveControl) -> i32 {
        let delta;
        let vibrato_pos = (self.pos >> 2) & 31;
        match wave_control {
            WaveControl::SIN => { delta = SIN_TABLE[vibrato_pos as usize] * self.depth as i32; }
            WaveControl::RAMP => {
                let temp:i32 = (vibrato_pos * 8) as i32;
                delta = if self.pos < 0 { 255 - temp } else { temp } as i32
            }
            WaveControl::SQUARE => { delta = 255; }
        }
        ((delta >> 5) * (if self.pos < 0 { -1 } else { 1 })) as i32
    }

    pub(crate) fn next_tick(&mut self) {
        self.pos += self.speed;
        if self.pos > 31 { self.pos -= 64; }
    }
}

// This and vibrato should derive from a base class probably
#[derive(Clone,Copy,Debug)]
pub(crate) struct TremoloState {
    speed:  i8,
    depth:  i8,
    pos:    i8,
}

impl TremoloState {

    pub(crate) fn new() -> TremoloState {
        TremoloState {
            speed: 0,
            depth: 0,
            pos: 0
        }
    }

    pub(crate) fn set_speed(&mut self, speed: i8) {
        if speed != 0 {
            self.speed = speed;
        }
    }

    pub(crate) fn set_depth(&mut self, depth: i8) {
        if depth != 0 {
            self.depth = depth;
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
pub(crate) struct EnvelopeState {
    frame:      u16,
    pub(crate) sustained:  bool,
    looped:     bool,
    idx:        usize,
    // instrument:         &'a Instrument,
}

impl EnvelopeState {
    pub(crate) fn new() -> EnvelopeState {
        EnvelopeState { frame: 0, sustained: false, looped: false, idx: 0 }
    }

    pub(crate) fn handle(&mut self, env: &Envelope, channel_sustained: bool, default: u16) -> u16 {
        if !env.on || env.size < 1 { return default * 256;} // bail out

        if env.size == 1 { // whatever
            return env.points[0].value * 256
        }

        // set sustained if channel_state is sustained, we have a sustain point and we reached the sustain point
        if !self.looped && !self.sustained && env.sustain && channel_sustained && self.frame == env.points[env.sustain_point as usize].frame {
            self.sustained = true;
        }

        // if sustain was triggered, it's sticky
        if self.sustained {
            return env.points[self.idx].value * 256
        }

        // loop
        if env.has_loop && self.frame == env.points[env.loop_end_point as usize].frame {
            self.looped = true;
            self.idx = env.loop_start_point as usize;
            self.frame = env.points[self.idx].frame;
        }

        // reached the end
        if self.idx == (env.size - 2) as usize && self.frame == env.points[self.idx + 1].frame {
            return env.points[self.idx + 1].value * 256
        }

        let retval = EnvelopeState::lerp(self.frame, &env.points[self.idx], &env.points[self.idx + 1]);

        if self.frame < env.points[(env.size - 1) as usize].frame {
            self.frame += 1;
            if self.idx < (env.size - 2) as usize && self.frame == env.points[self.idx + 1].frame {
                self.idx += 1;
            }
        }

        retval
    }

    // pre: e.on && e.size > 0
//    default: panning envelope: middle (0x80?), volume envelope - max (0x40)
    pub fn handle1(&mut self, e: &Envelope, sustained: bool, default: u16) -> u16 {
        // fn handle(&mut self, e: &Envelope, channel_sustained: bool) -> u16 {
        if !e.on || e.size < 1 { return default;} // bail out
        if e.size == 1 {
            // if !e.sustain {return default;}
            return e.points[0].value * 256;
        }

        if e.has_loop && self.frame >= e.points[e.loop_end_point as usize].frame as u32 as u16 {
            self.frame = e.points[e.loop_start_point as usize].frame as u32 as u16
        }

        let mut idx:usize = 0;
        loop {
            if idx >= e.size as usize - 2 { break; }
            if e.points[idx].frame as u32 <= self.frame as u32 && e.points[idx+1].frame as u32 >= self.frame as u32 {
                break;
            }
            idx += 1;
        }

        // if sustained && (e.sustain && self.idx == e.sustain_point as u32) && self.idx == e.size as u32 {
        //     return e.points[self.idx as usize].value;
        // }

        let retval = EnvelopeState::lerp(self.frame as u16, &e.points[idx as usize], &e.points[(idx + 1) as usize]);


        if !sustained || !e.sustain || self.frame != e.points[e.sustain_point as usize].frame as u32 as u16 {
            self.frame += 1;
        }

        retval
    }

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
        self.looped     = false;

        if self.frame > env.points[(env.size - 1) as usize].frame {self.frame = env.points[(env.size - 1) as usize].frame;}
        let mut idx:usize = 0;
        loop {
            if idx >= env.size as usize - 2 { break; }
            if env.points[idx].frame <= self.frame && env.points[idx+1].frame >= self.frame {
                break;
            }
            idx += 1;
        }
        self.idx = idx;
    }
}

#[derive(Clone,Copy,Debug)]
pub(crate) struct Note {
    note:       u8,
    finetune:   i8,
    pub(crate) period:     u16
}

impl Note {

    pub(crate) fn new() -> Note {
        Note{
            note: 0,
            finetune: 0,
            period: 0
        }
    }

    // note <= 120
    pub(crate) fn set_note(&mut self, note: u8, finetune: i8) {

        self.note = note;
        self.finetune = finetune;
        let idx = (self.note - 1) as u32 * 16 + ((self.finetune >> 3) + 16) as u32;

        unsafe { self.period = if USE_AMIGA.load(Relaxed) { tables::AMIGA_PERIODS[idx as usize] } else { tables::LINEAR_PERIODS[idx as usize] }; }
    }


    pub(crate) fn get_tone(note: u8, relative_note: i8) -> Result<u8, bool> {
        let tone = note as i8 + relative_note;
        if tone > 12 * 10 || tone < 0 {
            return Err(false);
        }
        Ok(tone as u8)
    }



    pub(crate) fn frequency(&self, period_shift: i16, semitone: bool) -> f32 {
        // let period = 10.0 * 12.0 * 16.0 * 4.0 - ((self.note - period_shift) * 16.0 * 4.0)  - self.finetune / 2.0;
        // if semitone {
            let period = self.period as i16 - (period_shift * 16 * 4) as i16;
        // }

        // LinearTables.dPeriod2HzTab[period as usize] as f32
        let two = 2.0f32;
        let freq = 8363.0 * two.powf((6 * 12 * 16 * 4 - period) as f32 / (12 * 16 * 4) as f32);
        return freq
    }

    const NOTES: [&'static str;12] = ["C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-"];

    pub(crate) fn to_string(&self) -> String {
        if self.note == 97 || self.note == 0 { (self.note as u32).to_string() } else {
            format!("{}{}", Self::NOTES[((self.note as u8 - 1) % 12) as usize], (((self.note as u8 - 1) / 12) + '0' as u8) as char )
        }
    }
}

#[derive(Clone,Copy,Debug)]
pub(crate) struct Volume {
    pub(crate) volume:         u8,
    pub(crate) volume_shift:   i32,
    pub(crate) output_volume:  f32,
    pub(crate) fadeout_vol:    i32,
    pub(crate) fadeout_speed:  i32,
    pub(crate) envelope_vol:   i32,
    pub(crate) global_vol:     i32,
}

impl Volume {
    pub(crate) fn new() -> Volume {
        Volume {
            volume: 0,
            volume_shift: 0,
            output_volume: 1.0,
            fadeout_vol: 65536,
            fadeout_speed: 0,
            envelope_vol: 0,
            global_vol: 0
        }
    }

    pub(crate) fn get_volume(&self) -> u8 {
        let outvol = self.volume as i32 + self.volume_shift;
        if outvol > 64 {64} else if outvol < 0 {0} else {outvol as u8}
    }

    pub(crate) fn retrig(&mut self, vol: i32) {
        self.set_volume(vol);
        self.volume_shift  = 0;
        self.fadeout_speed = 0;
        self.fadeout_vol = 65536;
        self.fadeout_speed = 0;
    }

    pub(crate) fn set_volume(&mut self, vol: i32) {
        self.volume = if vol > 64 { 64 } else if vol < 0 { 0 } else { vol } as u8;
    }
}


#[derive(Clone,Copy,Debug)]
pub(crate) struct Panning {
    pub(crate) panning:               u8,
    pub(crate) final_panning:         u8,
}

impl Panning {
    pub(crate) fn new() -> Panning {
        Panning {
            panning: 0x80,
            final_panning: 128,
        }
    }

    pub(crate) fn get_panning(&self) -> u8 {
        // let outvol = self.volume as i32 + self.volume_shift;
        // if outvol > 64 {64} else if outvol < 0 {0} else {outvol as u8}
        0x80
    }

    pub(crate) fn set_panning(&mut self, panning: i32) {
        self.panning = clamp(panning , 0, 255) as u8;
    }

    pub(crate) fn update_envelope_panning(&mut self, envelope_panning: u16) {
        self.final_panning = clamp(self.panning as i32 + (envelope_panning as i32-32*256)*(128 - (self.panning as i32 - 128).abs()) / (32i32 * 128i32), 0 ,255) as u8;
        // self.panning = clamp(self.panning , 0, 255) as u8;
    }
}
