use rustfft::{FftPlanner, Fft};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use crate::channel_state::{ChannelState, Voice};
use crate::instrument::Instrument;
use crate::module_reader::{SongData, SongType, Patterns};
#[cfg(test)]
#[allow(unused_imports)]
use crate::tables::{TableType, AMIGA_PERIODS, LINEAR_PERIODS};
use crate::tables::{AudioTables, AMIGA_TABLES, LINEAR_TABLES};
use shared_sync_primitives::TripleBufferWriter;
use std::collections::HashMap;

pub mod backend;
pub mod test_dump;
mod commands;
mod display;
mod output;
mod playback;

pub struct BPM {
    pub bpm:                    u32,
    tick_duration_in_ms:        f32,
    tick_duration_in_frames:    usize,

}

impl BPM {
    pub(crate) fn new(bpm: u32, rate: f32) -> BPM {
        let mut ret = BPM{
            bpm: 0,
            tick_duration_in_ms: 0.0,
            tick_duration_in_frames: 0
        };
        ret.update(bpm, rate);
        ret
    }
    pub(crate) fn update(&mut self, bpm: u32, rate: f32) {
        if bpm > 999 || bpm < 1 {return};
        self.bpm = bpm;
        self.tick_duration_in_ms = 2500.0 / self.bpm as f32;
        self.tick_duration_in_frames = (self.tick_duration_in_ms / 1000.0 * rate) as usize;
    }
}

pub struct PatternChange {
    pattern_break:  bool,
    pub(crate) pattern_jump:   bool,
    is_loop:        bool,
    pub(crate) row:            u8,
    pub(crate) pattern:        u8,
    pattern_delay:  u8,
    delay_processed:bool,
}

impl PatternChange {
    pub fn new() -> Self {
        Self{
            pattern_break: false,
            pattern_jump: false,
            is_loop: false,
            row: 0,
            pattern: 0,
            pattern_delay: 0, delay_processed: false,
        }
    }

    pub fn set_loop(&mut self, row: u8) {
        self.is_loop = true;
        self.row = row;
    }

    pub fn new_jump(order: usize) -> Self {
        Self {
            pattern_break: false,
            pattern_jump: true,
            is_loop: false,
            row: 0,
            pattern: order as u8,
            pattern_delay: 0, delay_processed: false,
        }
    }

    pub fn new_break(row: usize) -> Self {
        Self {
            pattern_break: true,
            pattern_jump: false,
            is_loop: false,
            row: row as u8,
            pattern: 0,
            pattern_delay: 0, delay_processed: false,
        }
    }
    fn reset(&mut self) {
        *self = Self::new();
    }

