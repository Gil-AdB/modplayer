use std::cmp::min;
use std::io::{stdout, Write};
use std::ops::Generator;
use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::mpsc::Receiver;

use crossterm::cursor::{MoveTo, Show, Hide};

use crate::channel_state::ChannelState;
use crate::channel_state::channel_state::{EnvelopeState, Note, PortaToNoteState, TremoloState, VibratoState, Volume, WaveControl, Panning, clamp};
use crate::instrument::LoopType;
use crate::producer_consumer_queue::{AUDIO_BUF_FRAMES, AUDIO_BUF_SIZE};
use crate::xm_reader::{SongData, is_note_valid};
use crate::tables::PANNING_TAB;

struct BPM {
    pub bpm:                        u32,
    tick_duration_in_ms:        f32,
    tick_duration_in_frames:    usize,

}

impl BPM {
    fn new(bpm: u32, rate: f32) -> BPM {
        let mut ret = BPM{
            bpm: 0,
            tick_duration_in_ms: 0.0,
            tick_duration_in_frames: 0
        };
        ret.update(bpm, rate);
        ret
    }
    fn update(&mut self, bpm: u32, rate: f32) {
        if bpm > 999 || bpm < 1 {return};
        self.bpm = bpm;
        self.tick_duration_in_ms = 2500.0 / self.bpm as f32;
        self.tick_duration_in_frames = ((self.tick_duration_in_ms / 1000.0 * rate) + 0.5) as usize;
    }
}

struct PatternChange {
    pattern_break:  bool,
    pattern_jump:   bool,
    row:            u8,
    pattern:        u8,
}

impl PatternChange {
    pub fn new() -> Self {
        Self{
            pattern_break: false,
            pattern_jump: false,
            row: 0,
            pattern: 0
        }
    }
    fn reset(&mut self) {
        *self = Self::new();
    }

    fn set_break(&mut self, first_tick: bool, param:u8) {
        if !first_tick {return;}
        self.pattern_break = true;
        self.row = param;
        if self.row > 63 {self.row = 0;}
    }

    fn set_jump(&mut self, first_tick: bool, param:u8) {
        if !first_tick {return;}
        self.pattern_jump = true;
        self.pattern = param;
        self.row = 0;
    }
}


struct GlobalVolume {
    volume:                     u32,
    last_volume_slide:          u8,
}

impl GlobalVolume {
    pub fn new() -> Self {
        GlobalVolume { volume: 64, last_volume_slide: 0 }
    }

    fn volume_slide(&mut self, first_tick: bool, param: u8) {
        if first_tick {
            if param != 0 {
                self.last_volume_slide = param;
            }
        } else {
            let up = self.last_volume_slide >> 4;
            let down = self.last_volume_slide & 0xf;
            if up != 0 {
                self.handle_volume_slide(first_tick, up as i8);
            } else if down != 0 {
                self.handle_volume_slide(first_tick, - (down as i8));
            }
        }
    }

    fn handle_volume_slide(&mut self, first_tick: bool, volume: i8) {
        if !first_tick { self.volume_slide_inner(volume);}
    }

    // fn fine_volume_slide(&mut self, first_tick: bool, volume: i8) {
    //     if first_tick { self.volume_slide_inner(volume);}
    // }

    fn volume_slide_inner(&mut self, volume: i8) {
        let mut new_volume = self.volume as i32  + volume as i32;
        new_volume = if new_volume < 0 {0} else if volume > 64 { 64 } else { new_volume };
        self.volume = new_volume as u32;
    }

    fn set_volume(&mut self, first_tick: bool, volume: u8) {
        if first_tick {
            self.volume = if volume <= 0x40 { volume } else { 0x40 } as u32;
        }
    }

}

pub enum PlaybackCmd {
    IncBPM,
    DecBPM,
    IncSpeed,
    DecSpeed,
    Next,
    Prev,
    LoopPattern,
    Restart,
    Quit,
    PauseToggle,
    ChannelToggle(u8),
}


