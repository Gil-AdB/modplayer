use std::cmp::min;
use std::sync::mpsc::Receiver;

use crate::channel_state::{ChannelState, Voice};
use crate::channel_state::channel_state::{EnvelopeState, Note, PortaToNoteState, TremoloState, VibratoState, WaveControl, Panning, clamp, VibratoEnvelopeState};
use crate::instrument::{LoopType, Instrument};
use crate::module_reader::{SongData, is_note_valid, Patterns};
use crate::tables::{PANNING_TAB, AudioTables};
use crate::triple_buffer::{TripleBufferWriter, Init};
use std::collections::HashMap;
use std::num::Wrapping;
use std::borrow::Borrow;

struct BPM {
    pub bpm:                    u32,
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
        self.tick_duration_in_frames = (self.tick_duration_in_ms / 1000.0 * rate) as usize;
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

pub fn fill<T>(arr: &mut [T], value: T)
    where
        T: Clone,
{
    if let Some((last, elems)) = arr.split_last_mut() {
        for el in elems {
            el.clone_from(&value);
        }

        *last = value
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
    AmigaTable,
    LinearTable,
    PauseToggle,
    FilterToggle,
    DisplayToggle,
    ChannelToggle(u8),
    SetUserData(String, UserData),
    ModifyUserDataAddUSize(String, usize),
    ModifyUserDataSubUSize(String, usize),
    ModifyUserDataAddISize(String, isize),
    ModifyUserDataSubISize(String, isize),
    SpeedUp,
    SpeedDown,
    SpeedReset,
    SetPosition(u32),
}


#[derive(Clone)]
pub struct ChannelStatus {
    pub volume:                             f32,
    pub envelope_volume:                    f32,
    pub global_volume:                      f32,
    pub fadeout_volume:                     f32,
    pub on:                                 bool,
    pub force_off:                          bool,
    pub frequency:                          f32,
    pub instrument:                         usize,
    pub sample:                             usize,
    pub sample_position:                    f32,
    pub note:                               String,
    pub period:                             u16,
    pub final_panning:                      u8,
}

#[derive(Clone)]
pub struct PlayData {
    pub name:                               String,
    pub tick_duration_in_frames:            usize,
    pub tick_duration_in_ms:                f32,
    pub tick:                               u32,
    pub song_position:                      usize,
    pub song_length:                        u16,
    pub row:                                usize,
    pub pattern_len:                        usize,
    pub bpm:                                u32,
    pub speed:                              u32,
    pub channel_status:                     Vec<ChannelStatus>,
    pub filter:                             bool,
    pub user_data:                          HashMap<String, UserData>,
}

impl Init for PlayData {
    fn new() -> Self {
        Self{
            name: "".to_string(),
            tick_duration_in_frames: 0,
            tick_duration_in_ms: 0.0,
            tick: 0,
            song_position: 0,
            song_length: 1,
            row: 0,
            pattern_len: 1,
            bpm: 0,
            speed: 0,
            channel_status: vec![],
            filter: false,
            user_data: Default::default()
        }
    }
}

enum BufferState {
    Start,
    FillBuffer,
    NextTick,
}

struct TickState {
    state:                  BufferState,
    current_buf_position:   usize,
    current_tick_position:  usize,
}

pub enum CallbackState {
    Ok,
    Complete
}

pub trait BufferAdapter {
    fn mix_sample(&mut self, channel:usize, value: f32, pos: usize);
    fn clear(&mut self);
    fn len(&mut self) -> usize;
    fn num_frames(&mut self) -> usize;
    fn post_process(&mut self);
}

pub struct InterleavedBufferAdaptar<'a> {
    pub buf: &'a mut [f32],
}

impl BufferAdapter for InterleavedBufferAdaptar<'_> {
    fn mix_sample(&mut self, channel: usize, value: f32, pos: usize) {
        self.buf[pos * 2 + channel] += value;
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
    }

    fn len(&mut self) -> usize {
        return self.buf.len();
    }

    fn num_frames(&mut self) -> usize {
        self.len() / 2
    }

    fn post_process(&mut self) {}
}

pub struct PlanarBufferAdaptar<'a> {
    pub buf: [&'a mut [f32];2],
}

