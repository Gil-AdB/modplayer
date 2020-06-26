use crate::channel::channel_state::{EnvelopeState, Note, PortaToNoteState, TremoloState, VibratoState, Volume};
use crate::instrument::{Instrument, Sample};
use crate::pattern::Pattern;
use crate::xm_reader::is_note_valid;

pub(crate) mod channel_state;

#[derive(Clone,Copy,Debug)]
pub(crate) struct Channel<'a> {
    pub(crate) instrument:                     &'a Instrument,
    pub(crate) sample:                         &'a Sample,
    pub(crate) note:                           Note,
    pub(crate) frequency:                      f32,
    pub(crate) du:                             f32,
    pub(crate) volume:                         Volume,
    pub(crate) sample_position:                f32,
    pub(crate) loop_started:                   bool,
    pub(crate) ping:                           bool,
    pub(crate) volume_envelope_state:          EnvelopeState,
    pub(crate) panning_envelope_state:         EnvelopeState,
    pub(crate) fadeout_vol:                    u16,
    pub(crate) sustained:                      bool,
    pub(crate) vibrato_state:                  VibratoState,
    pub(crate) tremolo_state:                  TremoloState,
    pub(crate) frequency_shift:                f32,
    pub(crate) period_shift:                   f32,
    pub(crate) on:                             bool,
    pub(crate) last_porta_up:                  f32,
    pub(crate) last_porta_down:                f32,
    pub(crate) last_volume_slide:              u8,
    pub(crate) last_fine_volume_slide_up:      u8,
    pub(crate) last_fine_volume_slide_down:    u8,
    pub(crate) porta_to_note:                  PortaToNoteState,
    pub(crate) last_sample_offset:             u32,
}