#[derive(Copy, Clone)]
struct RGB {
    R: u8,
    G: u8,
    B: u8,
}

// const BUFFER_SIZE: usize = 4096;
pub struct Song<'a> {
    song_position:              usize,
    row:                        usize,
    tick:                       u32,
    rate:                       f32,
    speed:                      u32,
    global_volume:              GlobalVolume,
    song_data:                  &'a SongData,
    channels:                   [ChannelState<'a>;32],
    pattern_change:             PatternChange,
    bpm:                        BPM,
    loop_pattern:               bool,
    pause:                      bool,
}

impl<'a> Song<'a> {
    // fn get_buffer(&mut self) -> Vec<f32> {
    //     let mut result: Vec<f32> = vec![];
    //     result.reserve_exact(BUFFER_SIZE);
    //     while result.len() < BUFFER_SIZE {
    //         if !self.internal_buffer.is_empty() {
    //             let copy_size = std::cmp::min(BUFFER_SIZE - result.len(), self.internal_buffer.len());
    //             result.extend(self.internal_buffer.drain(0..copy_size));
    //         }
    //         if !self.internal_buffer.is_empty() {
    //             return result;
    //         }
    //         self.get_next_tick();
    //     }
    //
    //     return result;
    // }

    pub fn new(song_data: &SongData, sample_rate: f32) -> Song {
        Song {
            song_position: 0,
            row: 0,
            tick: 0,
            rate: sample_rate,
            speed: song_data.tempo as u32,
            bpm: BPM::new(song_data.bpm as u32, sample_rate as f32),
            global_volume: GlobalVolume::new(),
            song_data: &song_data,
            channels: [ChannelState {
                instrument: &song_data.instruments[0],
                sample: &song_data.instruments[0].samples[0],
                note: Note::new(),
                frequency: 0.0,
                du: 0.0,
                volume: Volume::new(),
                sample_position: 0.0,
                loop_started: false,
                ping: true,
                volume_envelope_state: EnvelopeState::new(),
                panning_envelope_state: EnvelopeState::new(),
                sustained: false,
                vibrato_state: VibratoState::new(),
                tremolo_state: TremoloState::new(),
                frequency_shift: 0.0,
                period_shift: 0,
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
                panning: Panning::new(),
                force_off: false
            }; 32],
            loop_pattern: false,
            pattern_change: PatternChange::new(),
            pause: false
        }
    }

    // fn get_linear_frequency(note: i16, fine_tune: i32, period_offset: i32) -> f32 {
    //     let period = 10.0 * 12.0 * 16.0 * 4.0 - (note * 16 * 4) as f32  - (fine_tune as f32) / 2.0 + period_offset as f32;
    //     let two = 2.0f32;
    //     let frequency = 8363.0 * two.powf((6.0 * 12.0 * 16.0 * 4.0 - period) / (12.0 * 16.0 * 4.0));
    //     frequency as f32
    // }

    fn color(color: RGB, str: &str) -> String {
        format!("\x1b[38;2;{};{};{}m{}", color.R, color.G, color.B, str)
    }

    fn range(pos: u32, start: u32, end: u32, width: usize) -> String {
        let mut result : String = String::from("");
        let mut indicator_pos = ((pos - start) as f32 / (end - start) as f32 * (width) as f32) as usize;
        if indicator_pos > width {
            indicator_pos = width;
        }
        for i in 0..indicator_pos {
            result += "-";
        }
        result += "=";
        for i in indicator_pos+1..(width+1) as usize {
            result += "-";
        }
        result
    }

