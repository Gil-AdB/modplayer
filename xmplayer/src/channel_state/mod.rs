use crate::channel_state::channel_state::{clamp, EnvelopeState, Note, Panning, PortaToNoteState, TremoloState, VibratoState, Volume, VibratoEnvelopeState};
use crate::instrument::Instruments;
use crate::tables::AudioTables;
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
pub(crate) struct Voice {
    pub(crate) instrument:                     usize,
    pub(crate) sample:                         usize,
    pub(crate) frequency:                      f32,
    pub(crate) du:                             f32,
    pub(crate) volume:                         Volume,
    pub(crate) sample_position:                f32,
    pub(crate) loop_started:                   bool,
    pub(crate) ping:                           bool,
    pub(crate) sustained:                      bool,
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
    
    pub(crate) on:                             bool,
    pub(crate) channel_idx:                    usize, // The logical channel that "owns" or "started" this voice
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
            on: false,
            channel_idx: 0,
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

    pub(crate) fn update_envelopes(&mut self, instruments: &Instruments) {
        let instrument = &instruments[self.instrument];
        
        let envelope_volume = self.volume_envelope_state.handle(&instrument.volume_envelope, self.sustained, 64, false);
        let envelope_panning = self.panning_envelope_state.handle(&instrument.panning_envelope, self.sustained, 32, true);
        // let envelope_pitch = self.pitch_envelope_state.handle(&instrument.pitch_envelope, self.sustained, 0, false);

        self.panning.update_envelope_panning(envelope_panning);
        self.volume.envelope_vol = envelope_volume as i32;
    }

    pub(crate) fn update_output_volume(&mut self, global_volume: f32) {
        if !self.sustained {
            if self.volume.fadeout_vol - self.volume.fadeout_speed * 2 < 0 {
                self.volume.fadeout_vol = 0;
            } else {
                self.volume.fadeout_vol -= self.volume.fadeout_speed * 2;
            }
        }

        self.volume.output_volume = (self.volume.fadeout_vol as f32 / 65536.0) * 
                                    (self.volume.envelope_vol as f32 / 16384.0) * 
                                    (self.volume.get_volume() as f32 / 64.0) * 
                                    global_volume;
    }

    pub(crate) fn trigger_note(&mut self, instruments: &Instruments) {
        self.sample_position = 0.0;
        self.loop_started = false;
        self.ping = true;
        self.sustained = true;
        self.on = true;
        
        self.volume.fadeout_vol = 65536;
        
        let instrument = &instruments[self.instrument];
        self.volume_envelope_state.reset(0, &instrument.volume_envelope);
        self.panning_envelope_state.reset(0, &instrument.panning_envelope);
        self.pitch_envelope_state.reset(0, &instrument.pitch_envelope);
    }
}

