use crate::channel_state::channel_state::{clamp, EnvelopeState, Note, Panning, PortaToNoteState, TremoloState, VibratoState, Volume};
use crate::instrument::{Instrument, Sample};
use crate::pattern::Pattern;
use crate::tables::{AMIGA_PERIODS, LINEAR_PERIODS};
use crate::module_reader::{is_note_valid, SongData};
use crate::song::TableType;

pub(crate) mod channel_state;


#[derive(Clone,Copy,Debug)]
pub(crate) struct SplineData {
    p0: f32,
    p1: f32,
    p2: f32,
    p3: f32,
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
    
    pub fn push(&mut self, p: f32) {
        self.p0 = self.p1;
        self.p1 = self.p2;
        self.p2 = self.p3;
        self.p3 = p;
    }
}

#[derive(Clone,Copy,Debug)]
pub(crate) struct Voice<'a> {
    pub(crate) instrument:                     &'a Instrument,
    pub(crate) sample:                         &'a Sample,
    pub(crate) frequency:                      f32,
    pub(crate) du:                             f32,
    pub(crate) volume:                         Volume,
    pub(crate) sample_position:                f32,
    pub(crate) loop_started:                   bool,
    pub(crate) ping:                           bool,
    pub(crate) sustained:                      bool,
    pub(crate) spline_data:                    SplineData,
}

impl<'a> Voice<'a> {
    pub fn new(song_data: &'a SongData) -> Self {
        Self {
            instrument: &song_data.instruments[0],
            sample: &song_data.instruments[0].samples[0],
            frequency: 0.0,
            du: 0.0,
            volume: Volume::new(),
            sample_position: 0.0,
            loop_started: false,
            ping: true,
            sustained: false,
            spline_data: SplineData::new(),
        }
    }


    pub(crate) fn key_off(&mut self, is_note_delay: bool) -> bool {
        self.sustained = false;
        if !self.instrument.volume_envelope.on {
            // self.on = false;
            self.volume.retrig(0);

            if !is_note_delay {
                self.volume.fadeout_speed = self.instrument.volume_fadeout as i32;
            }

            if self.volume.fadeout_speed == 0 {
                self.volume.fadeout_vol = 0;
            }


            return false;
        }
        self.volume.fadeout_speed = self.instrument.volume_fadeout as i32;
        return true;
    }

    pub(crate) fn set_frequency(&mut self, frequency: f32, rate: f32) {
        self.frequency = frequency;
        self.du = self.frequency / rate;
    }

    pub(crate) fn trigger_note(&mut self) {
        self.sample_position = 0.0;
        self.loop_started = false;
        self.ping = true;
        self.sustained = true;
    }

}
#[derive(Clone,Copy,Debug)]
pub(crate) struct ChannelState<'a> {
    pub(crate) voice:                          Voice<'a>,
    pub(crate) note:                           Note,
    pub(crate) frequency:                      f32,
    pub(crate) volume_envelope_state:          EnvelopeState,
    pub(crate) panning_envelope_state:         EnvelopeState,
    pub(crate) vibrato_state:                  VibratoState,
    pub(crate) tremolo_state:                  TremoloState,
    pub(crate) frequency_shift:                f32,
    pub(crate) period_shift:                   i16,
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
    pub(crate) panning:                        Panning,
    pub(crate) force_off:                      bool,
    pub(crate) glissando:                      bool,
    pub(crate) vibrato_control:                u8,
    pub(crate) tremolo_control:                u8,
    pub(crate) tremor:                         u8,
    pub(crate) tremor_count:                   u32,
    // pub(crate) last_sample:                    i16,
    // pub(crate) last_sample_pos:                f32,
    pub(crate) last_played_note:               u8,
}