impl BufferAdapter for PlanarBufferAdaptar<'_> {
    fn mix_sample(&mut self, channel: usize, value: f32, pos: usize) {
        self.buf[channel][pos] += value;//(value - 0.5) * 2.0;
    }

    fn clear(&mut self) {
        self.buf[0].fill(0.0);
        self.buf[1].fill(0.0);
    }

    fn len(&mut self) -> usize {
        std::cmp::min(self.buf[0].len(), self.buf[1].len())
    }

    fn num_frames(&mut self) -> usize {
        self.len()
    }

    fn post_process(&mut self) {
        Self::normalize_array(self.buf[0]);
        Self::normalize_array(self.buf[1]);
    }
}

impl<'a> PlanarBufferAdaptar<'a> {
    fn normalize_array(buf: &mut [f32]) {
        for element in buf.iter_mut() {
            *element = (*element - 0.5f32) * 2.0f32;
        }
    }
}

#[derive(Clone, Debug)]
pub enum UserData {
    String(String),
    ISize(isize),
    USize(usize)
}

// const BUFFER_SIZE: usize = 4096;
pub struct Song {
    name:                       String,
    song_position:              usize,
    row:                        usize,
    tick:                       u32,
    rate:                       f32,
    original_rate:              f32,
    speed:                      u32,
    global_volume:              GlobalVolume,
    song_data:                  SongData,
    channels:                   Vec<ChannelState>,
    pattern_change:             PatternChange,
    bpm:                        BPM,
    loop_pattern:               bool,
    pause:                      bool,
    filter:                     bool,
    display:                    bool,
    frequency_tables:           Box<AudioTables>,
    triple_buffer_writer:       TripleBufferWriter<PlayData>,
    tick_state:                 TickState,
    user_data:                  HashMap<String, UserData>,
}

impl Song {
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

    pub fn new(song_data: &SongData, triple_buffer_writer: TripleBufferWriter<PlayData>, sample_rate: f32) -> Self {
        let use_amiga = if song_data.use_amiga {AudioTables::calc_tables_amiga()} else {AudioTables::calc_tables_linear()};
        Self {
            name: song_data.name.clone(),
            song_position: 0,
            row: 0,
            tick: 0,
            rate: sample_rate,
            original_rate: sample_rate,
            speed: song_data.tempo as u32,
            bpm: BPM::new(song_data.bpm as u32, sample_rate as f32),
            global_volume: GlobalVolume::new(),
            song_data: song_data.clone(),
            channels: vec![ChannelState {
                // instrument: &song_data.instruments[0],
                // sample: &song_data.instruments[0].samples[0],
                voice: Voice::new(),
                note: Note::new(),
                frequency: 0.0,
                // du: 0.0,
                // volume: Volume::new(),
                // sample_position: 0.0,
                // loop_started: false,
                // ping: true,
                volume_envelope_state: EnvelopeState::new(),
                panning_envelope_state: EnvelopeState::new(),
                // sustained: false,
                vibrato_envelope_state: VibratoEnvelopeState::new(),
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
                force_off: false,
                glissando: false,
                // last_sample: 0,
                // last_sample_pos: 0.0,
                vibrato_control: 0,
                tremolo_control: 0,
                tremor: 0,
                tremor_count: 0,
                multi_retrig_count: 0,
                multi_retrig_volume: 0,
                last_played_note: 0
            }; song_data.channel_count as usize],
            loop_pattern: false,
            pattern_change: PatternChange::new(),
            pause: false,
            filter: true,
            display: true,
            frequency_tables: use_amiga,
            triple_buffer_writer,
            tick_state: TickState {
                state: BufferState::Start,
                current_buf_position: 0,
                current_tick_position: 0
            },
            user_data: HashMap::new()
        }
    }


    // fn get_linear_frequency(note: i16, fine_tune: i32, period_offset: i32) -> f32 {
    //     let period = 10.0 * 12.0 * 16.0 * 4.0 - (note * 16 * 4) as f32  - (fine_tune as f32) / 2.0 + period_offset as f32;
    //     let two = 2.0f32;
    //     let frequency = 8363.0 * two.powf((6.0 * 12.0 * 16.0 * 4.0 - period) / (12.0 * 16.0 * 4.0));
    //     frequency as f32
    // }