#[derive(Clone,Copy,Debug)]
pub(crate) struct ChannelState {
    pub(crate) voice_idx:                      Option<usize>, // Which voice is currently "active" for this channel
    pub(crate) last_instrument:                usize,
    pub(crate) last_sample:                    usize,
    pub(crate) note:                           Note,
    pub(crate) frequency:                      f32,
    pub(crate) volume:                         Volume,
    pub(crate) panning:                        Panning,
    pub(crate) on:                             bool,
    pub(crate) last_porta_up:                  u16,
    pub(crate) last_porta_down:                u16,
    pub(crate) last_fine_porta_up:             u16,
    pub(crate) last_fine_porta_down:           u16,
    pub(crate) last_volume_slide:              u8,
    pub(crate) last_fine_volume_slide_up:      u8,
    pub(crate) last_fine_volume_slide_down:    u8,
    pub(crate) porta_to_note:                  PortaToNoteState,
    pub(crate) last_sample_offset:             u32,
    pub(crate) last_panning_speed:             u8,
    pub(crate) force_off:                      bool,
    pub(crate) glissando:                      bool,
    pub(crate) vibrato_control:                u8,
    pub(crate) tremolo_control:                u8,
    pub(crate) tremor:                         u8,
    pub(crate) tremor_count:                   u32,
    pub(crate) multi_retrig_count:             u8,
    pub(crate) multi_retrig_volume:            u8,
    pub(crate) period_shift:                   i16,
    pub(crate) last_played_note:               u8,
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
            last_samples: [0.0; 4096],
            last_samples_pos: 0,
        }
    }

    pub(crate) fn update_frequency_voice(&mut self, voice: &mut Voice, rate: f32, semitone: bool, frequency_tables: &AudioTables) {
        self.frequency = self.note.frequency(self.period_shift, semitone, frequency_tables) + voice.frequency_shift;
        voice.set_frequency(self.frequency, rate)
    }

    pub(crate) fn vibrato(&mut self, voice: Option<&mut Voice>, first_tick: bool, speed: u8, depth: u8) {
        if first_tick {
            if let Some(v) = voice {
                v.vibrato_state.set_speed(speed as i8);
                v.vibrato_state.set_depth(depth as i8);
            }
        } else {
            if let Some(v) = voice {
                v.vibrato_state.next_tick();
            }
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

    pub(crate) fn porta_up(&mut self, first_tick: bool, amount: u8) {
        if first_tick {
            if amount != 0 {
                self.last_porta_up = (amount as u16) * 4;
            }
        } else {
            self.note.period = (std::num::Wrapping(self.note.period) - std::num::Wrapping(self.last_porta_up)).0;
            if (self.note.period as i16) < 1 {
                self.note.period = 1;
            }
        }
    }

    pub(crate) fn porta_down(&mut self, first_tick: bool, amount: u8) {
        if first_tick {
            if amount != 0 {
                self.last_porta_down = (amount as u16) * 4;
            }
        } else {
            self.note.period += self.last_porta_down;
            if (self.note.period as i16) > 31999 {
                self.note.period = 31999;
            }
        }
    }

    pub(crate) fn fine_porta_up(&mut self, first_tick: bool, amount: u8) {
        if first_tick {
            if amount != 0 {
                self.last_fine_porta_up = (amount as u16) * 4;
            }
            self.note.period = (std::num::Wrapping(self.note.period) - std::num::Wrapping(self.last_fine_porta_up)).0;
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

    pub(crate) fn fine_porta_down(&mut self, first_tick: bool, amount: u8) {
        if first_tick {
            if amount != 0 {
                self.last_fine_porta_down = (amount as u16) * 4;
            }
            self.note.period += self.last_fine_porta_down;
            if (self.note.period as i16) > 31999 {
                self.note.period = 31999;
            }
        }
    }

    pub(crate) fn porta_to_note(&mut self, _voice: Option<&mut Voice>, first_tick: bool, amount: u8) {
        if first_tick {
            if amount != 0 {
                self.porta_to_note.speed = amount;
            }
        } else {
            self.porta_to_note.next_tick(&mut self.note);
        }
    }

    pub(crate) fn retrig(&mut self, voice: Option<&mut Voice>, instruments: &Instruments, tick: u32, amount: u8, volume_change: u8) {
        if amount == 0 { return; }
        if tick % (amount as u32) == 0 {
            if let Some(v) = voice {
                v.trigger_note(instruments);
                // Retrig volume logic simplified for now
                if volume_change > 0 {
                    // Implement volume change logic if needed
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

    pub(crate) fn panning_slide(&mut self, voice: Option<&mut Voice>, first_tick: bool, param: u8) {
        if first_tick {
            if param != 0 {
                self.last_panning_speed = param;
            }
        } else {
            let left = (self.last_panning_speed >> 4) as i32;
            let right = (self.last_panning_speed & 0xf) as i32;
            
            if let Some(v) = voice {
                let mut pan = v.panning.panning as i32;
                if left != 0 {
                    pan -= left;
                } else {
                    pan += right;
                }
                v.panning.set_panning(pan);
            }
        }
    }
}