    fn range_with_color(pos: u32, start: u32, end: u32, width: usize, colors: &[RGB]) -> String {
        let mut result : String = String::from("");
        if pos == 0 {
            for i in 0..width+1 {
                result += " ";
            }
            return result;
        }

        let mut indicator_pos = ((pos - start) as f32 / (end - start) as f32 * (width) as f32) as usize;
        if indicator_pos > width {
            indicator_pos = width;
        }
        for i in 0..indicator_pos {
            result += &*Self::color(colors[i], "=");
        }
        result += &*Self::color(colors[indicator_pos], "=");
        for i in indicator_pos+1..(width+1) as usize {
            result += " "; //&*Self::color(colors[i], "-");
        }
        result += "\x1b[0m";
        result
    }


    fn display(&self, cur_tick: usize) {
        let colors: [RGB; 12] = [
            RGB {R:   0, G: 120, B:   0},
            RGB {R:   0, G: 140, B:   0},
            RGB {R:   0, G: 160, B:   0},
            RGB {R:   0, G: 180, B:   0},
            RGB {R: 180, G: 180, B:   0},
            RGB {R: 195, G: 195, B:   0},
            RGB {R: 210, G: 210, B:   0},
            RGB {R: 225, G: 225, B:   0},
            RGB {R: 225, G:  64, B:   0},
            RGB {R: 225, G:  64, B:   0},
            RGB {R: 225, G:  64, B:   0},
            RGB {R: 225, G:  64 , B:   0},
        ];
        let first_tick = self.tick == 0;
        if let Err(_e) = crossterm::execute!(stdout(), Hide, MoveTo(0,0)) {}
        println!("duration in frames: {:5} duration in ms: {:5} tick: {:3} pos: {:3X}/{:<3X}  row: {:3}/{:<3} bpm: {:3} speed: {:3}, buf: {}",
                 self.bpm.tick_duration_in_frames, self.bpm.tick_duration_in_ms, self.tick, self.song_position, self.song_data.song_length, self.row,
                 self.song_data.patterns[self.song_data.pattern_order[self.song_position] as usize].rows.len() - 1,
                 self.bpm.bpm, self.speed,
                 Song::range(cur_tick as u32, 0, self.bpm.tick_duration_in_frames as u32, 15),

        );
        if let Err(_e) = crossterm::execute!(stdout(), MoveTo(0,1)) {}

        println!("on | channel |         instrument         |frequency|   volume   |sample_position| note | period |  chan vol  |   envvol   | globalvol  |   fadeout  | panning |");

        let mut idx = 0u32;
        for channel in &self.channels {
            idx = idx + 1;
//            if idx != 1  {continue;}


            if channel.on {
                let final_vol =
                    (channel.volume.get_volume() as f32 / 64.0) *
                    (channel.volume.envelope_vol as f32 / 16384.0) *
                    (channel.volume.global_vol as f32 / 64.0) *
                    (channel.volume.fadeout_vol as f32 / 65536.0);

                println!("{:3}| {:7} | {:26} |  {:<6} |{:11}|{:14}| {:4} | {:7}|{:11}|{:11}|{:11}|{:11}|{:8}|      ",
                         if channel.force_off { " x" } else if channel.on { "on" } else { "off" }, idx, channel.instrument.idx.to_string() + ": " + channel.instrument.name.trim(),
                         if channel.on { (channel.frequency + channel.frequency_shift) as u32 } else { 0 },
                         Song::range_with_color((final_vol * 12.0) as u32, 0, 12, 11, &colors),
                         Song::range((channel.sample_position + channel.du * self.bpm.tick_duration_in_frames as f32) as u32, 0, channel.sample.length - 1, 14),
                         channel.note.to_string(), channel.note.period,
                         Song::range_with_color(channel.volume.get_volume() as u32, 0, 64, 11, &colors),
                         Song::range_with_color(channel.volume.envelope_vol as u32, 0, 16384, 11, &colors),
                         Song::range_with_color(channel.volume.global_vol as u32, 0, 64, 11, &colors),
                         Song::range_with_color(channel.volume.fadeout_vol as u32, 0, 65536, 11, &colors),
                         Song::range(channel.panning.final_panning as u32, 0, 255, 8),
                );
            } else {
                println!("{:3}| {:7} | {:26} |  {:<6} |{:12}| {:14}| {:5}| {:7}|{:12}|{:12}|{:12}|{:12}| {:8}|      ", "off", idx, "", "", "",
                         "", "", "", "", "", "", "", "");
            }
        }
        if let Err(_e) = crossterm::execute!(stdout(), Show) {}

    }

