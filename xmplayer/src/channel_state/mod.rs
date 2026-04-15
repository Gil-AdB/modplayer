use crate::channel_state::channel_state::{clamp, EnvelopeState, Note, Panning, PortaToNoteState, TremoloState, VibratoState, Volume, VibratoEnvelopeState};
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
    pub(crate) instrument:                     usize,
    pub(crate) sample:                         usize,
    pub(crate) frequency:                      f32,
    pub(crate) du:                             f32,
    pub(crate) volume:                         Volume,
    pub(crate) sample_position:                f32,
    pub(crate) loop_started:                   bool,
    pub(crate) ping:                           bool,
    pub        sustained:                      bool,
    pub(crate) spline_data:                    SplineData,
    
    // Playback state moved from ChannelState
    pub(crate) volume_envelope_state:          EnvelopeState,
    pub(crate) panning_envelope_state:         EnvelopeState,
    pub(crate) pitch_envelope_state:           EnvelopeState,
    pub(crate) vibrato_envelope_state:         VibratoEnvelopeState,
    pub(crate) vibrato_state:                  VibratoState,
    pub(crate) tremolo_state:                  TremoloState,
    pub(crate) frequency_shift:                f32,
    pub(crate) panning:                        Panning,
    pub(crate) instrument_global_volume:       u8,
    pub(crate) sample_global_volume:           u8,
    pub(crate) filter_cutoff:                  u8,
    pub(crate) filter_resonance:               u8,
    pub(crate) filter_state:                   ResonantFilter,
    pub(crate) on:                             bool,
    pub(crate) surround:                       bool,
    pub(crate) channel_idx:                    usize, // The logical channel that "owns" or "started" this voice
    pub(crate) last_played_note:               u8,
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
            ping: true,
            sustained: false,
            spline_data: SplineData::new(),
            volume_envelope_state: EnvelopeState::new(),
            panning_envelope_state: EnvelopeState::new(),
            pitch_envelope_state: EnvelopeState::new(),
            vibrato_envelope_state: VibratoEnvelopeState::new(),
            vibrato_state: VibratoState::new(),
            tremolo_state: TremoloState::new(),
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
        }
    }


    pub(crate) fn key_off(&mut self, instruments: &Instruments, is_note_delay: bool) -> bool {
        let instrument = &instruments[self.instrument];
        self.sustained = false;
        self.volume_envelope_state.key_off(&instrument.volume_envelope);
        self.panning_envelope_state.key_off(&instrument.panning_envelope);
        self.pitch_envelope_state.key_off(&instrument.pitch_envelope);

        if !instrument.volume_envelope.on {
            // self.on = false;
            self.volume.retrig(0);

            if !is_note_delay {
                self.volume.fadeout_speed = instrument.volume_fadeout as i32;
            }

            if self.volume.fadeout_speed == 0 {
                self.volume.fadeout_vol = 0;
            }


            return false;
        }
        self.volume.fadeout_speed = instrument.volume_fadeout as i32;
        return true;
    }

    pub(crate) fn set_frequency(&mut self, frequency: f32, rate: f32) {
        self.frequency = frequency;
        self.du = self.frequency / rate;
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
        }

        self.update_filter(rate, final_cutoff.clamp(0, 127) as u8);
    }

    pub(crate) fn update_filter(&mut self, rate: f32, cutoff: u8) {
        if cutoff >= 127 {
            self.filter_state.a = 1.0;
            self.filter_state.b = 0.0;
            self.filter_state.c = 0.0;
            return;
        }

        // IT cutoff scale is roughly logarithmic. 
        // We'll map 0..127 to roughly 100Hz .. 10kHz
        let cutoff_freq = 110.0 * (2.0f32).powf(cutoff as f32 * 5.0 / 127.0);
        let p = 2.0 * (std::f32::consts::PI * cutoff_freq / rate).sin();
        let r = 1.0 - (self.filter_resonance as f32 / 128.0); // Simple damping

        // State Variable Filter coefficients
        // We'll store them in a, b, c for the mixing loop
        self.filter_state.a = p;
        self.filter_state.b = r;
    }

    pub(crate) fn update_output_volume(&mut self, global_volume: f32, channel_volume: f32, _divisor: f32) {
        if !self.sustained {
            if self.volume.fadeout_vol - self.volume.fadeout_speed < 0 {
                self.volume.fadeout_vol = 0;
            } else {
                self.volume.fadeout_vol -= self.volume.fadeout_speed;
            }
        }

        self.volume.output_volume = (self.volume.fadeout_vol as f32 / 65536.0) * 
                                    (self.volume.envelope_vol as f32 / 16384.0) * 
                                    (self.volume.get_volume() as f32 / 64.0) * 
                                    (self.instrument_global_volume as f32 / 64.0) *
                                    (self.sample_global_volume as f32 / 64.0) *
                                    channel_volume *
                                    global_volume;
    }

    pub(crate) fn trigger_note(&mut self, instruments: &Instruments) {
        self.sample_position = 4.0;
        self.loop_started = false;
        self.ping = true;
        self.sustained = true;
        self.on = true;
        
        self.volume.fadeout_vol = 65536;
        
        let instrument = &instruments[self.instrument];
        self.instrument_global_volume = instrument.global_volume;
        self.filter_cutoff = instrument.initial_filter_cutoff;
        self.filter_resonance = instrument.initial_filter_resonance;
        if self.sample < instrument.samples.len() {
            self.sample_global_volume = instrument.samples[self.sample].global_volume;
        }

        self.volume_envelope_state.reset(0, &instrument.volume_envelope);
        self.panning_envelope_state.reset(0, &instrument.panning_envelope);
        self.pitch_envelope_state.reset(0, &instrument.pitch_envelope);
    }
}