impl Channel<'_> {
    fn set_note(&mut self, note: i16, fine_tune: i32) {
        self.note.set_note(note as f32, fine_tune as f32);
        self.frequency_shift = 0.0;
        self.period_shift = 0.0;
        self.frequency = self.note.frequency(self.period_shift);
    }

    pub(crate) fn key_off(&mut self) -> bool {
        self.sustained = false;
        if !self.instrument.volume_envelope.on {
            self.on = false;
            return false;
        }
        return true;
    }


    pub(crate) fn update_frequency(&mut self, rate: f32) {
        // self.frequency = self.note.frequency(self.period_shift) + self.frequency_shift;
        self.frequency = self.note.frequency(self.period_shift) + self.frequency_shift;
        self.du = self.frequency / rate;
    }

    pub(crate) fn reset_envelopes(&mut self) {
        self.volume_envelope_state.reset(0, &self.instrument.volume_envelope);
        self.panning_envelope_state.reset(0, &self.instrument.panning_envelope);
        self.fadeout_vol = 65535;
    }


    pub(crate) fn trigger_note(&mut self, pattern: &Pattern, rate: f32) {
        let mut reset_envelope = false;
        if pattern.note >= 1 && pattern.note < 97 { // trigger note

            let tone = match self.get_tone(pattern) {
                Ok(p) => p,
                Err(e) => return,
            };

            self.on = true;
            self.sample_position = 0.0;
            self.loop_started = false;
            self.ping = true;
            self.frequency_shift = 0.0;
            self.period_shift = 0.0;

            // println!("channel: {}, note: {}, relative: {}, real: {}, vol: {}", i, pattern.note, self.sample.relative_note, pattern.note as i8 + self.sample.relative_note, self.volume);

            self.set_note(tone as i16, self.sample.finetune as i32);
            self.update_frequency(rate);
            self.sustained = true;
            reset_envelope = true;
        }

        if reset_envelope {
            self.reset_envelopes();
        }
    }

    fn get_tone(&mut self, pattern: &Pattern) -> Result<i8, bool> {
        let tone = pattern.note as i8 + self.sample.relative_note;
        if tone > 12 * 10 || tone < 0 {
            return Err(false);
        }
        Ok(tone)
    }

    pub(crate) fn vibrato(&mut self, first_tick: bool, speed:u8, depth: u8) {
        if first_tick {
            self.vibrato_state.set_speed(speed as i8);
            self.vibrato_state.set_depth(depth as i8);
        } else {
            self.vibrato_state.next_tick();
        }
    }

    pub(crate) fn tremolo(&mut self, first_tick: bool, speed:u8, depth: u8) {
        if first_tick {
            self.tremolo_state.set_speed(speed as i8);
            self.tremolo_state.set_depth(depth as i8);
        } else {
            self.tremolo_state.next_tick();
        }
    }


    pub(crate) fn arpeggio(&mut self, tick: u32, x:u8, y: u8) {
        match tick % 3 {
            0 => {self.period_shift = 0.0;}
            1 => {self.period_shift = x as f32;}
            2 => {self.period_shift = y as f32;}
            _ => {}
        }
    }


    pub(crate) fn set_volume(&mut self, first_tick: bool, volume: u8) {
        if first_tick {
            self.volume.retrig(volume as i32);
        }
    }

    pub(crate) fn volume_slide_main(&mut self, first_tick: bool, param: u8) {
        if first_tick {
            if param != 0 {
                self.last_volume_slide = param;
            }
        } else {
            let up = self.last_volume_slide >> 4;
            let down = self.last_volume_slide & 0xf;
            if up != 0 {
                self.volume_slide(first_tick, up as i8);
            } else if down != 0 {
                self.volume_slide(first_tick, - (down as i8));
            }
        }
    }

    pub(crate) fn fine_volume_slide_up(&mut self, first_tick: bool, speed: u8) {
        if first_tick {
            if speed != 0 {
                self.last_fine_volume_slide_up = speed;
            }
            self.volume_slide(first_tick, self.last_fine_volume_slide_up as i8);
        }
    }

    pub(crate) fn fine_volume_slide_down(&mut self, first_tick: bool, speed: u8) {
        if first_tick {
            if speed != 0 {
                self.last_fine_volume_slide_down = speed;
            }
            self.volume_slide(first_tick, -(self.last_fine_volume_slide_down as i8));
        }
    }


    pub(crate) fn volume_slide(&mut self, first_tick: bool, volume: i8) {
        if !first_tick { self.volume_slide_inner(volume);}
    }

    pub(crate) fn fine_volume_slide(&mut self, first_tick: bool, volume: i8) {
        if first_tick { self.volume_slide_inner(volume);}
    }

    fn volume_slide_inner(&mut self, volume: i8) {
        let mut new_volume = self.volume.volume as i32  + volume as i32;
        self.volume.retrig(new_volume);
    }

    pub(crate) fn porta_to_note(&mut self, first_tick: bool, speed: u8, note: u8, rate: f32) {
        // let speed = pattern.effect_param;

        if first_tick {
            if speed != 0 {
                self.porta_to_note.speed = speed;
            }

            if is_note_valid(note) {
                self.porta_to_note.target_note.set_note((note as i16 + self.sample.relative_note as i16) as f32, self.sample.finetune as f32);
            }

        } else {
            let mut up = true;
            if self.note.period < self.porta_to_note.target_note.period {
                self.note.period += self.porta_to_note.speed as f32 * 4.0;
                up = true;

            } else if self.note.period > self.porta_to_note.target_note.period {
                self.note.period -= self.porta_to_note.speed as f32 * 4.0;
                up = false;
            }

            if up {
                if self.note.period > self.porta_to_note.target_note.period {
                    self.note = self.porta_to_note.target_note;
                    self.period_shift = 0.0;
                    self.frequency_shift = 0.0;
                }
            } else if self.note.period < self.porta_to_note.target_note.period {
                self.note = self.porta_to_note.target_note;
                self.period_shift = 0.0;
                self.frequency_shift = 0.0;
            }


            self.update_frequency(rate);
        }
    }

    pub(crate) fn porta_up(&mut self, first_tick: bool, amount: u8, rate: f32) {
        if first_tick {
            if amount != 0 {
                self.last_porta_up = amount as f32 * 4.0;
            }
        } else {
            self.note.period -= self.last_porta_up;
            if self.note.period < 1.0 {
                self.note.period = 1.0;
            }
            self.update_frequency(rate);
        }
    }

    pub(crate) fn porta_down(&mut self, first_tick: bool, amount: u8, rate: f32) {
        if first_tick {
            if amount != 0 {
                self.last_porta_down = amount as f32 * 4.0;
            }
        } else {
            self.note.period += self.last_porta_down;
            if self.note.period > 31999.0 {
                self.note.period = 31999.0;
            }
            self.update_frequency(rate);
        }
    }


//     fn new(song_data : SongData) -> ChannelData {
//         ChannelData {
//             instrument: &song_data.instruments[0],
//             sample: &song_data.instruments[0].samples[0],
//             note: 0,
//             frequency: 0.0,
//             du: 0.0,
//             volume: 64,
//             output_volume: 1.0,
//             sample_position: 0.0,
//             loop_started: false,
//             volume_envelope_state: EnvelopeState::new(),
//             panning_envelope_state: EnvelopeState::new(),
//             fadeout_vol: 65535,
//             sustained: false,
//             vibrato_state: VibratoState::new(),
//             frequency_shift: 0.0,
//             on: false
//         }
//     }
}