    pub fn get_next_tick_callback(&'a mut self, buffer: Arc<AtomicPtr<[f32; AUDIO_BUF_SIZE]>>, rx: Receiver<PlaybackCmd>) -> impl Generator<Yield=(), Return=()> + 'a {
        move || {
            let mut current_buf_position = 0;
            self.bpm.update(self.bpm.bpm, self.rate);
            let mut buf = &mut unsafe { *buffer.load(Ordering::Acquire) };
            loop {
                if !self.handle_commands(&rx) {return;}

                if self.pause {
                    //let temp_buf = &mut unsafe { *buffer.load(Ordering::Acquire) };
                    unsafe { buf = &mut *buffer.load(Ordering::Acquire); }
                    buf.fill(0.0);

                    current_buf_position = 0;
                    yield;
                    continue
                }

                self.process_tick();

                self.display(0);

//            self.internal_buffer.resize((tick_duration_in_frames * 2) as usize, 0.0);

                let mut current_tick_position = 0usize;

                while current_tick_position < self.bpm.tick_duration_in_frames {
                    let ticks_to_generate = min(self.bpm.tick_duration_in_frames - current_tick_position, AUDIO_BUF_FRAMES - current_buf_position);

                    // if let Err(_e) = crossterm::execute!(stdout(), MoveTo(0,1)) {}
                    self.output_channels(current_buf_position, buf, ticks_to_generate);
                    current_tick_position += ticks_to_generate;
                    current_buf_position += ticks_to_generate;
                    // println!("tick: {}, buf: {}, row: {}", self.tick, current_buf_position, self.row);
                    if current_buf_position == AUDIO_BUF_FRAMES {
                        // println!("Yielding: {}", current_buf_position);
                        yield;
                        //let temp_buf = &mut unsafe { *buffer.load(Ordering::Acquire) };
                        unsafe { buf = &mut *buffer.load(Ordering::Acquire); }
                        buf.fill(0.0);

                        current_buf_position = 0;
                    } else {
                        // We finished current with the current tick, but buffer is still not full...
                    }

                }

                if !self.next_tick() {return;}
            }
        }
    }

    fn handle_commands(&mut self, rx: & Receiver<PlaybackCmd>) -> bool {
        loop {
            if let Ok(cmd) = rx.try_recv() {
                match cmd {
                    PlaybackCmd::Quit => {
                        return false;
                    }
                    PlaybackCmd::Next => {
                        if self.song_position < (self.song_data.song_length - 1) as usize {
                            self.song_position += 1;
                            self.row = 0;
                            self.tick = 0;
                        }
                    }
                    PlaybackCmd::Prev => {
                        if self.song_position > 0 as usize {
                            self.song_position -= 1;
                            self.row = 0;
                            self.tick = 0;
                        }
                    }
                    PlaybackCmd::Restart => {
                        self.row = 0;
                        self.tick = 0;
                    }
                    PlaybackCmd::IncBPM => {self.bpm.update(self.bpm.bpm + 1, self.rate);}
                    PlaybackCmd::DecBPM => {self.bpm.update(self.bpm.bpm - 1, self.rate);}
                    PlaybackCmd::IncSpeed => {self.speed += 1;}
                    PlaybackCmd::DecSpeed => {self.speed -= 1;}
                    PlaybackCmd::LoopPattern => {self.loop_pattern = !self.loop_pattern}
                    PlaybackCmd::PauseToggle => {self.pause = !self.pause}
                    PlaybackCmd::ChannelToggle(channel) => {self.channels[channel as usize].force_off = !self.channels[channel as usize].force_off;}
                }
            }
            else
            {
                break;
            }

        }
        return true;
    }

    fn next_tick(&mut self) -> bool {
        self.tick += 1;
        if self.tick >= self.speed {
            if self.pattern_change.pattern_break || self.pattern_change.pattern_jump {
                if !self.pattern_change.pattern_jump {
                    self.next_pattern();
                } else {
                    self.song_position = self.pattern_change.pattern as usize;
                }
                self.row = self.pattern_change.row as usize;
            } else {
                self.row = self.row + 1;
                if self.row >= self.song_data.patterns[self.song_data.pattern_order[self.song_position as usize] as usize].rows.len() {
                    self.row = 0;
                    self.next_pattern();
                }
            }
            if self.song_position >= self.song_data.song_length as usize { self.song_position = self.song_data.restart_position as usize; }
            if self.song_position >= self.song_data.song_length as usize { return false; }
            self.tick = 0;
            self.pattern_change.reset();
        }
        true
    }

    fn next_pattern(&mut self) {
        if !self.loop_pattern {
            self.song_position = self.song_position + 1;
        }
    }

    fn process_tick(&mut self) {
        let instruments = &self.song_data.instruments;

        if self.song_position as usize >= self.song_data.pattern_order.len() {
            panic!("{} {}", self.song_position, self.song_data.song_length);
        }
        let patterns = &self.song_data.patterns[self.song_data.pattern_order[self.song_position] as usize];
        let row = &patterns.rows[self.row];
        let first_tick = self.tick == 0;

        let mut missing = String::new();
        for (i, pattern) in row.channels.iter().enumerate() {
            // if i != 12 { continue; }
            let mut channel = &mut self.channels[i];
            //let mut channel_state = &mut self.channels.split_at_mut(i).1[0];//channel_borrow_mut(i);

            if !channel.sustained {
                if channel.volume.fadeout_vol - channel.volume.fadeout_speed < 0 {
                    channel.volume.fadeout_vol = 0;
                } else {
                    channel.volume.fadeout_vol -= channel.volume.fadeout_speed;
                }
            }

            if first_tick && pattern.is_porta_to_note() && pattern.instrument != 0 {
                channel.volume.retrig(channel.sample.volume as i32);
            }

            if !pattern.is_porta_to_note() &&
                ((pattern.is_note_delay() && self.tick == pattern.get_y() as u32) ||
                    (!pattern.is_note_delay() && first_tick)) { // new row, set instruments


                if pattern.note == 97 { // note off
                    if !channel.key_off() {
                        continue;
                    }
                }

                if pattern.instrument != 0 {
                    let instrument = &instruments[pattern.instrument as usize];
                    channel.instrument = instrument;
                    if is_note_valid(pattern.note) {
                        channel.sample = &instrument.samples[instrument.sample_indexes[(pattern.note - 1)  as usize] as usize];
                    }
                    channel.volume.retrig(channel.sample.volume as i32);
                    channel.panning.panning = channel.sample.panning;
                }

                channel.frequency_shift = 0.0;
                channel.period_shift = 0;


                // let mut reset_envelope = false;
                if pattern.instrument != 0 {
                    channel.reset_envelopes();
                }

                channel.trigger_note(pattern, self.rate);
            }

            // handle vibrato
            if !first_tick && pattern.has_vibrato() { // vibrate
                channel.frequency_shift = channel.vibrato_state.get_frequency_shift(WaveControl::SIN) as f32;
                channel.update_frequency(self.rate);
            }

            // handle tremolo (not really need to do it here, but oh, well)
            if !first_tick && pattern.has_tremolo() { // tremolate
                channel.volume.volume_shift = channel.tremolo_state.get_volume_shift(WaveControl::SIN);
            }

            match pattern.volume {
                0x10..=0x50 => { channel.set_volume(first_tick, pattern.volume - 0x10); }       // set volume
                0x60..=0x6f => { channel.volume_slide(first_tick, -(pattern.get_volume_param() as i8)); }       // Volume slide down
                0x70..=0x7f => { channel.volume_slide(first_tick, pattern.get_volume_param() as i8); }    // Volume slide up
                0x80..=0x8f => { channel.fine_volume_slide(first_tick, -(pattern.get_volume_param() as i8)); }   // Fine volume slide down
                0x90..=0x9f => { channel.fine_volume_slide(first_tick, pattern.get_volume_param() as i8); } // Fine volume slide up
                0xa0..=0xaf => { channel.vibrato_state.set_speed((pattern.get_volume_param() * 4) as i8); } // Set vibrato speed (*4 is probably because S3M did this in order to support finer vibrato)
                0xb0..=0xbf => { channel.vibrato(first_tick, 0,pattern.get_volume_param()) } // Vibrato
                0xc0..=0xcf => { channel.panning.set_panning((pattern.get_volume_param() as i32) * 16);}// Set panning
                0xd0..=0xdf => { // Panning slide left
                    let pan = channel.panning.panning as i16 - pattern.get_volume_param() as i16;
                    if pattern.get_volume_param() == 0 || pan < 0 {
                        channel.panning.set_panning(0); // FT2 bug: param 0 = pan gets set to 0
                    } else {
                        channel.panning.set_panning(pan as i32);
                    }
                }
                0xe0..=0xef => { // Panning slide right
                    let pan = channel.panning.panning as i16 + pattern.get_volume_param() as i16;
                    if pattern.get_volume_param() > 255 {
                        channel.panning.set_panning(255);
                    } else {
                        channel.panning.set_panning(pan as i32);
                    }
                }
                0xf0..=0xff => {channel.porta_to_note(first_tick, pattern.volume & 0xf, pattern.note, self.rate); }// Tone porta

                _ => {}
            }


            // handle effects
            match pattern.effect {
                0x0 => {  // Arpeggio
                    if pattern.effect_param != 0 {
                        channel.arpeggio(self.tick, pattern.get_x(), pattern.get_y());
                        channel.update_frequency(self.rate);
                    }
                }
                0x1 => { channel.porta_up(first_tick, pattern.effect_param, self.rate); } // Porta up
                0x2 => { channel.porta_down(first_tick, pattern.effect_param, self.rate); } // Porta down
                0x3 => { channel.porta_to_note(first_tick,pattern.effect_param,  pattern.note, self.rate); } // Porta to note
                0x4 => { channel.vibrato(first_tick, pattern.get_x() * 4, pattern.get_y()); } // vibrato
                0x5 => { // porta to note + volume slide
                    channel.porta_to_note(first_tick, 0,0, self.rate);
                    channel.volume_slide_main(first_tick, pattern.effect_param);
                }
                0x6 => { // vibrato + volume slide
                    channel.vibrato(first_tick, 0, 0);
                    channel.volume_slide_main(first_tick, pattern.effect_param);
                }
                0x7 => {
                    channel.tremolo(first_tick, pattern.get_x() * 4, pattern.get_y());
                }
                0x8 => { // panning
                    channel.panning.set_panning(pattern.effect_param as i32);
                }
                0x9 => { // sample offset
                    if first_tick && pattern.instrument != 0 {
                        if pattern.effect_param != 0 {
                            channel.last_sample_offset = pattern.effect_param as u32 * 256;
                        }
                        channel.sample_position = channel.last_sample_offset as f32;
                    }
                }
                0xA => {
                    channel.volume_slide_main(first_tick, pattern.effect_param);
                }
                0xB => { // Pattern Jump
                    self.pattern_change.set_jump(first_tick, pattern.effect_param);
                }
                0xC => { channel.set_volume(first_tick, pattern.effect_param); } // set volume
                0xD => { // Pattern Break
                    self.pattern_change.set_break(first_tick, pattern.get_x() * 10 + pattern.get_y());
                }
                0xE => {} // handled separately
                0xF => { // set speed
                    if first_tick && pattern.effect_param > 0 {
                        if pattern.effect_param <= 0x1f {
                            self.speed = pattern.effect_param as u32;
                        } else {
                            self.bpm.update(pattern.effect_param as u32, self.rate);
                        }
                    }
                }

                0x10 => { // set global volume
                    self.global_volume.set_volume(first_tick, pattern.effect_param);
                }
                0x11 => { // global volume slide
                    self.global_volume.volume_slide(first_tick, pattern.effect_param);
                }
                0x14 => {
                    if self.tick == pattern.effect_param as u32 {
                        channel.key_off();
                    }
                }
                0x19 => {
                    channel.panning_slide(first_tick, pattern.effect_param);
                }
                _ => {missing.push_str(format!("channel: {}, eff: {:x},", i, pattern.effect).as_ref());}
            }

            if pattern.effect == 0xe {
                match pattern.get_x() {
                    0x1 => { channel.fine_porta_up(first_tick, pattern.get_y(), self.rate); } // Porta up
                    0x2 => { channel.fine_porta_down(first_tick, pattern.get_y(), self.rate); } // Porta down
                    0x8 => { channel.panning.set_panning((pattern.get_y() * 17) as i32);}
                    0x9 => { // retrig note
                        if !first_tick && (self.tick % pattern.get_y() as u32 == 0) {
                            channel.trigger_note(pattern, self.rate);
                        }
                    }
                    0xa => { channel.fine_volume_slide_up(first_tick, pattern.get_y());} // volume slide up
                    0xb => { channel.fine_volume_slide_down(first_tick, pattern.get_y());} // volume slide up
                    0xc => { channel.set_volume(self.tick == pattern.get_y() as u32, 0); }
                    0xd => {} // handled elsewhere
                    _ => {missing.push_str(format!("channel_state: {}, eff: 0xe{:x},", i, pattern.get_x()).as_ref());}
                }
            }



            let mut ves = channel.volume_envelope_state;


            let envelope_volume = channel.volume_envelope_state.handle(&channel.instrument.volume_envelope, channel.sustained, 64);

            // if i == 7 && self.song_position == 8 && channel.volume_envelope_state.sustained == false {
            //     let _test = ves.handle(&channel.instrument.volume_envelope, channel.sustained, 64);
            //     let _banana = 1;
            // }
            //
            // if self.song_position == 8 && i == 7 && envelope_volume == 0 {
            //     let _test = ves.handle(&channel.instrument.volume_envelope, channel.sustained, 64);
            //     let _banana = 1;
            // }

            // let envelope_volume1 = ves.handle1(&channel.instrument.volume_envelope, channel.sustained, 64);
            // if envelope_volume != envelope_volume1 {
            //     let banana = 1;
            // }
            let mut envelope_panning = channel.panning_envelope_state.handle(&channel.instrument.panning_envelope, channel.sustained, 32);
            // let scale = 0.9;
            envelope_panning = clamp(envelope_panning, 0, 64 * 256);


            channel.panning.update_envelope_panning(envelope_panning);
            // FinalVol = (FadeOutVol/65536)*(EnvelopeVol/64)*(GlobalVol/64)*(Vol/64)*Scale;
            // channel_state.update_frequency(self.rate);

            let global_volume = self.global_volume.volume as f32 / 64.0 ;
            channel.volume.envelope_vol = envelope_volume as i32;
            channel.volume.global_vol = self.global_volume.volume as i32;
            channel.volume.output_volume = (channel.volume.fadeout_vol as f32 / 65536.0) * (envelope_volume as f32 / 16384.0) * (channel.volume.get_volume() as f32 / 64.0) * global_volume;
            
        }
        if !missing.is_empty() {
            if let Err(_) = crossterm::execute!(stdout(), MoveTo(0,40)) {}
            println!("{:80}", missing);
        }

//            row
    }

    // fn channel_borrow_mut<'b>(&'b mut self, i: usize) -> &'b mut ChannelState<'a> {
    //     let channels = &mut (self.channels);
    //     let (_, r) = channels.split_at_mut(i);
    //     r[0].borrow_mut()
    // }

    // fn porta_inner(frequncy_shift: i8, channel_state: &mut ChannelData) {
    //     channel_state.frequency_shift += frequency_shift;
    // }



    fn output_channels(&mut self, current_buf_position: usize, buf: &mut [f32; AUDIO_BUF_SIZE], ticks_to_generate: usize) {
        // let mut  idx: u32 = 0;

        // let onecc = 1.0f32;// / cc as f32;
        // FT2 quirk: global volume is used at channel volume calculation time, not at mixing time
        //let global_volume = self.volume as f32 / 64.0 ;
        // println!("position: {:3}, row: {:3}", self.song_position, self.row);


        for channel in &mut self.channels {

            // idx = idx + 1;
//            if idx != 1  {continue;}
            if !channel.on || channel.force_off {
                continue;
            }

            // print!("channel_state: {}, instrument: {}, frequency: {}, volume: {}\n", idx, channel_state.instrument.name, channel_state.frequency, channel_state.volume);

            let vol_right = PANNING_TAB[      channel.panning.final_panning as usize] as f32 / 65536.0;
            let vol_left  = PANNING_TAB[256 - channel.panning.final_panning as usize] as f32 / 65536.0;
            for i in 0..ticks_to_generate as usize {

                if channel.sample_position as u32 >= channel.sample.length { // we could have this after set sample position
                    channel.on = false;
                    break;
                }

                buf[(current_buf_position + i) * 2 + 0] +=  vol_left * channel.sample.data[channel.sample_position as usize] as f32 / 32768.0  / 4.0 * channel.volume.output_volume;// * global_volume;
                buf[(current_buf_position + i) * 2 + 1] += vol_right * channel.sample.data[channel.sample_position as usize] as f32 / 32768.0  / 4.0 * channel.volume.output_volume;// * global_volume;

                // if (i & 63) == 0 {print!("{}\n", channel_state.sample_position);}
                if channel.sample.loop_type == LoopType::PingPongLoop && !channel.ping {
                    channel.sample_position -= channel.du;
                } else {
                    channel.sample_position += channel.du;
                }

                if channel.sample_position as u32 >= channel.sample.length ||
                    (channel.loop_started && channel.sample_position >= channel.sample.loop_end as f32) {
                    channel.loop_started = true;
                    match channel.sample.loop_type {
                        LoopType::PingPongLoop => {
                            channel.sample_position = (channel.sample.loop_end - 1) as f32 - (channel.sample_position - channel.sample.loop_end as f32);
                            channel.ping = false;
                            // channel_state.sample_position = (channel_state.sample.loop_end - 1) as f32;
                            // channel_state.du = -channel_state.du;
                        }
                        LoopType::NoLoop => {
                            channel.on = false;
                            channel.volume.set_volume(0);
                            break;
                        }
                        LoopType::ForwardLoop => {
                            channel.sample_position = (channel.sample_position - channel.sample.loop_end as f32) + channel.sample.loop_start as f32;
                        }
                    }
                }

                if channel.loop_started && channel.sample_position < channel.sample.loop_start as f32 {
                    match channel.sample.loop_type {
                        LoopType::PingPongLoop => {
                            channel.ping = true;
                        }
                        _ => {}
                    }
                    channel.sample_position = channel.sample.loop_start as f32 + (channel.sample.loop_start as f32 - channel.sample_position) as f32;
                }
            }
        }
    }
}