    fn set_break(&mut self, song_type: SongType, first_tick: bool, param: u8) {
        if !first_tick { return; }
        self.pattern_break = true;
        if song_type == SongType::MOD || song_type == SongType::XM {
            // MOD and XM use BCD for Pattern Break
            self.row = ((param >> 4) * 10 + (param & 0x0F)) as u8;
        } else {
            // S3M and IT use Hex (Decimal in S3M UI, but stored as raw byte)
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
        let mut actual_param = param;
        if actual_param == 0 {
            actual_param = self.last_volume_slide;
        } else {
            self.last_volume_slide = actual_param;
        }

        if first_tick {
            let up = (actual_param >> 4) as i32;
            let down = (actual_param & 0xf) as i32;
            if up == 0xf && down != 0 {
                self.volume_slide_inner(down as i8);
            } else if down == 0xf && up != 0 {
                self.volume_slide_inner(-(up as i8));
            }
        } else {
            let up = (actual_param >> 4) as i32;
            let down = (actual_param & 0xf) as i32;
            if up != 0 && up != 0xf && down == 0 {
                self.volume_slide_inner(up as i8);
            } else if down != 0 && down != 0xf && up == 0 {
                self.volume_slide_inner(-(down as i8));
            }
        }
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

    pub(crate) fn set_volume(&mut self, first_tick: bool, volume: u8) {
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
    SeekForward10s,
    SeekBackward10s,
    LoopPattern,
    Restart,
    Quit,
    AmigaTable,
    LinearTable,
    PauseToggle,
    FilterToggle,
    DisplayToggle,
    ChannelToggle(u8),
    ChannelSolo(u8),
    ChannelUnmuteAll,
    ChannelMuteAll,
    SetUserData(String, UserData),
    ModifyUserDataAddUSize(String, usize),
    ModifyUserDataSubUSize(String, usize),
    ModifyUserDataAddISize(String, isize),
    ModifyUserDataSubISize(String, isize),
    SpeedUp,
    SpeedDown,
    SpeedReset,
    SetPosition(u32),
    SetViewMode(u32),
    CycleTheme,
    ToggleScopes,
    ToggleVisualizerMode,
    IncLatency,
    DecLatency,
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
    pub pitch_shift:                        f32,
    pub oscilloscope:                       Vec<f32>,
    pub instrument_name:                    String,
}

impl Default for ChannelStatus {
    fn default() -> Self {
        Self {
            volume: 0.0,
            envelope_volume: 0.0,
            global_volume: 0.0,
            fadeout_volume: 0.0,
            on: false,
            force_off: false,
            frequency: 0.0,
            instrument: 0,
            sample: 0,
            sample_position: 0.0,
            note: "".to_string(),
            period: 0,
            final_panning: 128,
            pitch_shift: 0.0,
            oscilloscope: vec![0.0; 512],

            instrument_name: "".to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum FilterType {
    None,
    Linear,
    Cubic,
    Sinc,
}

impl std::fmt::Display for FilterType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilterType::None => write!(f, "None"),
            FilterType::Linear => write!(f, "Linear"),
            FilterType::Cubic => write!(f, "Cubic"),
            FilterType::Sinc => write!(f, "Sinc"),
        }
    }
}

#[derive(Clone, Serialize)]
pub struct PlayData {
    pub name:                               String,
    pub tick_duration_in_frames:            usize,
    pub tick_duration_in_ms:                f32,
    pub total_duration_ms:                  f32,
    pub current_duration_ms:                f32,
    pub global_volume:                      u32,
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
    pub visualizer_enabled:                 bool,
    pub scopes_enabled:                     bool,
    pub visualizer_mode:                    u32,
    pub master_spectrum:                    Vec<f32>,
    pub master_oscilloscope:                Vec<f32>,
    pub display_fps:                        f32,
    pub theme_id:                           u32,
    pub view_mode:                          u32,
    pub user_data:                          HashMap<String, UserData>,
    pub dump:                               crate::song::test_dump::TickDump,
}

impl Default for PlayData {
    fn default() -> Self {
        Self{
            name: "".to_string(),
            tick_duration_in_frames: 0,
            tick_duration_in_ms: 0.0,
            total_duration_ms: 0.0,
            current_duration_ms: 0.0,
            global_volume: 64,
            tick: 0,
            song_position: 0,
            song_length: 1,
            row: 0,
            pattern_len: 1,
            bpm: 0,
            speed: 0,
            channel_status: vec![],
            filter: FilterType::Sinc,
            song_message: "".to_string(),
            virtual_channels: 0,
            visualizer_enabled: true,
            scopes_enabled: true,
            visualizer_mode: 0,
            master_spectrum: vec![0.0; 128],
            master_oscilloscope: vec![0.0; 512],
            display_fps: 0.0,
            theme_id: 0,
            view_mode: 0,
            user_data: Default::default(),
            dump: Default::default(),
        }
    }
}

pub(crate) enum BufferState {
    Start,
    FillBuffer,
    NextTick,
}

pub struct TickState {
    pub(crate) state:                  BufferState,
    pub(crate) current_buf_position:   usize,
    pub(crate) current_tick_position:  usize,
}

pub enum CallbackState {
    Ok,
    Complete
}

pub trait BufferAdapter {
    fn mix_sample(&mut self, channel:usize, value: f32, pos: usize);
    fn mix_samples(&mut self, channel: usize, values: &[f32], pos: usize);
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

    fn mix_samples(&mut self, channel: usize, values: &[f32], pos: usize) {
        let mut p = pos * 2 + channel;
        for &v in values {
            self.buf[p] += v;
            p += 2;
        }
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
        self.buf[channel][pos] += value;
    }

    fn mix_samples(&mut self, channel: usize, values: &[f32], pos: usize) {
        let target = &mut self.buf[channel][pos..pos + values.len()];
        for i in 0..values.len() {
            target[i] += values[i];
        }
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
    pub total_duration_ms:          f32,
    pub bpm:                        BPM,
    pub loop_pattern:               bool,
    pub pause:                      bool,
    /// Counter for the paused-mode "play one row" UX. When > 0, playback
    /// runs as normal; each time `next_tick` advances to a new row this
    /// is decremented, and on reaching 0 `pause` is set back to true.
    /// Set to 1 by PlaybackCmd::Next while paused (see commands.rs).
    pub play_rows_remaining:        u32,
    pub filter:                     FilterType,
    pub display:                    bool,
    pub frequency_tables:           &'static AudioTables,
    pub is_fast_forwarding:         bool,
    pub is_calculating_duration:    bool,
    pub triple_buffer_writer:       TripleBufferWriter<PlayData>,
    pub master_samples:             [f32; 8192],
    pub master_samples_pos:         usize,
    pub visual_latency:             isize,
    pub tick_state:                 TickState,
    pub visualizer_enabled:         bool,
    pub visualizer_mode:            u32,
    pub master_spectrum:            Vec<f32>,
    pub master_oscilloscope:        Vec<f32>,
    pub theme_id:                   u32,
    pub view_mode:                  u32,
    pub display_count:              u32,
    pub total_samples:              u64,
    pub last_fps_sample:            u64,
    pub song_message:               String,
    pub user_data:                  HashMap<String, UserData>,
    pub last_display_update_sample: u64,
    pub fps:                        f32,
    #[cfg(not(target_arch = "wasm32"))]
    pub last_fps_time:              Instant,
    pub fft_planner:                FftPlanner<f32>,
    pub spectral_peaks:             Vec<f32>,
    pub hann_window:                Vec<f32>,
    pub cached_fft:                 Option<Arc<dyn Fft<f32>>>,
    pub bin_map:                    Vec<(usize, usize)>,
    pub old_effects:         bool,
    pub compatible_g:        bool,
    pub master_volume:       u8,
    pub mixing_volume:       u8,
    pub backend:             Option<Box<dyn crate::song::backend::ModuleBackend>>,
}

impl Song {
    pub fn new(song_data: &SongData, triple_buffer_writer: TripleBufferWriter<PlayData>, sample_rate: f32) -> Self {
        // Period/frequency tables depend only on use_amiga and are immutable;
        // pull a 'static reference into the lazy_static rather than building
        // and boxing fresh copies per Song.
        let frequency_tables: &'static AudioTables = if song_data.use_amiga {
            AMIGA_TABLES.as_ref()
        } else {
            LINEAR_TABLES.as_ref()
        };
        let mut channels = Vec::with_capacity(song_data.channel_count as usize);
        for i in 0..song_data.channel_count as usize {
            let mut channel = ChannelState::new();
            if i < 64 {
                let p = song_data.initial_channel_panning[i];
                if p == 100 {
                    channel.panning.panning = 128; // Surround -> Center for now
                } else {
                    channel.panning.panning = p;
                }
                channel.volume.set_volume(song_data.initial_channel_volume[i] as i32);
                channel.channel_volume = song_data.initial_channel_volume[i];
            }
            channels.push(channel);
        }

        let mut voices = Vec::with_capacity(256);
        for i in 0..256 {
            let mut v = Voice::new();
            let ch_count = (song_data.channel_count as usize).max(1);
            v.channel_idx = i % ch_count;
            voices.push(v);
        }

        let mut result = Self {
            name: song_data.name.clone(),
            song_position: 0,
            row: 0,
            tick: 0,
            rate: sample_rate,
            original_rate: sample_rate,
            speed: song_data.tempo as u32,
            total_duration_ms: 0.0,
            bpm: BPM::new(song_data.bpm as u32, sample_rate),
            global_volume: {
                let mut gv = GlobalVolume::new();
                gv.volume = song_data.global_volume as u32;
                gv.song_type = Some(song_data.song_type);
                gv
            },
            song_message: song_data.song_message.clone(),
            song_data: song_data.clone(),
            channels,
            voices,
            old_effects: song_data.old_effects,
            compatible_g: song_data.compatible_g,
            loop_pattern: false,
            pattern_change: PatternChange::new(),
            pause: false,
            play_rows_remaining: 0,
            filter: FilterType::Sinc,
            display: true,
            frequency_tables,
            is_fast_forwarding: false,
            is_calculating_duration: false,
            triple_buffer_writer,
            master_samples: [0.0; 8192],
            master_samples_pos: 0,
            visual_latency: 0,
            tick_state: TickState {
                state: BufferState::Start,
                current_buf_position: 0,
                current_tick_position: 0,
            },
            visualizer_enabled: true,
            visualizer_mode: 2,
            master_spectrum: vec![0.0; 128],
            master_oscilloscope: vec![0.0; 512],
            theme_id: 2,
            view_mode: 0,
            display_count: 0,
            total_samples: 0,
            last_fps_sample: 0,
            user_data: HashMap::new(),
            last_display_update_sample: 0,
            fps: 0.0,
            #[cfg(not(target_arch = "wasm32"))]
            last_fps_time: Instant::now(),
            fft_planner: FftPlanner::new(),
            spectral_peaks: vec![0.0; 128],
            hann_window: (0..2048).map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / 2047.0).cos())).collect(),
            cached_fft: None,
            bin_map: vec![],
            master_volume: song_data.master_volume,
            mixing_volume: song_data.mixing_volume,
            backend: match song_data.song_type {
                SongType::IT => Some(Box::new(crate::song::backend::ItBackend::new())),
                SongType::XM => Some(Box::new(crate::song::backend::XmBackend::new())),
                SongType::S3M => Some(Box::new(crate::song::backend::S3MBackend::new())),
                _ => Some(Box::new(crate::song::backend::ModBackend::new())),
            },
        };

        result.cached_fft = Some(result.fft_planner.plan_fft_forward(2048));
        result.recalculate_bin_map();
        result.total_duration_ms = result.compute_total_duration();
        result
    }

    pub fn free(&mut self) {}

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.rate = sample_rate;
        self.original_rate = sample_rate;
        self.recalculate_bin_map();
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

    pub fn get_channel_count(&self) -> usize {
        self.song_data.channel_count as usize
    }
}