impl ChannelState<'_> {
    fn set_note(&mut self, note: u8, fine_tune: i8, use_amiga: TableType) {
        self.note.set_note(note, fine_tune, use_amiga);
        self.frequency_shift = 0.0;
        self.period_shift = 0;
        self.frequency = self.note.frequency(self.period_shift, false, use_amiga);
    }

    pub(crate) fn key_off(&mut self, is_note_delay: bool) -> bool {
        if self.voice.instrument.volume_envelope.on && self.volume_envelope_state.frame >= self.voice.instrument.volume_envelope.points[self.volume_envelope_state.idx].frame {
            if self.volume_envelope_state.idx > 0 {
                self.volume_envelope_state.frame = self.voice.instrument.volume_envelope.points[self.volume_envelope_state.idx].frame - 1;
                self.volume_envelope_state.idx -= 1;
            }
            self.volume_envelope_state.sustained = false;
        }

        self.voice.key_off(is_note_delay)
    }

    pub(crate) fn update_frequency(&mut self, rate: f32, semitone: bool, use_amiga: TableType) {
        // self.frequency = self.note.frequency(self.period_shift) + self.frequency_shift;
        // self.frequency = self.note.frequency(self.period_shift, semitone, use_amiga) + self.frequency_shift;
        // self.du = self.frequency / rate;
        self.voice.set_frequency(self.note.frequency(self.period_shift, semitone, use_amiga) + self.frequency_shift, rate)
    }

    pub(crate) fn reset_envelopes(&mut self) {
        self.voice.volume.fadeout_vol = 65536;
        self.voice.volume.fadeout_speed = 0;//self.instrument.volume_fadeout as i32;
        self.volume_envelope_state.reset(0, &self.voice.instrument.volume_envelope);
        self.panning_envelope_state.reset(0, &self.voice.instrument.panning_envelope);
        if self.vibrato_control & 0x4 != 4 {self.vibrato_state.set_pos(0);}
        if self.tremolo_control & 0x4 != 4 {self.tremolo_state.set_pos(0);}
    }


    pub(crate) fn trigger_note(&mut self, note: u8, rate: f32, use_amiga: TableType) {
        if note >= 1 && note < 97 { // trigger note

            let tone = match Note::get_tone(note, self.voice.sample.relative_note) {
                Ok(p) => p,
                Err(_e) => return,
            };

            self.on = true;
            self.voice.trigger_note();
            self.frequency_shift = 0.0;
            self.period_shift = 0;
            self.tremor_count = 0;
            // self.last_sample = 0;
            // self.last_sample_pos = 0.0;

            // println!("channel_state: {}, note: {}, relative: {}, real: {}, vol: {}", i, pattern.note, self.sample.relative_note, pattern.note as i8 + self.sample.relative_note, self.volume);

            self.set_note(tone, self.voice.sample.finetune, use_amiga);
            self.update_frequency(rate, false, use_amiga);
            // self.voice.sustained = true;
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

    pub(crate) fn multi_retrig(&mut self, tick: u32, param: u8) {
        // if tick == 0 {
        //     if param != 0 {
        //         self.multi_retrig_count = (param & 0xf0) >> 4;
        //         self.multi_retrig_vol = param & 0xf;
        //     }
        // }

        //
        // let x = (self.tremor & 0xf0 >> 4) as u32;
        // let y = (self.tremor & 0xf) as u32;
        // self.force_off = !(self.tremor_count <  x + 1);
        //
        // self.tremor_count += 1;
        // self.tremor_count = self.tremor_count % (x + 1 + y + 1);
    }

    pub(crate) fn tremor(&mut self, tick: u32, param: u8) {
        if tick == 0 {
            if param != 0 {
                self.tremor = param;
            }
        }

        let mut tremorPos = self.tremor_count;

        let mut tremorSign = (self.tremor_count & 0x80);
        let mut tremorData = (self.tremor_count & 0x7F) as i8;

        tremorData -= 1;
        if tremorData < 0
        {
            if tremorSign == 0x80
            {
                tremorSign = 0x00;
                tremorData = (self.tremor & 0xf) as i8;
            } else {
                tremorSign = 0x80;
                tremorData = (self.tremor >> 4) as i8;
            }
        }

        self.tremor_count = tremorSign | tremorData as u32;
        self.on = tremorSign == 0x80;

        // if tick == 0 {
        //     if param != 0 {
        //         self.tremor = param;
        //     }
        // }
        //
        //
        // let x = (self.tremor & 0xf0 >> 4) as u32;
        // let y = (self.tremor & 0xf) as u32;
        // self.on = (self.tremor_count <  x + 1);
        //
        // self.tremor_count += 1;
        // self.tremor_count = self.tremor_count % (x + 1 + y + 1);
    }

    fn panning_inner(&mut self, first_tick: bool, panning: i8) {
        if !first_tick {
            let new_panning = self.panning.panning as i32 + panning as i32;
            self.panning.set_panning(new_panning);
        }
    }


    pub(crate) fn set_volume(&mut self, first_tick: bool, volume: u8) {
        if first_tick {
            self.voice.volume.set_volume(volume as i32);
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
        let new_volume = self.voice.volume.volume as i32  + volume as i32;
        self.voice.volume.set_volume(new_volume);
    }

    pub(crate) fn porta_to_note(&mut self, first_tick: bool, speed: u8, note: u8, rate: f32, use_amiga: TableType) {
        // let speed = pattern.effect_param;

        if first_tick {
            if speed != 0 {
                self.porta_to_note.speed = speed;
            }

            if is_note_valid(note) {
                self.porta_to_note.target_note.set_note(clamp(note as i16 + self.voice.sample.relative_note as i16, 0, 119) as u8, self.voice.sample.finetune, use_amiga);
            }
        } else {
            let mut up = true;
            if self.note.period < self.porta_to_note.target_note.period {
                self.note.period += self.porta_to_note.speed as i16 * 4;
                up = true;
            } else if self.note.period > self.porta_to_note.target_note.period {
                self.note.period -= self.porta_to_note.speed as i16 * 4;
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

            self.update_frequency(rate, self.glissando, use_amiga);
        }
    }

    pub(crate) fn porta_up(&mut self, first_tick: bool, amount: u8, rate: f32, use_amiga: TableType) {
        if first_tick {
            if amount != 0 {
                self.last_porta_up = (amount as u16) * 4;
            }
        } else {
            self.note.period -= self.last_porta_up as i16;
            if self.note.period < 1 {
                self.note.period = 1;
            }
            self.update_frequency(rate, false, use_amiga);
        }
    }

    pub(crate) fn porta_down(&mut self, first_tick: bool, amount: u8, rate: f32, use_amiga: TableType) {
        if first_tick {
            if amount != 0 {
                self.last_porta_down = (amount as u16) * 4;
            }
        } else {
            self.note.period += self.last_porta_down as i16;
            if self.note.period > 31999 {
                self.note.period = 31999;
            }
            self.update_frequency(rate, false, use_amiga);
        }
    }

    pub(crate) fn fine_porta_up(&mut self, first_tick: bool, amount: u8, rate: f32, use_amiga: TableType) {
        if first_tick {
            if amount != 0 {
                self.last_fine_porta_up = (amount as u16) * 4;
            }
            self.note.period -= self.last_fine_porta_up as i16;
            if self.note.period < 1 {
                self.note.period = 1;
            }
            self.update_frequency(rate, false, use_amiga);
        }
    }

    pub(crate) fn fine_porta_down(&mut self, first_tick: bool, amount: u8, rate: f32, use_amiga: TableType) {
        if first_tick {
            if amount != 0 {
                self.last_fine_porta_down = (amount as u16) * 4;
            }
            self.note.period += self.last_fine_porta_down as i16;
            if self.note.period > 31999 {
                self.note.period = 31999;
            }
            self.update_frequency(rate, false, use_amiga);
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
