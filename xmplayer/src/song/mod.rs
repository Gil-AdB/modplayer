use std::cmp::min;
use serde::Serialize;
use std::sync::mpsc::Receiver;

use crate::channel_state::{ChannelState, Voice};
use crate::channel_state::channel_state::{Note, Panning};
use crate::instrument::{LoopType, Instrument};
use crate::module_reader::{SongData, is_note_valid, Patterns, SongType};
#[cfg(test)]
#[allow(unused_imports)]
use crate::tables::{TableType, AMIGA_PERIODS, LINEAR_PERIODS};
use crate::tables::{PANNING_TAB, AudioTables};
use shared_sync_primitives::TripleBufferWriter;
use std::collections::HashMap;
use std::num::Wrapping;
use std::borrow::Borrow;
pub(crate) mod backend;

pub struct BPM {
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

pub struct PatternChange {
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

    fn set_break(&mut self, song_type: SongType, first_tick: bool, param: u8) {
        if !first_tick { return; }
        self.pattern_break = true;
        if song_type == SongType::MOD {
            // MOD uses BCD for Pattern Break
            self.row = (param >> 4) * 10 + (param & 0x0F);
        } else {
            // XM, IT, S3M use Hex
            self.row = param;
        }
    }

    fn set_jump(&mut self, first_tick: bool, param: u8) {
        if !first_tick { return; }
        self.pattern_jump = true;
        self.pattern = param;
        // Jump usually stays on same row unless Break is also present,
        // but often trackers treat Jump as "Jump to Pattern X, row 0"
        // and if both are present, Break wins for the row.
        if !self.pattern_break {
            self.row = 0;
        }
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



pub struct GlobalVolume {
    pub volume:            u32,
    pub(crate) last_volume_slide: u8,
    pub(crate) song_type:         Option<SongType>,
}

impl GlobalVolume {
    pub(crate) fn new() -> GlobalVolume {
        GlobalVolume {
            volume:            64,
            last_volume_slide: 0,
            song_type:         None,
        }
    }

    pub(crate) fn volume_slide(&mut self, first_tick: bool, param: u8) {
        if first_tick {
            let up = (param >> 4) as i32;
            let down = (param & 0xf) as i32;
            if up == 0xf && down != 0 {
                self.volume_slide_inner(down as i8);
            } else if down == 0xf && up != 0 {
                self.volume_slide_inner(-(up as i8));
            }
        } else {
            let up = (param >> 4) as i32;
            let down = (param & 0xf) as i32;
            if up != 0 && up != 0xf && down == 0 {
                self.volume_slide_inner(up as i8);
            } else if down != 0 && down != 0xf && up == 0 {
                self.volume_slide_inner(-(down as i8));
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
        let max_vol = if self.song_type == Some(SongType::IT) { 128 } else { 64 };
        new_volume = if new_volume < 0 {0} else if new_volume > max_vol { max_vol } else { new_volume };
        self.volume = new_volume as u32;
    }

    fn set_volume(&mut self, first_tick: bool, volume: u8) {
        if first_tick {
            let max_vol = if self.song_type == Some(SongType::IT) { 128 } else { 64 };
            self.volume = (volume as u32).min(max_vol as u32);
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


#[derive(Clone, Serialize)]
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
    pub oscilloscope:                       Vec<f32>,
    pub instrument_name:                    String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum FilterType {
    None,
    Linear,
    Cubic,
}

impl std::fmt::Display for FilterType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilterType::None => write!(f, "None"),
            FilterType::Linear => write!(f, "Linear"),
            FilterType::Cubic => write!(f, "Cubic"),
        }
    }
}

#[derive(Clone, Serialize)]
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
    pub filter:                             FilterType,
    pub song_message:                       String,
    pub virtual_channels:                   usize,
    pub user_data:                          HashMap<String, UserData>,
}

impl Default for PlayData {
    fn default() -> Self {
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
            filter: FilterType::Cubic,
            song_message: "".to_string(),
            virtual_channels: 0,
            user_data: Default::default()
        }
    }
}

enum BufferState {
    Start,
    FillBuffer,
    NextTick,
}

pub struct TickState {
    state:                  BufferState,
    current_buf_position:   usize,
    current_tick_position:  usize,
    pub row_delay:              usize,
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

#[derive(Clone, Debug, Serialize)]
pub enum UserData {
    String(String),
    ISize(isize),
    USize(usize)
}

// const BUFFER_SIZE: usize = 4096;
pub struct Song {
    pub name:                       String,
    pub song_position:              usize,
    pub row:                        usize,
    pub tick:                       u32,
    pub rate:                       f32,
    pub original_rate:              f32,
    pub speed:                      u32,
    pub global_volume:              GlobalVolume,
    pub song_data:                  SongData,
    pub channels:                   Vec<ChannelState>,
    pub voices:                     Vec<Voice>,
    pub pattern_change:             PatternChange,
    pub bpm:                        BPM,
    pub loop_pattern:               bool,
    pub pause:                      bool,
    pub filter:                     FilterType,
    pub display:                    bool,
    pub frequency_tables:           Box<AudioTables>,
    pub triple_buffer_writer:       TripleBufferWriter<PlayData>,
    pub tick_state:                 TickState,
    pub song_message:               String,
    pub user_data:                  HashMap<String, UserData>,
    pub(crate) old_effects:         bool,
    pub(crate) compatible_g:        bool,
    pub(crate) master_volume:       u8,
    pub(crate) mixing_volume:       u8,
    pub(crate) backend:             Option<Box<dyn crate::song::backend::ModuleBackend>>,
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
        let mut channels = Vec::with_capacity(song_data.channel_count as usize);
        for i in 0..song_data.channel_count as usize {
            let mut channel = ChannelState::new();
            if i < 64 {
                let p = song_data.initial_channel_panning[i];
                if p == 100 {
                    channel.panning.panning = 128; // Surround -> Center for now, but flagged
                    // TODO: Implement actual surround
                } else {
                    channel.panning.panning = (p as i16 * 4).min(255) as u8;
                }
                channel.volume.set_volume(song_data.initial_channel_volume[i] as i32);
                channel.channel_volume = song_data.initial_channel_volume[i];
            }
            channels.push(channel);
        }

        let mut voices = Vec::with_capacity(256);
        for i in 0..256 {
            let mut v = Voice::new();
            v.channel_idx = i % (song_data.channel_count as usize);
            voices.push(v);
        }

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
            song_message: song_data.song_message.clone(),
            song_data: song_data.clone(),
            channels,
            voices,
            old_effects: song_data.old_effects,
            compatible_g: song_data.compatible_g,
            loop_pattern: false,
            pattern_change: PatternChange::new(),
            pause: false,
            filter: FilterType::Cubic,
            display: true,
            frequency_tables: use_amiga,
            triple_buffer_writer,
            tick_state: TickState {
                state: BufferState::Start,
                current_buf_position: 0,
                current_tick_position: 0,
                row_delay: 0,
            },
            user_data: HashMap::new(),
            master_volume: song_data.master_volume,
            mixing_volume: song_data.mixing_volume,
            backend: match song_data.song_type {
                SongType::IT => Some(Box::new(crate::song::backend::ItBackend::new())),
                SongType::XM => Some(Box::new(crate::song::backend::XmBackend::new())),
                _ => Some(Box::new(crate::song::backend::S3MModBackend::new())),
            },
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
        play_data.song_message              = self.song_data.song_message.clone();
        play_data.channel_status.clear();
        play_data.filter                    = self.filter;
        play_data.user_data                 = self.user_data.clone();

        let active_voices = self.voices.iter().filter(|v| v.on).count();
        let mut host_voices = 0;
        for channel in &self.channels {
            if let Some(v_idx) = channel.voice_idx {
                if self.voices[v_idx].on {
                    host_voices += 1;
                }
            }
        }
        play_data.virtual_channels = active_voices.saturating_sub(host_voices);

        for (i, channel) in self.channels.iter().enumerate() {
            let (instrument, sample, volume, envelope_vol, global_vol, fadeout_vol, sample_position, note_str, period, final_panning, instrument_name) = 
            if let Some(v_idx) = channel.voice_idx {
                if self.voices[v_idx].channel_idx == i {
                    let v = &self.voices[v_idx];
                    let instrument_name = if v.instrument < self.song_data.instruments.len() {
                        self.song_data.instruments[v.instrument].name.clone()
                    } else {
                        "".to_string()
                    };
                    (v.instrument, v.sample, v.volume.volume, v.volume.envelope_vol, v.volume.global_vol, v.volume.fadeout_vol, v.sample_position, channel.note.to_string(), channel.note.period, v.panning.final_panning, instrument_name)
                } else {
                    (0, 0, 0, 0, 0, 0, 0.0, "---".to_string(), 0, 128, "".to_string())
                }
            } else {
                (0, 0, 0, 0, 0, 0, 0.0, "---".to_string(), 0, 128, "".to_string())
            };

            let mut scope = vec![0.0; 512];
            for i in 0..512 {
                let idx = (channel.last_samples_pos + 4096 - 512 + i) % 4096;
                scope[i] = channel.last_samples[idx];
            }

            play_data.channel_status.push(ChannelStatus {
                volume:             volume as f32,
                envelope_volume:    envelope_vol as f32,
                global_volume:      global_vol as f32,
                fadeout_volume:     fadeout_vol as f32,
                on:                 channel.on,
                force_off:          channel.force_off,
                frequency:          channel.frequency, // needs to be updated per voice
                instrument,
                sample,
                sample_position,
                note:               note_str,
                period,
                final_panning,
                oscilloscope:       scope,
                instrument_name
            });
        }
        // Song::display(&play_data, 0);
    }

    pub fn get_channel_count(&self) -> usize {
        self.song_data.channel_count as usize
    }

    pub fn free(&mut self) {}
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
                    PlaybackCmd::FilterToggle => {
                        self.filter = match self.filter {
                            FilterType::None => FilterType::Linear,
                            FilterType::Linear => FilterType::Cubic,
                            FilterType::Cubic => FilterType::None,
                        }
                    }
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

    pub(crate) fn next_tick(&mut self) -> bool {
        if self.song_position >= self.song_data.song_length as usize {
            return false;
        }

        self.tick += 1;
        if self.tick >= self.speed * (self.tick_state.row_delay as u32 + 1) {
            self.tick_state.row_delay = 0;
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



    pub fn process_tick(&mut self) {
        let mut backend = self.backend.take().expect("Backend missing");
        
        let mut resources = crate::song::backend::SongPlaybackResources {
            song_position:      &mut self.song_position,
            row:                &mut self.row,
            tick:               &mut self.tick,
            speed:              &mut self.speed,
            global_volume:      &mut self.global_volume,
            song_data:          &self.song_data,
            channels:           &mut self.channels,
            voices:             &mut self.voices,
            pattern_change:     &mut self.pattern_change,
            row_delay:          &mut self.tick_state.row_delay,
            bpm:                &mut self.bpm,
            frequency_tables:   self.frequency_tables.borrow(),
            rate:               self.rate,
            old_effects:        self.old_effects,
            compatible_g:       self.compatible_g,
        };

        backend.process_tick(&mut resources);
        self.backend = Some(backend);
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


        let (voices, channels) = (&mut self.voices, &mut self.channels);
        let master_gain = (self.master_volume as f32 / 128.0) * (self.mixing_volume as f32 / 128.0);
        
        for voice in voices {
            if !voice.on { continue; }
            let final_master_gain = if self.song_data.song_type == SongType::IT { master_gain } else { 1.0 };

            let sample = self.song_data.get_sample(voice);

            let vol_right = PANNING_TAB[      voice.panning.final_panning as usize] as f32 / 65536.0;
            let vol_left  = PANNING_TAB[256 - voice.panning.final_panning as usize] as f32 / 65536.0;
            for i in 0..ticks_to_generate as usize {

                if voice.sample_position as u32 >= sample.length { // we could have this after set sample position
                    voice.on = false;
                    break;
                }

                let sample_data = sample.data[voice.sample_position as usize];
                let mut out_sample: f32 = match self.filter {
                    FilterType::Linear => {
                        let next_sample_data = if voice.sample_position as usize + 1 < sample.data.len() {
                            sample.data[voice.sample_position as usize + 1]
                        } else {
                            0.0
                        };
                        Self::lerp(voice.sample_position, sample_data, next_sample_data)
                    },
                    FilterType::Cubic => {
                        let pos = voice.sample_position as usize;
                        voice.spline_data.p0 = sample.data[pos.saturating_sub(1)];
                        voice.spline_data.p1 = sample_data;
                        voice.spline_data.p2 = sample.data[(pos + 1).min(sample.data.len() - 1)];
                        voice.spline_data.p3 = sample.data[(pos + 2).min(sample.data.len() - 1)];
                        
                        voice.spline_data.interpolate(voice.sample_position.fract())
                    },
                    FilterType::None => {
                        sample_data
                    }
                };

                // Apply Resonant Filter if active
                if voice.filter_cutoff < 127 {
                    let input = out_sample;
                    let low = voice.filter_state.history[0];
                    let band = voice.filter_state.history[1];
                    
                    let new_band = band + voice.filter_state.a * (input - low - voice.filter_state.b * band);
                    let new_low = low + voice.filter_state.a * new_band;
                    
                    voice.filter_state.history[0] = new_low;
                    voice.filter_state.history[1] = new_band;
                    
                    out_sample = new_low;
                }

                let final_sample = out_sample / 4.0 * voice.volume.output_volume * final_master_gain;
                
                // Sum in per-channel oscilloscope buffer
                let channel = &mut channels[voice.channel_idx];
                channel.last_samples[(channel.last_samples_pos + i) % 4096] += final_sample;

                buf.mix_sample(0, final_sample * vol_left, current_buf_position + i);
                buf.mix_sample(1, final_sample * vol_right, current_buf_position + i);

                voice.sample_position += voice.du;

                if voice.sample_position as u32 >= sample.length {
                    if sample.loop_type == LoopType::NoLoop {
                        voice.on = false;
                        break;
                    }
                }

                if sample.loop_type != LoopType::NoLoop {
                    if voice.sample_position >= sample.loop_end as f32 {
                        voice.loop_started = true;
                        match sample.loop_type {
                            LoopType::ForwardLoop => {
                                voice.sample_position = (voice.sample_position - sample.loop_end as f32) + sample.loop_start as f32;
                            }
                            LoopType::PingPongLoop => {
                                voice.sample_position = (sample.loop_end - 1) as f32 - (voice.sample_position - sample.loop_end as f32);
                                voice.ping = false;
                                voice.du = -voice.du;
                            }
                            _ => {}
                        }
                    } else if voice.ping == false && voice.sample_position < sample.loop_start as f32 {
                         voice.sample_position = sample.loop_start as f32 + (sample.loop_start as f32 - voice.sample_position) as f32;
                         voice.ping = true;
                         voice.du = -voice.du;
                    }
                }
            }
        }

        for channel in &mut self.channels {
            channel.last_samples_pos = (channel.last_samples_pos + ticks_to_generate) % 4096;
            for i in 0..ticks_to_generate {
                channel.last_samples[(channel.last_samples_pos + i) % 4096] = 0.0;
            }
        }
    }

}