#[derive(Clone,Copy,Debug)]
pub struct ChannelState {
    pub voice_idx:                      Option<usize>, // Which voice is currently "active" for this channel
    pub last_instrument:                usize,
    pub last_sample:                    usize,
    pub note:                           Note,
    pub frequency:                      f32,
    pub volume:                         Volume,
    pub panning:                        Panning,
    pub on:                             bool,
    pub last_porta_up:                  u16,
    pub last_porta_down:                u16,
    pub last_fine_porta_up:             u16,
    pub last_fine_porta_down:           u16,
    pub channel_volume:                 u8,
    pub last_volume_slide:              u8,
    pub last_fine_volume_slide_up:      u8,
    pub last_fine_volume_slide_down:    u8,
    pub porta_to_note:                  PortaToNoteState,
    pub last_sample_offset:             u32,
    pub last_panning_speed:             u8,
    pub force_off:                      bool,
    pub(crate) glissando:                      bool,
    pub(crate) vibrato_control:                u8,
    pub(crate) tremolo_control:                u8,
    pub(crate) tremor:                         u8,
    pub(crate) tremor_count:                   u32,
    pub(crate) multi_retrig_count:             u8,
    pub(crate) multi_retrig_volume:            u8,
    pub(crate) period_shift:                   i16,
    pub(crate) last_played_note:               u8,
    pub(crate) last_it_slide_speed:            u8,
    pub(crate) last_it_vol_slide:              u8,
    pub(crate) last_samples:                   [f32; 4096],
    pub(crate) last_samples_pos:               usize,
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
            last_porta_up: 0,
            last_porta_down: 0,
            last_fine_porta_up: 0,
            last_fine_porta_down: 0,
            channel_volume: 64,
            last_volume_slide: 0,
            last_fine_volume_slide_up: 0,
            last_fine_volume_slide_down: 0,
            porta_to_note: PortaToNoteState::new(),
            last_sample_offset: 0,
            last_panning_speed: 0,
            force_off: false,
            glissando: false,
            vibrato_control: 0,
            tremolo_control: 0,
            tremor: 0,
            tremor_count: 0,
            multi_retrig_count: 0,
            multi_retrig_volume: 0,
            period_shift: 0,
            last_played_note: 0,
            last_it_slide_speed: 0,
            last_it_vol_slide: 0,
            last_samples: [0.0; 4096],
            last_samples_pos: 0,
        }
    }

    pub(crate) fn update_frequency_voice(&mut self, voice: &mut Voice, rate: f32, semitone: bool, frequency_tables: &AudioTables) {
        self.frequency = self.note.frequency(self.period_shift, semitone, frequency_tables) + voice.frequency_shift;
        voice.set_frequency(self.frequency, rate)
    }

    pub(crate) fn vibrato(&mut self, voice: Option<&mut Voice>, first_tick: bool, speed: u8, depth: u8, old_effects: bool, tables: &AudioTables) {
        if let Some(v) = voice {
            if first_tick {
                if speed != 0 {
                    v.vibrato_state.speed = speed as i8;
                }
                if depth != 0 {
                    let multiplier = if old_effects { 8 } else { 4 };
                    v.vibrato_state.depth = ((depth as u16) * multiplier) as i16;
                }
            } else {
                v.vibrato_state.next_tick();
            }
            self.update_frequency_voice(v, 0.0, true, tables);
        }
    }

    pub(crate) fn tremolo(&mut self, voice: Option<&mut Voice>, first_tick: bool, speed: u8, depth: u8) {
        if first_tick {
            if let Some(v) = voice {
                v.tremolo_state.set_speed(speed as i8);
                v.tremolo_state.set_depth(depth as i8);
            }
        } else {
            if let Some(v) = voice {
                v.tremolo_state.next_tick();
            }
        }
    }

    pub(crate) fn arpeggio(&mut self, tick: u32, x: u8, y: u8) {
        match tick % 3 {
            0 => { self.period_shift = 0; }
            1 => { self.period_shift = x as i16; }
            2 => { self.period_shift = y as i16; }
            _ => {}
        }
    }

    pub(crate) fn porta_up(&mut self, song_type: SongType, first_tick: bool, amount: u8) {
        if song_type == SongType::IT {
            if amount >= 0xF0 { // Extra Fine
                if first_tick {
                    let val = (amount & 0x0F) as u16;
                    self.note.period = self.note.period.saturating_sub(val);
                }
            } else if amount >= 0xE0 { // Fine
                if first_tick {
                    let val = ((amount & 0x0F) as u16) << 2;
                    self.note.period = self.note.period.saturating_sub(val);
                }
            } else { // Normal
                if !first_tick {
                    let val = (amount as u16) << 2;
                    self.note.period = self.note.period.saturating_sub(val);
                }
            }
        } else {
            if first_tick {
                if amount != 0 {
                    self.last_porta_up = (amount as u16) * 4;
                }
            } else {
                self.note.period = (std::num::Wrapping(self.note.period) - std::num::Wrapping(self.last_porta_up)).0;
            }
        }
        
        let min_period = if song_type == SongType::S3M || song_type == SongType::IT { 113 } else { 1 };
        if (self.note.period as i16) < min_period {
            self.note.period = min_period as u16;
        }
    }

    pub(crate) fn porta_down(&mut self, song_type: SongType, first_tick: bool, amount: u8) {
        if song_type == SongType::IT {
            if amount >= 0xF0 { // Extra Fine
                if first_tick {
                    let val = (amount & 0x0F) as u16;
                    self.note.period = self.note.period.saturating_add(val);
                }
            } else if amount >= 0xE0 { // Fine
                if first_tick {
                    let val = ((amount & 0x0F) as u16) << 2;
                    self.note.period = self.note.period.saturating_add(val);
                }
            } else { // Normal
                if !first_tick {
                    let val = (amount as u16) << 2;
                    self.note.period = self.note.period.saturating_add(val);
                }
            }
        } else {
            if first_tick {
                if amount != 0 {
                    self.last_porta_down = (amount as u16) * 4;
                }
            } else {
                self.note.period += self.last_porta_down;
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
                self.last_fine_porta_up = (amount as u16) * 4;
            }
            self.note.period = (std::num::Wrapping(self.note.period) - std::num::Wrapping(self.last_fine_porta_up)).0;
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
                self.last_fine_porta_down = (amount as u16) * 4;
            }
            self.note.period += self.last_fine_porta_down;
            let max_period = if song_type == SongType::S3M || song_type == SongType::IT { 27392 } else { 31999 };
            if self.note.period > max_period {
                self.note.period = max_period;
            }
        }
    }

    pub(crate) fn porta_to_note(&mut self, song_type: SongType, voice: Option<&mut Voice>, first_tick: bool, speed: u8, compatible_g: bool, tables: &AudioTables) {
        if first_tick {
            if speed != 0 {
                self.porta_to_note.speed = (speed as u16) * 4;
            }
        } else {
            self.porta_to_note.next_tick(&mut self.note);
        }
        if let Some(v) = voice {
            self.update_frequency_voice(v, 0.0, true, tables);
        }
    }

    pub(crate) fn it_volume_slide(&mut self, voice: Option<&mut Voice>, first_tick: bool, mut param: u8) {
        if param == 0 { param = self.last_it_vol_slide; }
        self.last_it_vol_slide = param;

        let x = param >> 4;
        let y = param & 0x0F;

        if x == 0x0F && y != 0 { // DFy: Fine Down
            self.fine_volume_slide(voice, first_tick, -(y as i8));
        } else if y == 0x0F && x != 0 { // DxF: Fine Up
            self.fine_volume_slide(voice, first_tick, x as i8);
        } else if x != 0 && y == 0 { // Dx0: Up
            self.volume_slide(voice, first_tick, x as i8);
        } else if y != 0 && x == 0 { // D0y: Down
            self.volume_slide(voice, first_tick, -(y as i8));
        }
    }

    pub(crate) fn it_retrig(&mut self, voice: Option<&mut Voice>, instruments: &Instruments, tick: u32, param: u8) {
        let y = param & 0x0F;
        let x = param >> 4;
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
                v.trigger_note(instruments);
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
        if first_tick {
            if param != 0 {
                self.last_volume_slide = param;
            }
        } else {
            let up = (self.last_volume_slide >> 4) as i32;
            let down = (self.last_volume_slide & 0xf) as i32;
            if let Some(v) = voice {
                if up != 0 {
                    v.volume.set_volume(v.volume.volume as i32 + up);
                } else {
                    v.volume.set_volume(v.volume.volume as i32 - down);
                }
            }
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

    pub(crate) fn panning_slide(&mut self, voice: Option<&mut Voice>, first_tick: bool, param: u8) {
        if first_tick {
            let right = (param >> 4) as i32;
            let left = (param & 0xf) as i32;
            if right == 0xf && left != 0 {
                if let Some(v) = voice {
                    v.panning.set_panning(v.panning.panning as i32 - (left << 2));
                }
            } else if left == 0xf && right != 0 {
                if let Some(v) = voice {
                    v.panning.set_panning(v.panning.panning as i32 + (right << 2));
                }
            }
        } else {
            let right = (param >> 4) as i32;
            let left = (param & 0xf) as i32;
            if let Some(v) = voice {
                if right != 0 && right != 0xf && left == 0 {
                    v.panning.set_panning(v.panning.panning as i32 + (right << 2));
                } else if left != 0 && left != 0xf && right == 0 {
                    v.panning.set_panning(v.panning.panning as i32 - (left << 2));
                }
            }
        }
    }
}