    fn queue_display(&mut self) {
        let play_data = self.triple_buffer_writer.write();

        play_data.name                      = self.name.clone();
        play_data.tick_duration_in_frames   = self.bpm.tick_duration_in_frames;
        play_data.tick_duration_in_ms       = self.bpm.tick_duration_in_ms;
        play_data.tick                      = self.tick;
        play_data.song_position             = self.song_position;
        play_data.song_length               = self.song_data.song_length;
        play_data.row                       = self.row;
        play_data.pattern_len               = self.song_data.patterns[self.song_data.pattern_order[self.song_position] as usize].rows.len() - 1;
        play_data.bpm                       = self.bpm.bpm;
        play_data.speed                     = self.speed;
        play_data.channel_status.clear();
        play_data.filter                    = self.filter;
        play_data.user_data                 = self.user_data.clone();

        for channel in &self.channels {
            play_data.channel_status.push(ChannelStatus {
                volume:             channel.voice.volume.volume as f32,
                envelope_volume:    channel.voice.volume.envelope_vol as f32,
                global_volume:      channel.voice.volume.global_vol as f32,
                fadeout_volume:     channel.voice.volume.fadeout_vol as f32,
                on:                 channel.on,
                force_off:          channel.force_off,
                frequency:          channel.frequency + channel.frequency_shift,
                instrument:         channel.voice.instrument,
                sample:             channel.voice.sample,
                sample_position:    channel.voice.sample_position,
                note:               channel.note.to_string(),
                period:             channel.note.period,
                final_panning:      channel.panning.final_panning,
            });
        }
        // Song::display(&play_data, 0);
    }

