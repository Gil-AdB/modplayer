use crate::channel_state::channel_state::{EnvelopeState, Note, PortaToNoteState, TremoloState, VibratoState, Volume, clamp, Panning};
use crate::instrument::{Instrument, Sample};
use crate::pattern::Pattern;
use crate::xm_reader::is_note_valid;

pub(crate) mod channel_state;

#[derive(Clone,Copy,Debug)]
pub(crate) struct ChannelState<'a> {
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
    pub(crate) sustained:                      bool,
    pub(crate) vibrato_state:                  VibratoState,
    pub(crate) tremolo_state:                  TremoloState,
    pub(crate) frequency_shift:                f32,
    pub(crate) period_shift:                   i16,
    pub(crate) on:                             bool,
    pub(crate) last_porta_up:                  u16,
    pub(crate) last_porta_down:                u16,
    pub(crate) last_volume_slide:              u8,
    pub(crate) last_fine_volume_slide_up:      u8,
    pub(crate) last_fine_volume_slide_down:    u8,
    pub(crate) porta_to_note:                  PortaToNoteState,
    pub(crate) last_sample_offset:             u32,
    pub(crate) last_panning_speed:             u8,
    pub(crate) panning:                        Panning,
}

impl ChannelState<'_> {
    fn set_note(&mut self, note: u8, fine_tune: i8) {
        self.note.set_note(note, fine_tune);
        self.frequency_shift = 0.0;
        self.period_shift = 0;
        self.frequency = self.note.frequency(self.period_shift, false);
    }

    pub(crate) fn key_off(&mut self) -> bool {
        self.sustained = false;
        if !self.instrument.volume_envelope.on {
            self.on = false;
            self.volume.retrig(0);
            return false;
        }
        self.volume.fadeout_speed = self.instrument.volume_fadeout as i32;
        return true;
    }

    // // for arpeggio and portamento (semitone-slide mode)
    // fn relocateTon(&self, period: u16, arpNote: u8) -> u16 {
    //     // int32_t fineTune, loPeriod, hiPeriod, tmpPeriod, tableIndex;
    //
    //     let fineTune : i32 = (((self.sample.finetune >> 3) + 16) << 1) as i32;
    //     let mut hiPeriod = (8 * 12 * 16) * 2;
    //     let mut loPeriod = 0;
    //
    //     let note2Period = Ok(Tables.load());
    //
    //     for i in 0..8 {
    //         let tmpPeriod = (((loPeriod + hiPeriod) >> 1) & 0xFFFFFFE0) + fineTune;
    //
    //         let mut tableIndex = (tmpPeriod - 16) as u32 >> 1;
    //         tableIndex = clamp(tableIndex, 0, 1935); // 8bitbubsy: added security check
    //
    //         if period >= note2Period[tableIndex] {
    //             hiPeriod = (tmpPeriod - fineTune) & 0xFFFFFFE0;
    //         } else {
    //             loPeriod = (tmpPeriod - fineTune) & 0xFFFFFFE0;
    //         }
    //     }
    //
    //     // arpNote is between 0..16
    //     let mut tmpPeriod = loPeriod + fineTune + (arpNote << 5) as i32;
    //
    //     if tmpPeriod < 0 {// 8bitbubsy: added security check
    //         tmpPeriod = 0;
    //     }
    //
    //     if tmpPeriod >= (8*12*16+15)*2-1 { // FT2 bug: off-by-one edge case
    //         tmpPeriod = (8 * 12 * 16 + 15) * 2;
    //     }
    //
    //     return note2Period[tmpPeriod>>1];
    // }



    pub(crate) fn update_frequency(&mut self, rate: f32) {
        // self.frequency = self.note.frequency(self.period_shift) + self.frequency_shift;
        self.frequency = self.note.frequency(self.period_shift, false) + self.frequency_shift;
        self.du = self.frequency / rate;
    }

    pub(crate) fn reset_envelopes(&mut self) {
        self.volume_envelope_state.reset(0, &self.instrument.volume_envelope);
        self.panning_envelope_state.reset(0, &self.instrument.panning_envelope);
    }


    pub(crate) fn trigger_note(&mut self, pattern: &Pattern, rate: f32) {
        if pattern.note >= 1 && pattern.note < 97 { // trigger note

            let tone = match Note::get_tone(pattern.note, self.sample.relative_note) {
                Ok(p) => p,
                Err(_e) => return,
            };

            self.on = true;
            self.sample_position = 0.0;
            self.loop_started = false;
            self.ping = true;
            self.frequency_shift = 0.0;
            self.period_shift = 0;

            // println!("channel_state: {}, note: {}, relative: {}, real: {}, vol: {}", i, pattern.note, self.sample.relative_note, pattern.note as i8 + self.sample.relative_note, self.volume);

            self.set_note(tone, self.sample.finetune);
            self.update_frequency(rate);
            self.sustained = true;
            self.reset_envelopes();
        }
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
            0 => {self.period_shift = 0;}
            1 => {self.period_shift = x as i16;}
            2 => {self.period_shift = y as i16;}
            _ => {}
        }
    }

    pub(crate) fn panning_slide(&mut self, first_tick: bool, param: u8) {
        if first_tick {
            if param != 0 {
                self.last_panning_speed = param;
            }
        } else {
            let up = self.last_panning_speed >> 4;
            let down = self.last_panning_speed & 0xf;
            if up != 0 {
                self.panning_inner(first_tick, up as i8);
            } else if down != 0 {
                self.panning_inner(first_tick, - (down as i8));
            }
        }
    }

    fn panning_inner(&mut self, first_tick: bool, panning: i8) {
        if !first_tick {
            let new_panning = self.panning.panning as i32 + panning as i32;
            self.panning.set_panning(new_panning);
        }
    }


    pub(crate) fn set_volume(&mut self, first_tick: bool, volume: u8) {
        if first_tick {
            self.volume.set_volume(volume as i32);
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
        let new_volume = self.volume.volume as i32  + volume as i32;
        self.volume.set_volume(new_volume);
    }

    pub(crate) fn porta_to_note(&mut self, first_tick: bool, speed: u8, note: u8, rate: f32) {
        // let speed = pattern.effect_param;

        if first_tick {
            if speed != 0 {
                self.porta_to_note.speed = speed;
            }

            if is_note_valid(note) {

                self.porta_to_note.target_note.set_note(clamp(note as i8 + self.sample.relative_note, 0, 119) as u8, self.sample.finetune);
            }

        } else {
            let mut up = true;
            if self.note.period < self.porta_to_note.target_note.period {
                self.note.period += (self.porta_to_note.speed as u16 * 4) as u16;
                up = true;

            } else if self.note.period > self.porta_to_note.target_note.period {
                self.note.period -= (self.porta_to_note.speed as u16 * 4) as u16;
                up = false;
            }

            if up {
                if self.note.period > self.porta_to_note.target_note.period {
                    self.note = self.porta_to_note.target_note;
                    self.period_shift = 0;
                    self.frequency_shift = 0.0;
                }
            } else if self.note.period < self.porta_to_note.target_note.period {
                self.note = self.porta_to_note.target_note;
                self.period_shift = 0;
                self.frequency_shift = 0.0;
            }


            self.update_frequency(rate);
        }
    }

    pub(crate) fn porta_up(&mut self, first_tick: bool, amount: u8, rate: f32) {
        if first_tick {
            if amount != 0 {
                self.last_porta_up = (amount * 4) as u16;
            }
        } else {
            self.note.period -= self.last_porta_up;
            if self.note.period < 1 {
                self.note.period = 1;
            }
            self.update_frequency(rate);
        }
    }

    pub(crate) fn porta_down(&mut self, first_tick: bool, amount: u8, rate: f32) {
        if first_tick {
            if amount != 0 {
                self.last_porta_down = (amount * 4) as u16;
            }
        } else {
            self.note.period += self.last_porta_down;
            if self.note.period > 31999 {
                self.note.period = 31999;
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