    // pub fn get_next_tick(&mut self, buf: &mut [f32], rx: &mut Receiver<PlaybackCmd>) -> CallbackState {
    //     buf.fill(0.0);
    //     self.bpm.update(self.bpm.bpm, self.rate);
    //     // let mut buf = &mut unsafe { *buffer.load(Ordering::Acquire) };
    //     loop { // loop1
    //         match self.tick_state.state {
    //             BufferState::Start => {
    //                 if !self.handle_commands(rx) { return CallbackState::Complete; }
    //
    //                 if self.pause {
    //                     //let temp_buf = &mut unsafe { *buffer.load(Ordering::Acquire) };
    //
    //
    //                     self.tick_state.current_buf_position = 0;
    //                     return CallbackState::Ok;
    //                 }
    //
    //                 self.process_tick();
    //
    //                 if self.display {
    //                     self.queue_display();
    //                 }
    //
    //                 self.tick_state.current_tick_position = 0usize;
    //                 self.tick_state.state = BufferState::FillBuffer
    //             }
    //             BufferState::FillBuffer => {
    //                 while self.tick_state.current_tick_position < self.bpm.tick_duration_in_frames {
    //                     let ticks_to_generate = min(self.bpm.tick_duration_in_frames - self.tick_state.current_tick_position,
    //                                                 AUDIO_BUF_FRAMES - self.tick_state.current_buf_position);
    //
    //                     // if let Err(_e) = crossterm::execute!(stdout(), MoveTo(0,1)) {}
    //                     self.output_channels(self.tick_state.current_buf_position, buf, ticks_to_generate);
    //                     self.tick_state.current_tick_position += ticks_to_generate;
    //                     self.tick_state.current_buf_position += ticks_to_generate;
    //                     // println!("tick: {}, buf: {}, row: {}", self.tick, current_buf_position, self.row);
    //                     if self.tick_state.current_buf_position == AUDIO_BUF_FRAMES {
    //                         self.tick_state.current_buf_position = 0;
    //                         return CallbackState::Ok;
    //                     } else {
    //                         // We finished current with the current tick, but buffer is still not full...
    //                     }
    //                 }
    //                 self.tick_state.state = BufferState::NextTick
    //             }
    //             BufferState::NextTick => {
    //                 if !self.next_tick() { return CallbackState::Complete; }
    //                 self.tick_state.state = BufferState::Start
    //             }
    //         }
    //     }
    // }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.rate = sample_rate;
        self.original_rate = sample_rate;
    }

    pub fn get_instruments(&self) -> Vec<Instrument>{
        self.song_data.instruments.clone()
    }

    pub fn get_patterns(&self) -> Vec<Patterns> {
        self.song_data.patterns.clone()
    }

    pub fn get_order(&self) -> Vec<u8> {
        self.song_data.pattern_order.clone()
    }

    pub fn get_next_tick(&mut self, buf: &mut impl BufferAdapter, rx: &mut Receiver<PlaybackCmd>) -> CallbackState {
        buf.clear();
        self.bpm.update(self.bpm.bpm, self.rate);
        loop { // loop1
            match self.tick_state.state {
                BufferState::Start => {
                    if !self.handle_commands(rx) { return CallbackState::Complete; }

                    if self.pause {
                        self.tick_state.current_buf_position = 0;
                        return CallbackState::Ok;
                    }

                    self.process_tick();

                    if self.display {
                        self.queue_display();
                    }

                    self.tick_state.current_tick_position = 0usize;
                    self.tick_state.state = BufferState::FillBuffer
                }
                BufferState::FillBuffer => {
                    while self.tick_state.current_tick_position < self.bpm.tick_duration_in_frames {
                        let ticks_to_generate = min(self.bpm.tick_duration_in_frames - self.tick_state.current_tick_position,
                                                    buf.num_frames() - self.tick_state.current_buf_position);

                        self.output_channels(self.tick_state.current_buf_position, buf, ticks_to_generate);
                        self.tick_state.current_tick_position += ticks_to_generate;
                        self.tick_state.current_buf_position += ticks_to_generate;
                        if self.tick_state.current_buf_position == buf.num_frames() {
                            self.tick_state.current_buf_position = 0;
                            return CallbackState::Ok;
                        } else {
                            // We finished current with the current tick, but buffer is still not full...
                        }
                    }
                    self.tick_state.state = BufferState::NextTick
                }
                BufferState::NextTick => {
                    if !self.next_tick() { return CallbackState::Complete; }
                    self.tick_state.state = BufferState::Start
                }
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
                    PlaybackCmd::LoopPattern => {self.loop_pattern = !self.loop_pattern;}
                    PlaybackCmd::PauseToggle => {self.pause = !self.pause;}
                    PlaybackCmd::FilterToggle => {self.filter = !self.filter;}
                    PlaybackCmd::DisplayToggle => {self.display = !self.display;}
                    PlaybackCmd::ChannelToggle(channel) => {self.channels[channel as usize].force_off = !self.channels[channel as usize].force_off;}
                    PlaybackCmd::AmigaTable => {self.frequency_tables = AudioTables::calc_tables_amiga();}
                    PlaybackCmd::LinearTable => {self.frequency_tables = AudioTables::calc_tables_linear();}
                    PlaybackCmd::SetUserData(key, value) => {self.user_data.insert(key, value);}
                    PlaybackCmd::ModifyUserDataAddUSize(key, value) => {
                        let entry = self.user_data.entry(key).or_insert(UserData::USize(0));
                        if let UserData::USize(x) = entry {
                            *x = (Wrapping(*x) + Wrapping(value)).0;
                        }
                    }
                    PlaybackCmd::ModifyUserDataSubUSize(key, value) => {
                        let entry = self.user_data.entry(key).or_insert(UserData::USize(0));
                        if let UserData::USize(x) = entry {
                            *x = (Wrapping(*x) - Wrapping(value)).0;
                        }
                    }
                    PlaybackCmd::ModifyUserDataAddISize(key, value) => {
                        let entry = self.user_data.entry(key).or_insert(UserData::ISize(0));
                        if let UserData::ISize(x) = entry {
                            let res = (Wrapping(*x) + Wrapping(value)).0;
                            *entry = UserData::ISize(res);
                        }
                    }
                    PlaybackCmd::ModifyUserDataSubISize(key, value) => {
                        let entry = self.user_data.entry(key).or_insert(UserData::ISize(0));
                        if let UserData::ISize(x) = entry {
                            let res = (Wrapping(*x) - Wrapping(value)).0;
                            *entry = UserData::ISize(res);
                        }
                    }
                    PlaybackCmd::SpeedUp => {
                        self.rate /= 1.1;
                    }
                    PlaybackCmd::SpeedDown => {
                        self.rate *= 1.1;
                    }
                    PlaybackCmd::SpeedReset => {
                        self.rate = self.original_rate;
                    }
                    PlaybackCmd::SetPosition(order) => {
                        self.pattern_change.pattern = order as u8;
                        self.pattern_change.pattern_jump = true;
                        self.pattern_change.row = 0;
                        self.next_tick();
                    }
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
        if self.song_position >= self.song_data.song_length as usize {
            return false;
        }

        self.tick += 1;
        if self.tick >= self.speed {
            if self.pattern_change.pattern_break || self.pattern_change.pattern_jump {
                if !self.pattern_change.pattern_jump {
                    self.next_pattern();
                } else {
                    self.song_position = self.pattern_change.pattern as usize;
                    if self.song_position >=  self.song_data.song_length as usize {
                        return false;
                    }
                }
                self.row = self.pattern_change.row as usize;
            } else {
                self.row = self.row + 1;
                if self.row >= self.song_data.patterns[self.song_data.pattern_order[self.song_position as usize] as usize].rows.len() {
                    self.row = 0;
                    self.next_pattern();
                }
            }
            // if self.song_position >= self.song_data.song_length as usize { self.song_position = self.song_data.restart_position as usize; }
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
            let channel = &mut self.channels[i];
            let note_delay_first_tick = if pattern.is_note_delay() { self.tick == pattern.get_y() as u32 } else {first_tick};

            if !channel.voice.sustained {
                if channel.voice.volume.fadeout_vol - channel.voice.volume.fadeout_speed * 2 < 0 {
                    channel.voice.volume.fadeout_vol = 0;
                } else {
                    channel.voice.volume.fadeout_vol -= channel.voice.volume.fadeout_speed * 2;
                }
            }

            if first_tick && pattern.is_porta_to_note() && pattern.instrument != 0 {
                let sample = self.song_data.get_sample(&channel);
                channel.voice.volume.retrig(sample.volume as i32);
                channel.reset_envelopes(instruments);
            }

            let note = if !is_note_valid(pattern.note) && pattern.is_note_delay() && !first_tick {channel.last_played_note} else {pattern.note};

            // if note_delay_first_tick && note == 97 && !pattern.is_porta_to_note() { // note off
            //     channel.key_off(pattern.is_note_delay());
            // }

            if !pattern.is_porta_to_note() &&
                ((pattern.is_note_delay() && self.tick == pattern.get_y() as u32) ||
                    (!pattern.is_note_delay() && first_tick)) { // new row, set instruments

                let mut reset_envelope = false;
                if pattern.instrument != 0 {
                    let instrument = if pattern.instrument < instruments.len() as u8 {pattern.instrument as usize} else {0};
                    channel.voice.instrument = instrument;
                    if is_note_valid(note) {
                        channel.voice.sample = instruments[instrument].sample_indexes[(note - 1)  as usize] as usize;
                        channel.last_played_note = note;
                    }

                    // channel.volume.retrig(channel.sample.volume as i32);
                    reset_envelope = true;

                    channel.panning.panning = self.song_data.get_sample(channel).panning;
                }


                if note == 97 { // note off
                    if !channel.key_off(instruments, pattern.is_note_delay()) {
                        // continue;
                    }
                }

                channel.frequency_shift = 0.0;
                channel.period_shift = 0;

                // let mut reset_envelope = false;
                if reset_envelope {
                    channel.voice.volume.retrig(self.song_data.get_sample(channel).volume as i32);
                    channel.reset_envelopes(instruments);
                }

                if pattern.is_note_delay() {
                    channel.reset_envelopes(instruments);
                }

                channel.trigger_note(instruments, note, self.rate, self.frequency_tables.borrow());
            }

            // handle vibrato
            if !first_tick && pattern.has_vibrato() { // vibrate
                channel.frequency_shift = channel.vibrato_state.get_frequency_shift(WaveControl::from(channel.vibrato_control)) as f32;
                channel.update_frequency(self.rate, false, self.frequency_tables.borrow());
            }

            // handle tremolo (not really need to do it here, but oh, well)
            if !first_tick && pattern.has_tremolo() { // tremolate
                channel.voice.volume.volume_shift = channel.tremolo_state.get_volume_shift(WaveControl::from(channel.tremolo_control));
            }

            match pattern.volume {
                0x10..=0x50 => { channel.set_volume(note_delay_first_tick, pattern.volume - 0x10); }       // set volume
                0x60..=0x6f => { channel.volume_slide(note_delay_first_tick, -(pattern.get_volume_param() as i8)); }       // Volume slide down
                0x70..=0x7f => { channel.volume_slide(note_delay_first_tick, pattern.get_volume_param() as i8); }    // Volume slide up
                0x80..=0x8f => { channel.fine_volume_slide(note_delay_first_tick, -(pattern.get_volume_param() as i8)); }   // Fine volume slide down
                0x90..=0x9f => { channel.fine_volume_slide(note_delay_first_tick, pattern.get_volume_param() as i8); } // Fine volume slide up
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
                    if pan > 255 {
                        channel.panning.set_panning(255);
                    } else {
                        channel.panning.set_panning(pan as i32);
                    }
                }
                0xf0..=0xff => {channel.porta_to_note(instruments, first_tick, pattern.volume & 0xf, pattern.note, self.rate, self.frequency_tables.borrow()); }// Tone porta

                _ => {}
            }


            // handle effects
            match pattern.effect {
                0x0 => {  // Arpeggio
                    if pattern.effect_param != 0 {
                        channel.arpeggio(self.tick, pattern.get_x(), pattern.get_y());
                        channel.update_frequency(self.rate, true, self.frequency_tables.borrow());
                    }
                }
                0x1 => { channel.porta_up(first_tick, pattern.effect_param, self.rate, self.frequency_tables.borrow()); } // Porta up
                0x2 => { channel.porta_down(first_tick, pattern.effect_param, self.rate, self.frequency_tables.borrow()); } // Porta down
                0x3 => { channel.porta_to_note(instruments, first_tick, pattern.effect_param, pattern.note, self.rate, self.frequency_tables.borrow()); } // Porta to note
                0x4 => { channel.vibrato(first_tick, pattern.get_x() * 4, pattern.get_y()); } // vibrato
                0x5 => { // porta to note + volume slide
                    channel.porta_to_note(instruments, first_tick, 0, 0, self.rate, self.frequency_tables.borrow());
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
                        channel.voice.sample_position = channel.last_sample_offset as f32;
                        if channel.last_sample_offset > self.song_data.get_sample(channel).length {
                            channel.key_off(instruments, false);
                        }
                    }
                }
                0xA => {
                    channel.volume_slide_main(note_delay_first_tick, pattern.effect_param);
                }
                0xB => { // Pattern Jump
                    self.pattern_change.set_jump(first_tick, pattern.effect_param);
                }
                0xC => { channel.set_volume(note_delay_first_tick, pattern.effect_param); } // set volume
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
                    self.global_volume.set_volume(note_delay_first_tick, pattern.effect_param);
                }
                0x11 => { // global volume slide
                    self.global_volume.volume_slide(note_delay_first_tick, pattern.effect_param);
                }
                0x14 => { // key off
                    if self.tick == pattern.effect_param as u32 {
                        channel.key_off(instruments, pattern.is_note_delay());
                    }
                }
                0x15 => { // set envelope position
                    let instrument = self.song_data.get_instrument(channel);
                    if instrument.volume_envelope.on { channel.volume_envelope_state.set_position(&instrument.volume_envelope, pattern.effect_param);}
                    // FT2 bug - only set panning position if volume sustain is set
                    if instrument.volume_envelope.sustain { channel.panning_envelope_state.set_position(&instrument.panning_envelope, pattern.effect_param);}
                }
                0x19 => {
                    channel.panning_slide(first_tick, pattern.effect_param);
                }
                0x1b => {
                    channel.multi_retrig(instruments, first_tick, self.tick, pattern.effect_param, note, self.rate, self.frequency_tables.borrow());
                }
                0x1d => {
                    channel.tremor(self.tick, pattern.effect_param);
                }
                _ => {missing.push_str(format!("channel: {}, eff: {:x},", i, pattern.effect).as_ref());}
            }

            if pattern.effect == 0xe {
                match pattern.get_x() {
                    0x1 => { channel.fine_porta_up(first_tick, pattern.get_y(), self.rate, self.frequency_tables.borrow()); } // Porta up
                    0x2 => { channel.fine_porta_down(first_tick, pattern.get_y(), self.rate, self.frequency_tables.borrow()); } // Porta down
                    0x3 => { channel.glissando = pattern.get_y() == 1; }
                    0x4 => { channel.vibrato_control = pattern.get_y();}
                    0x7 => { channel.tremolo_control = pattern.get_y();}
                    0x8 => { channel.panning.set_panning((pattern.get_y() * 17) as i32);}
                    0x9 => { channel.retrig_note(instruments, first_tick, self.tick, pattern.get_y(), pattern.note, self.rate, self.frequency_tables.borrow());}
                    0xa => { channel.fine_volume_slide_up(note_delay_first_tick, pattern.get_y());} // volume slide up
                    0xb => { channel.fine_volume_slide_down(note_delay_first_tick, pattern.get_y());} // volume slide up
                    0xc => { channel.set_volume(self.tick == pattern.get_y() as u32, 0); }
                    0xd => {} // handled elsewhere
                    _ => {missing.push_str(format!("channel_state: {}, eff: 0xe{:x},", i, pattern.get_x()).as_ref());}
                }
            }

            let instrument = self.song_data.get_instrument(channel);

            let envelope_volume = channel.volume_envelope_state.handle(&instrument.volume_envelope, channel.voice.sustained, 64, false);

            let mut envelope_panning = channel.panning_envelope_state.handle(&instrument.panning_envelope, channel.voice.sustained, 32, true);
            // let scale = 0.9;
            envelope_panning = clamp(envelope_panning, 0, 64 * 256);


            channel.panning.update_envelope_panning(envelope_panning);
            // FinalVol = (FadeOutVol/65536)*(EnvelopeVol/64)*(GlobalVol/64)*(Vol/64)*Scale;
            // channel_state.update_frequency(self.rate);

            let global_volume = self.global_volume.volume as f32 / 64.0 ;
            channel.voice.volume.envelope_vol = envelope_volume as i32;
            channel.voice.volume.global_vol = self.global_volume.volume as i32;
            channel.voice.volume.output_volume = (channel.voice.volume.fadeout_vol as f32 / 65536.0) * (envelope_volume as f32 / 16384.0) * (channel.voice.volume.get_volume() as f32 / 64.0) * global_volume;
            
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

    fn lerp(pos: f32, p1: f32, p2: f32) -> f32 {

        let t = pos.fract();

        return (1.0 - t) * p1 + t * p2;
    }

//     fn output_channels(&mut self, current_buf_position: usize, buf: &mut [f32; AUDIO_BUF_SIZE], ticks_to_generate: usize) {
//         // let mut  idx: u32 = 0;
//
//         // let onecc = 1.0f32;// / cc as f32;
//         // FT2 quirk: global volume is used at channel volume calculation time, not at mixing time
//         //let global_volume = self.volume as f32 / 64.0 ;
//         // println!("position: {:3}, row: {:3}", self.song_position, self.row);
//
//
//         for channel in &mut self.channels {
//
//             // idx = idx + 1;
// //            if idx != 1  {continue;}
//             if !channel.on || channel.force_off {
//                 continue;
//             }
//
//             // print!("channel_state: {}, instrument: {}, frequency: {}, volume: {}\n", idx, channel_state.instrument.name, channel_state.frequency, channel_state.volume);
//
//             let sample = self.song_data.get_sample(channel);
//
//             let vol_right = PANNING_TAB[      channel.panning.final_panning as usize] as f32 / 65536.0;
//             let vol_left  = PANNING_TAB[256 - channel.panning.final_panning as usize] as f32 / 65536.0;
//             for i in 0..ticks_to_generate as usize {
//
//                 if channel.voice.sample_position as u32 >= sample.length { // we could have this after set sample position
//                     channel.on = false;
//                     break;
//                 }
//
//                 let sample_data = sample.data[channel.voice.sample_position as usize];
//                 let out_sample: f32 = if self.filter {
//                        Self::lerp(channel.voice.sample_position, sample_data, sample.data[channel.voice.sample_position as usize + 1])
//                     } else {
//                         sample_data
//                     };
//                 // channel.last_sample = sample_data;
//                 // channel.last_sample_pos = channel.sample_position;
//
//                 buf[(current_buf_position + i) * 2 + 0] +=  vol_left * out_sample / 4.0 * channel.voice.volume.output_volume;// * global_volume;
//                 buf[(current_buf_position + i) * 2 + 1] += vol_right * out_sample / 4.0 * channel.voice.volume.output_volume;// * global_volume;
//
//                 // if (i & 63) == 0 {print!("{}\n", channel_state.sample_position);}
//                 if sample.loop_type == LoopType::PingPongLoop && !channel.voice.ping {
//                     channel.voice.sample_position -= channel.voice.du;
//                 } else {
//                     channel.voice.sample_position += channel.voice.du;
//                 }
//
//                 if channel.voice.sample_position as u32 >= sample.length ||
//                     (sample.loop_type != LoopType::NoLoop && channel.voice.sample_position >= sample.loop_end as f32) {
//                     channel.voice.loop_started = true;
//                     match sample.loop_type {
//                         LoopType::PingPongLoop => {
//                             channel.voice.sample_position = (sample.loop_end - 1) as f32 - (channel.voice.sample_position - sample.loop_end as f32);
//                             channel.voice.ping = false;
//                             // channel_state.sample_position = (channel_state.sample.loop_end - 1) as f32;
//                             // channel_state.du = -channel_state.du;
//                         }
//                         LoopType::NoLoop => {
//                             channel.on = false;
//                             channel.voice.volume.set_volume(0);
//                             break;
//                         }
//                         LoopType::ForwardLoop => {
//                             channel.voice.sample_position = (channel.voice.sample_position - sample.loop_end as f32) + sample.loop_start as f32;
//                         }
//                     }
//                 }
//
//                 if channel.voice.loop_started && channel.voice.sample_position < sample.loop_start as f32 {
//                     match sample.loop_type {
//                         LoopType::PingPongLoop => {
//                             channel.voice.ping = true;
//                         }
//                         _ => {}
//                     }
//                     channel.voice.sample_position = sample.loop_start as f32 + (sample.loop_start as f32 - channel.voice.sample_position) as f32;
//                 }
//             }
//         }
//     }

    fn output_channels(&mut self, current_buf_position: usize, buf: &mut impl BufferAdapter, ticks_to_generate: usize) {
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

            let sample = self.song_data.get_sample(channel);

            let vol_right = PANNING_TAB[      channel.panning.final_panning as usize] as f32 / 65536.0;
            let vol_left  = PANNING_TAB[256 - channel.panning.final_panning as usize] as f32 / 65536.0;
            for i in 0..ticks_to_generate as usize {

                if channel.voice.sample_position as u32 >= sample.length { // we could have this after set sample position
                    channel.on = false;
                    break;
                }

                let sample_data = sample.data[channel.voice.sample_position as usize];
                let out_sample: f32 = if self.filter {
                    Self::lerp(channel.voice.sample_position, sample_data, sample.data[channel.voice.sample_position as usize + 1])
                } else {
                    sample_data
                };
                // channel.last_sample = sample_data;
                // channel.last_sample_pos = channel.sample_position;

                let final_sample = out_sample / 4.0 * channel.voice.volume.output_volume;
                buf.mix_sample(0, final_sample * vol_left, current_buf_position + i);
                buf.mix_sample(1, final_sample * vol_right, current_buf_position + i);

                // buf[(current_buf_position + i) * 2 + 0] +=  vol_left * out_sample / 4.0 * channel.voice.volume.output_volume;// * global_volume;
                // buf[(current_buf_position + i) * 2 + 1] += vol_right * out_sample / 4.0 * channel.voice.volume.output_volume;// * global_volume;

                // if (i & 63) == 0 {print!("{}\n", channel_state.sample_position);}
                if sample.loop_type == LoopType::PingPongLoop && !channel.voice.ping {
                    channel.voice.sample_position -= channel.voice.du;
                } else {
                    channel.voice.sample_position += channel.voice.du;
                }

                if channel.voice.sample_position as u32 >= sample.length ||
                    (sample.loop_type != LoopType::NoLoop && channel.voice.sample_position >= sample.loop_end as f32) {
                    channel.voice.loop_started = true;
                    match sample.loop_type {
                        LoopType::PingPongLoop => {
                            channel.voice.sample_position = (sample.loop_end - 1) as f32 - (channel.voice.sample_position - sample.loop_end as f32);
                            channel.voice.ping = false;
                            // channel_state.sample_position = (channel_state.sample.loop_end - 1) as f32;
                            // channel_state.du = -channel_state.du;
                        }
                        LoopType::NoLoop => {
                            channel.on = false;
                            channel.voice.volume.set_volume(0);
                            break;
                        }
                        LoopType::ForwardLoop => {
                            channel.voice.sample_position = (channel.voice.sample_position - sample.loop_end as f32) + sample.loop_start as f32;
                        }
                    }
                }

                if channel.voice.loop_started && channel.voice.sample_position < sample.loop_start as f32 {
                    match sample.loop_type {
                        LoopType::PingPongLoop => {
                            channel.voice.ping = true;
                        }
                        _ => {}
                    }
                    channel.voice.sample_position = sample.loop_start as f32 + (sample.loop_start as f32 - channel.voice.sample_position) as f32;
                }
            }
        }
    }

}
