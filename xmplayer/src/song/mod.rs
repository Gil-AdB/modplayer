use std::cmp::min;
use rustfft::{FftPlanner, num_complex::Complex};
use serde::Serialize;
use std::sync::mpsc::Receiver;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

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
            oscilloscope: vec![0.0; 512],
            instrument_name: "".to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
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
    row_delay:              usize,
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
    pub master_samples:             [f32; 8192],
    pub master_samples_pos:         usize,
    pub visual_latency:             isize,
    pub tick_state:                 TickState,
    pub song_message:               String,
    pub user_data:                  HashMap<String, UserData>,
    pub(crate) old_effects:         bool,
    pub(crate) compatible_g:        bool,
    pub(crate) master_volume:       u8,
    pub(crate) mixing_volume:       u8,
    pub visualizer_enabled:         bool,
    pub visualizer_mode:            u32,
    pub master_spectrum:            Vec<f32>,
    pub master_oscilloscope:        Vec<f32>,
    pub theme_id:                   u32,
    pub view_mode:                  u32,
    pub display_count:              u32,
    pub total_samples:              u64,
    pub last_fps_sample:            u64,
    pub last_display_update_sample: u64,
    pub fps:                        f32,
    #[cfg(not(target_arch = "wasm32"))]
    pub last_fps_time:              Instant,
    fft_planner:                    FftPlanner<f32>,
    pub spectral_peaks:             Vec<f32>,
}

impl Song {
    fn apply_it_action(voices: &mut [Voice], voice_idx: usize, action: u8, instrument: &Instrument) {
        let voice = &mut voices[voice_idx];
        match action {
            0 => { // Cut
                voice.on = false;
                voice.volume.output_volume = 0.0;
            }
            1 => { // Continue
                // Do nothing
            }
            2 => { // Note Off
                if instrument.volume_envelope.on {
                    voice.sustained = false;
                } else {
                    // IT: If no volume envelope is active, Note Off = Cut
                    voice.on = false;
                    voice.volume.output_volume = 0.0;
                }
            }
            3 => { // Note Fade
                voice.sustained = false;
                voice.volume.fadeout_speed = (instrument.volume_fadeout as i32) << 6;
            }
            _ => {}
        }
    }

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
        let channel_count = if song_data.channel_count == 0 { 1 } else { song_data.channel_count as usize };
        for i in 0..256 {
            let mut v = Voice::new();
            v.channel_idx = i % channel_count;
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
            filter: FilterType::Sinc,
            display: true,
            frequency_tables: use_amiga,
            triple_buffer_writer,
            master_samples: [0.0; 8192],
            master_samples_pos: 0,
            visual_latency: 2048,
            tick_state: TickState {
                state: BufferState::Start,
                current_buf_position: 0,
                current_tick_position: 0,
                row_delay: 0,
            },
            user_data: HashMap::new(),
            master_volume: song_data.master_volume,
            mixing_volume: song_data.mixing_volume,
            visualizer_enabled: true,
            theme_id: 2, 
            view_mode: 0,
            visualizer_mode: 2,
            master_spectrum: vec![0.0; 128],
            master_oscilloscope: vec![0.0; 512],
            display_count: 0,
            fps: 0.0,
            total_samples: 0,
            last_fps_sample: 0,
            #[cfg(not(target_arch = "wasm32"))]
            last_fps_time: Instant::now(),
            last_display_update_sample: 0,
            fft_planner: FftPlanner::new(),
            spectral_peaks: vec![0.0; 128],
        }
    }


    // fn get_linear_frequency(note: i16, fine_tune: i32, period_offset: i32) -> f32 {
    //     let period = 10.0 * 12.0 * 16.0 * 4.0 - (note * 16 * 4) as f32  - (fine_tune as f32) / 2.0 + period_offset as f32;
    //     let two = 2.0f32;
    //     let frequency = 8363.0 * two.powf((6.0 * 12.0 * 16.0 * 4.0 - period) / (12.0 * 16.0 * 4.0));
    //     frequency as f32
    // }

    fn queue_display(&mut self) {
        let mut play_data = self.triple_buffer_writer.acquire_buffer();
        
        play_data.name                      = self.name.clone();
        play_data.tick_duration_in_frames   = self.bpm.tick_duration_in_frames;
        play_data.tick_duration_in_ms       = self.bpm.tick_duration_in_ms;
        play_data.tick                      = self.tick;
        play_data.song_position             = self.song_position;
        play_data.song_length               = self.song_data.song_length;
        play_data.row                       = self.row;
        if self.song_position < self.song_data.pattern_order.len() {
            let pat_idx = self.song_data.pattern_order[self.song_position] as usize;
            if pat_idx < self.song_data.patterns.len() {
                play_data.pattern_len           = self.song_data.patterns[pat_idx].rows.len();
            }
        }
        play_data.bpm                       = self.bpm.bpm;
        play_data.speed                     = self.speed;
        play_data.song_message              = self.song_data.song_message.clone();

        // --- INSTANT UI FEEDBACK (Always update user-controllable state) ---
        play_data.theme_id                  = self.theme_id;
        play_data.view_mode                 = self.view_mode;
        play_data.scopes_enabled            = match self.user_data.get("scopes_enabled") {
            Some(UserData::USize(v)) => *v % 2 != 0,
            _ => true
        };
        play_data.visualizer_mode           = self.visualizer_mode;
        play_data.filter                    = self.filter;
        play_data.user_data                 = self.user_data.clone();

        // IT virtual channel tracking
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

        // Optimized Channel Status (In-place update)
        let num_channels = self.channels.len();
        while play_data.channel_status.len() < num_channels {
            play_data.channel_status.push(ChannelStatus::default());
        }
        play_data.channel_status.truncate(num_channels);

        for i in 0..num_channels {
            let channel = &self.channels[i];
            let status = &mut play_data.channel_status[i];

            let (instrument, sample, volume, envelope_vol, global_vol, fadeout_vol, sample_position, note_str, period, final_panning, instrument_name) = 
            if let Some(v_idx) = channel.voice_idx {
                if self.voices[v_idx].channel_idx == i {
                    let v = &self.voices[v_idx];
                    let instrument_name = if v.instrument < self.song_data.instruments.len() {
                        self.song_data.instruments[v.instrument].name.clone()
                    } else {
                        "".to_string()
                    };
                    let mut sp = v.sample_position;
                    // Ping-pong loop position reporting
                    if v.instrument < self.song_data.instruments.len() {
                        let inst = &self.song_data.instruments[v.instrument];
                        if v.sample < inst.samples.len() {
                            let smp = &inst.samples[v.sample];
                            if smp.is_ping_pong && sp >= smp.original_loop_end as f32 {
                                let over = sp - smp.original_loop_end as f32;
                                sp = (smp.original_loop_end as f32 - 1.0) - over;
                            }
                        }
                    }
                    (v.instrument, v.sample, v.volume.volume, v.volume.envelope_vol, v.volume.global_vol, v.volume.fadeout_vol, sp, channel.note.to_string(), channel.note.period, v.panning.final_panning, instrument_name)
                } else {
                    (0, 0, 0, 0, 0, 0, 0.0, "---".to_string(), 0, 128, "".to_string())
                }
            } else {
                (0, 0, 0, 0, 0, 0, 0.0, "---".to_string(), 0, 128, "".to_string())
            };

            // High-fidelity scope normalization
            let mut peak = 0.01f32;
            for &s in channel.last_samples.iter() {
                peak = peak.max(s.abs());
            }
            let gain = if peak > 0.0001 { 0.5 / peak } else { 1.0 };

            if status.oscilloscope.len() != 512 {
                status.oscilloscope = vec![0.0; 512];
            }
            for j in 0..512 {
                let idx = (channel.last_samples_pos + 4096 - 512 + j) % 4096;
                status.oscilloscope[j] = channel.last_samples[idx] * gain;
            }

            status.volume             = volume as f32;
            status.envelope_volume    = envelope_vol as f32;
            status.global_volume      = global_vol as f32;
            status.fadeout_volume     = fadeout_vol as f32;
            status.on                 = channel.on;
            status.force_off          = channel.force_off;
            status.frequency          = channel.frequency;
            status.instrument         = instrument;
            status.sample             = sample;
            status.sample_position    = sample_position;
            status.note               = note_str;
            status.period             = period;
            status.final_panning      = final_panning;
            status.instrument_name    = instrument_name;
        }

        // Always update Visualizers (Scopes/FFT)
        self.display_count += 1;
        #[cfg(not(target_arch = "wasm32"))]
        {
            let elapsed = self.last_fps_time.elapsed();
            if elapsed.as_secs_f32() > 0.5 {
                self.fps = self.display_count as f32 / elapsed.as_secs_f32();
                self.display_count = 0;
                self.last_fps_time = Instant::now();
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
             let elapsed_secs = (self.total_samples - self.last_fps_sample) as f32 / self.rate;
             if elapsed_secs > 0.5 {
                 self.fps = self.display_count as f32 / elapsed_secs;
                 self.display_count = 0;
                 self.last_fps_sample = self.total_samples;
             }
        }

        // High-Fidelity Master Visualizer Data with Latency Compensation
        if play_data.master_oscilloscope.len() != 512 {
            play_data.master_oscilloscope = vec![0.0; 512];
        }
        
        // Calculate the history offset. 
        let history_len = 8192;
        let start_offset = (self.master_samples_pos as isize - self.visual_latency).rem_euclid(history_len as isize) as usize;

        for i in 0..512 {
            let idx = (start_offset + i) % history_len;
            play_data.master_oscilloscope[i] = self.master_samples[idx];
        }
        
        // Master FFT (Optimized with persistent planner and robust indexing)
        let fft = self.fft_planner.plan_fft_forward(1024);
        let mut fft_input_buffer = vec![0.0f32; 1024];
        let base_offset = (start_offset as isize - 256).rem_euclid(history_len as isize) as usize;
        for i in 0..1024 {
            let idx = (base_offset + i) % history_len; 
            fft_input_buffer[i] = self.master_samples[idx];
        }
        
        let mut fft_buffer: Vec<Complex<f32>> = fft_input_buffer.iter()
            .map(|&s| Complex::new(s, 0.0))
            .collect();
        fft.process(&mut fft_buffer);

        if play_data.master_spectrum.len() != 128 {
            play_data.master_spectrum = vec![0.0; 128];
        }

        let decay = if cfg!(target_arch = "wasm32") { 0.82f32 } else { 0.88f32 };

        for i in 0..128 {
            let val = fft_buffer[i].norm() / 10.0;
            self.spectral_peaks[i] = val.max(self.spectral_peaks[i] * decay);
            play_data.master_spectrum[i] = self.spectral_peaks[i];
        }

        play_data.display_fps = self.fps;
    }
    // Song::display(&play_data, 0);

    pub fn get_channel_count(&self) -> usize {
        self.song_data.channel_count as usize
    }

    pub fn free(&mut self) {}

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
                        self.total_samples += ticks_to_generate as u64;
                        self.tick_state.current_tick_position += ticks_to_generate;
                        self.tick_state.current_buf_position += ticks_to_generate;

                        if self.tick_state.current_buf_position == buf.num_frames() {
                             self.tick_state.current_buf_position = 0;
                             return CallbackState::Ok;
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

    pub fn handle_commands(&mut self, rx: & Receiver<PlaybackCmd>) -> bool {
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
                            FilterType::Cubic => FilterType::Sinc,
                            FilterType::Sinc => FilterType::None,
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
                    PlaybackCmd::SetViewMode(mode) => {
                        self.view_mode = mode;
                    }
                    PlaybackCmd::CycleTheme => {
                        self.theme_id = (self.theme_id + 1) % 4;
                    }
                    PlaybackCmd::ToggleScopes => {
                        self.visualizer_enabled = !self.visualizer_enabled;
                    }
                    PlaybackCmd::ToggleVisualizerMode => {
                        self.visualizer_mode = (self.visualizer_mode + 1) % 3;
                    }
                    PlaybackCmd::IncLatency => {
                        self.visual_latency = (self.visual_latency + 128).min(7000);
                    }
                    PlaybackCmd::DecLatency => {
                        self.visual_latency = (self.visual_latency - 128).max(0);
                    }
                }
                if self.display {
                    self.queue_display();
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
        let (voices, channels) = (&mut self.voices, &mut self.channels);
        let instruments = &self.song_data.instruments;
        let first_tick = self.tick == 0;

        // 1. Process channels (Note trigger and Effects)
        for i in 0..channels.len() {
            let channel = &mut channels[i];

            // Lazy cleanup: if the voice we were tracking was stolen by another channel, detach it now.
            if let Some(v_idx) = channel.voice_idx {
                if voices[v_idx].channel_idx != i {
                    channel.voice_idx = None;
                }
            }

            let patterns = &self.song_data.patterns[self.song_data.pattern_order[self.song_position] as usize];
            let row = &patterns.rows[self.row];
            let pattern = &row.channels[i];

            let note_delay_first_tick = if pattern.is_note_delay(self.song_data.song_type) { self.tick == pattern.get_y() as u32 } else {first_tick};

            if pattern.is_porta_to_note(self.song_data.song_type) && first_tick && is_note_valid(pattern.note, self.song_data.song_type) {
                channel.porta_to_note.target_note.period = channel.note.note_to_period(pattern.note, 0, self.frequency_tables.as_ref());
            }

            if !pattern.is_porta_to_note(self.song_data.song_type) &&
                ((pattern.is_note_delay(self.song_data.song_type) && self.tick == pattern.get_y() as u32) ||
                    (!pattern.is_note_delay(self.song_data.song_type) && first_tick)) {
                
                let note = pattern.note;
                let mut inst_idx = channel.last_instrument;
                if pattern.instrument != 0 {
                    inst_idx = if (pattern.instrument as usize) < instruments.len() { pattern.instrument as usize } else { 0 };
                    channel.last_instrument = inst_idx;
                }

                if is_note_valid(note, self.song_data.song_type) {
                    // IT Duplicate Check (DCT/DCA)
                    if let SongType::IT = self.song_data.song_type {
                        if inst_idx != 0 {
                            let new_inst = &instruments[inst_idx];
                            let mut dca_applied = false;

                            // Check all active voices for duplicates on this host channel
                            for vi in 0..voices.len() {
                                let v = &mut voices[vi];
                                if !v.on || v.channel_idx != i { continue; }

                                match new_inst.dct {
                                    1 => { // Note match
                                        if v.last_played_note == note {
                                            Song::apply_it_action(voices, vi, new_inst.dca, new_inst);
                                            dca_applied = true;
                                        }
                                    }
                                    2 => { // Sample match
                                        // Find sample index for new note
                                        let sample_idx = if (note as usize - 1) < new_inst.sample_indexes.len() {
                                            new_inst.sample_indexes[note as usize - 1].1
                                        } else { 0 };
                                        
                                        if sample_idx > 0 && v.sample == (sample_idx - 1) as usize && v.instrument == inst_idx {
                                            Song::apply_it_action(voices, vi, new_inst.dca, new_inst);
                                            dca_applied = true;
                                        }
                                    }
                                    3 => { // Instrument match
                                        if v.instrument == inst_idx {
                                            Song::apply_it_action(voices, vi, new_inst.dca, new_inst);
                                            dca_applied = true;
                                        }
                                    }
                                    _ => {}
                                }
                            }

                            // If no DCT matched, apply NNA to current voice if it exists
                            if !dca_applied {
                                if let Some(v_idx) = channel.voice_idx {
                                    if voices[v_idx].on {
                                        Song::apply_it_action(voices, v_idx, new_inst.nna, new_inst);
                                    }
                                }
                            }
                        }
                    } else if let Some(old_v_idx) = channel.voice_idx {
                        // Standard XM/S3M/MOD NNA (Cut/Off/Fade) - simplified
                        let instrument_nna = &instruments[voices[old_v_idx].instrument];
                        match instrument_nna.nna {
                            0 => { voices[old_v_idx].on = false; }
                            1 => { voices[old_v_idx].key_off(instruments, pattern.is_note_delay(self.song_data.song_type)); }
                            2 => {
                                voices[old_v_idx].sustained = false;
                                voices[old_v_idx].volume.fadeout_speed = (instrument_nna.volume_fadeout as i32) << 6;
                            }
                            _ => { voices[old_v_idx].key_off(instruments, pattern.is_note_delay(self.song_data.song_type)); }
                        }
                    }

                    // Start a new voice
                    let note_idx = (note - 1) as usize;
                    let mut trigger_voice = false;
                    let mut final_sample_idx = 0;
                    let mut mapped_note = note;

                    if inst_idx != 0 && note_idx < instruments[inst_idx].sample_indexes.len() {
                        let it_mapping = instruments[inst_idx].sample_indexes[note_idx];
                        let sample_idx = it_mapping.1 as usize;
                        if let SongType::IT = self.song_data.song_type {
                            mapped_note = it_mapping.0 + 1; // IT notes are 0..119, Pattern notes are 1..120
                            if sample_idx > 0 {
                                final_sample_idx = sample_idx - 1;
                                if final_sample_idx < instruments[inst_idx].samples.len() {
                                    trigger_voice = true;
                                }
                            }
                        } else {
                            final_sample_idx = sample_idx;
                            if final_sample_idx < instruments[inst_idx].samples.len() {
                                trigger_voice = true;
                            }
                        }
                    }

                    if trigger_voice {
                        // Find free voice or steal quietest
                        let mut v_idx = 0;
                        let mut found = false;
                        for vi in 0..voices.len() {
                            if !voices[vi].on { v_idx = vi; found = true; break; }
                        }
                        if !found {
                            let mut min_vol = 1_000_000.0f32;
                            for vi in 0..voices.len() {
                                if voices[vi].volume.output_volume < min_vol {
                                    min_vol = voices[vi].volume.output_volume;
                                    v_idx = vi;
                                }
                            }
                        }
                        
                        let mut clone_voice = None;
                        if pattern.instrument == 0 {
                            if let Some(old_idx) = channel.voice_idx {
                                clone_voice = Some(voices[old_idx].clone());
                            }
                        }
                        
                        let voice = &mut voices[v_idx];
                        voice.on = true;
                        voice.channel_idx = i;
                        voice.instrument = inst_idx;
                        voice.sample = final_sample_idx;
                        voice.last_played_note = mapped_note;
                        
                        if let Some(old_voice) = clone_voice {
                            voice.volume = old_voice.volume;
                            voice.panning = old_voice.panning;
                            voice.volume_envelope_state = old_voice.volume_envelope_state;
                            voice.panning_envelope_state = old_voice.panning_envelope_state;
                            voice.pitch_envelope_state = old_voice.pitch_envelope_state;
                            voice.vibrato_state = old_voice.vibrato_state;
                            voice.tremolo_state = old_voice.tremolo_state;
                            voice.instrument_global_volume = instruments[inst_idx].global_volume;
                            voice.sample_global_volume = old_voice.sample_global_volume;
                            voice.sample_position = 0.0;
                            voice.loop_started = false;
                            voice.ping = true;
                        } else {
                            voice.trigger_note(instruments);
                            let sample = &instruments[inst_idx].samples[final_sample_idx];
                            voice.volume.retrig(sample.volume as i32);
                            voice.panning.panning = sample.panning;
                        }

                        channel.voice_idx = Some(v_idx);
                        channel.last_played_note = note;
                        channel.on = true;
                        
                        // We need to borrow voice again because trigger_note might have changed it
                        let voice = &mut voices[v_idx];
                        let sample = &instruments[inst_idx].samples[final_sample_idx];
                        voice.surround = sample.surround;
                        let real_note = (mapped_note as i16 + sample.relative_note as i16) as u8;
                        channel.note.set_note(real_note, sample.finetune, mapped_note, self.frequency_tables.as_ref());
                        channel.update_frequency_voice(voice, self.rate, false, self.frequency_tables.as_ref());
                    }
                }

                if note == 97 { // Note Off
                    if let Some(v_idx) = channel.voice_idx {
                        voices[v_idx].key_off(&instruments, pattern.is_note_delay(self.song_data.song_type));
                    }
                } else if note == 121 { // Note Cut
                    if let Some(v_idx) = channel.voice_idx {
                        voices[v_idx].on = false;
                        voices[v_idx].volume.output_volume = 0.0;
                    }
                } else if note == 122 { // Note Fade
                    if let Some(v_idx) = channel.voice_idx {
                        voices[v_idx].sustained = false;
                        let instrument_nna = &instruments[voices[v_idx].instrument];
                        voices[v_idx].volume.fadeout_speed = (instrument_nna.volume_fadeout as i32) << 6;
                    }
                }
            }

            // Handle effects (even if there is no active voice, global effects and volume slides apply to channel state)
            let mut voice_ref = channel.voice_idx.and_then(|idx| {
                if voices[idx].channel_idx == i {
                    Some(&mut voices[idx])
                } else {
                    None
                }
            });
                
            if !first_tick && (pattern.has_vibrato(self.song_data.song_type) || pattern.effect == 0x4 || pattern.effect == 0x6) {
                channel.vibrato(voice_ref.as_deref_mut(), first_tick, pattern.get_vibrato_speed(), pattern.get_vibrato_depth(), self.old_effects, self.frequency_tables.as_ref());
            }

                match self.song_data.song_type {
                    SongType::IT => {
                        match pattern.volume {
                            0..=64 => { channel.set_volume(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.volume); }
                            65..=74 => { channel.fine_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, (pattern.volume - 65) as i8); }
                            75..=84 => { channel.fine_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, -((pattern.volume - 75) as i8)); }
                            85..=94 => { channel.volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, (pattern.volume - 85) as i8); }
                            95..=104 => { channel.volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, -((pattern.volume - 95) as i8)); }
                            105..=114 => { channel.porta_up(self.song_data.song_type, first_tick, (pattern.volume - 105) << 2); }
                            115..=124 => { channel.porta_down(self.song_data.song_type, first_tick, (pattern.volume - 115) << 2); }
                            128..=192 => { if let Some(v) = voice_ref.as_deref_mut() { v.panning.set_panning(((pattern.volume - 128) << 2) as i32); } } // Panning
                            193..=202 => { channel.porta_up(self.song_data.song_type, first_tick, pattern.volume - 192); } // Portamento Up
                            203..=212 => { channel.porta_down(self.song_data.song_type, first_tick, pattern.volume - 202); } // Portamento Down
                            _ => {}
                        }
                    }
                    _ => { // XM / S3M
                        match pattern.volume {
                            0x10..=0x50 => { channel.set_volume(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.volume - 0x10); }
                            0x60..=0x6f => { channel.volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, -(pattern.get_volume_param() as i8)); }
                            0x70..=0x7f => { channel.volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.get_volume_param() as i8); }
                            0x80..=0x8f => { channel.fine_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, -(pattern.get_volume_param() as i8)); }
                            0x90..=0x9f => { channel.fine_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.get_volume_param() as i8); }
                            0xa0..=0xaf => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, 0, pattern.get_volume_param(), self.old_effects, self.frequency_tables.as_ref()); }
                            0xb0..=0xbf => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, pattern.get_volume_param(), 0, self.old_effects, self.frequency_tables.as_ref()); }
                            0xd0..=0xdf => { channel.panning_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.get_volume_param() << 4); }
                            0xe0..=0xef => { channel.panning_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.get_volume_param()); }
                            0xf0..=0xff => { channel.porta_to_note(self.song_data.song_type, voice_ref.as_deref_mut(), note_delay_first_tick, pattern.get_volume_param(), self.compatible_g, self.frequency_tables.as_ref()); }
                            _ => {}
                        }
                    }
                }

                match self.song_data.song_type {
                    SongType::IT => {
                        match pattern.effect {
                            0x01 => { if first_tick { self.speed = pattern.effect_param as u32; } } // A: Set Speed
                            0x02 => { self.pattern_change.set_jump(first_tick, pattern.effect_param); } // B: Pattern Jump
                            0x03 => { self.pattern_change.set_break(self.song_data.song_type, first_tick, pattern.effect_param); } // C: Pattern Break
                            0x04 => { channel.it_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param); } // D: Volume Slide
                            0x05 => { // E: Porta Down
                                let param = if !self.compatible_g && pattern.effect_param == 0 { channel.last_it_slide_speed } else { pattern.effect_param };
                                if !self.compatible_g && pattern.effect_param != 0 { channel.last_it_slide_speed = pattern.effect_param; }
                                channel.porta_down(self.song_data.song_type, first_tick, param); 
                            }
                            0x06 => { // F: Porta Up
                                let param = if !self.compatible_g && pattern.effect_param == 0 { channel.last_it_slide_speed } else { pattern.effect_param };
                                if !self.compatible_g && pattern.effect_param != 0 { channel.last_it_slide_speed = pattern.effect_param; }
                                channel.porta_up(self.song_data.song_type, first_tick, param); 
                            }
                            0x07 => { // G: Porta Note
                                let param = if !self.compatible_g && pattern.effect_param == 0 { channel.last_it_slide_speed } else { pattern.effect_param };
                                if !self.compatible_g && pattern.effect_param != 0 { channel.last_it_slide_speed = pattern.effect_param; }
                                channel.porta_to_note(self.song_data.song_type, voice_ref.as_deref_mut(), first_tick, param, self.compatible_g, self.frequency_tables.as_ref()); 
                            }
                            0x08 => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, pattern.get_x(), pattern.get_y(), self.old_effects, self.frequency_tables.as_ref()); } // H: Vibrato
                            0x0A => { channel.arpeggio(self.tick, pattern.get_x(), pattern.get_y()); } // J: Arpeggio
                            0x0B => { // K: Vibrato + Volume Slide
                                channel.vibrato(voice_ref.as_deref_mut(), first_tick, 0, 0, self.old_effects, self.frequency_tables.as_ref());
                                channel.it_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param);
                            }
                            0x0C => { // L: Porta Note + Volume Slide
                                channel.porta_to_note(self.song_data.song_type, voice_ref.as_deref_mut(), first_tick, 0, self.compatible_g, self.frequency_tables.as_ref());
                                channel.it_volume_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param);
                            }
                            0x0F => { /* O: Offset - to be implemented */ }
                            0x11 => { channel.it_retrig(voice_ref.as_deref_mut(), &self.song_data.instruments, self.tick, pattern.effect_param); } // Q: Multi-Retrig
                            0x14 => { if first_tick { self.bpm.update(pattern.effect_param as u32, self.rate); } } // T: Set Tempo
                            0x16 => { self.global_volume.set_volume(note_delay_first_tick, pattern.effect_param); } // V: Set Global Vol
                            0x17 => { self.global_volume.volume_slide(note_delay_first_tick, pattern.effect_param); } // W: Global Volume Slide
                            0x18 => { if first_tick { if let Some(v) = voice_ref.as_deref_mut() { v.panning.set_panning((pattern.effect_param as i32 * 4).min(255)); } } } // X: Set Panning
                            0x13 => { // S: Special
                                let x = pattern.get_x();
                                match x {
                                    0x08 => { if first_tick { if let Some(v) = voice_ref.as_deref_mut() { v.panning.set_panning((pattern.get_y() << 4) as i32); } } } // S8x: Set Panning
                                    0x0C => { if self.tick == pattern.get_y() as u32 { channel.on = false; if let Some(v) = voice_ref.as_deref_mut() { v.on = false; } } } // SCx: Note Cut
                                    0x0D => { /* SDx: Note Delay - already handled by note_delay_first_tick logic */ }
                                    0x0E => { if first_tick { self.tick_state.row_delay = pattern.get_y() as usize; } } // SEx: Pattern Row Delay
                                    _ => {}
                                }
                            }
                            0x0D => { if first_tick { channel.channel_volume = pattern.effect_param.min(64); } } // M: Channel Volume
                            0x0E => { channel.channel_volume_slide(note_delay_first_tick, pattern.effect_param); } // N: Channel Volume Slide
                            0x10 => { channel.panning_slide(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param); } // P: Panning Slide
                            0x1A => { // Z: Resonant Filter
                                if let Some(v) = voice_ref.as_deref_mut() {
                                    if pattern.effect_param < 0x80 {
                                        v.filter_cutoff = pattern.effect_param;
                                    } else if (0x80..=0x8F).contains(&pattern.effect_param) {
                                        v.filter_resonance = (pattern.effect_param & 0x0F) << 3; // Map 0-15 to 0-120ish
                                    }
                                }
                            }
                            _ => {
                                // Fallback to existing logic if needed, but IT mapping is different
                            }
                        }
                    }
                    _ => { // XM / S3M / MOD
                        match pattern.effect {
                            0x1 => { channel.porta_up(self.song_data.song_type, first_tick, pattern.effect_param); }
                            0x2 => { channel.porta_down(self.song_data.song_type, first_tick, pattern.effect_param); }
                            0x3 => { channel.porta_to_note(self.song_data.song_type, voice_ref.as_deref_mut(), first_tick, pattern.effect_param, self.compatible_g, self.frequency_tables.as_ref()); }
                            0x4 => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, pattern.get_x(), pattern.get_y(), self.old_effects, self.frequency_tables.as_ref()); }
                            0x5 => { channel.porta_to_note(self.song_data.song_type, voice_ref.as_deref_mut(), first_tick, 0, self.compatible_g, self.frequency_tables.as_ref()); channel.volume_slide_main(voice_ref.as_deref_mut(), first_tick, pattern.effect_param); }
                            0x6 => { channel.vibrato(voice_ref.as_deref_mut(), first_tick, 0, 0, self.old_effects, self.frequency_tables.as_ref()); channel.volume_slide_main(voice_ref.as_deref_mut(), first_tick, pattern.effect_param); }
                            0x7 => { channel.tremolo(voice_ref.as_deref_mut(), first_tick, pattern.get_x(), pattern.get_y()); }
                            0xA => { channel.volume_slide_main(voice_ref.as_deref_mut(), note_delay_first_tick, pattern.effect_param); }
                            0xB => { self.pattern_change.set_jump(first_tick, pattern.effect_param); } // B: Pattern Jump
                            0xD => { self.pattern_change.set_break(self.song_data.song_type, first_tick, pattern.effect_param); } // D: Pattern Break
                            0x8 => { if let Some(v) = voice_ref.as_deref_mut() { v.panning.set_panning(pattern.effect_param as i32); } }
                            0x0F => { // Set Speed / BPM (Fxx)
                                if first_tick {
                                    if pattern.effect_param < 32 {
                                        self.speed = pattern.effect_param as u32;
                                    } else {
                                        self.bpm.update(pattern.effect_param as u32, self.rate);
                                    }
                                }
                            }
                            0x10 => { 
                                self.global_volume.set_volume(note_delay_first_tick, pattern.effect_param); 
                            }
                            0x11 => {
                                self.global_volume.volume_slide(first_tick, pattern.effect_param);
                            }
                            0xE => {
                                let subcommand = pattern.get_x();
                                let param = pattern.get_y();
                                match subcommand {
                                    0x1 => { channel.fine_porta_up(self.song_data.song_type, first_tick, param); }
                                    0x2 => { channel.fine_porta_down(self.song_data.song_type, first_tick, param); }
                                    0xA => { channel.fine_volume_slide(voice_ref.as_deref_mut(), first_tick, param as i8); }
                                    0xB => { channel.fine_volume_slide(voice_ref.as_deref_mut(), first_tick, -(param as i8)); }
                                    0xC => { if self.tick == param as u32 { if let Some(v) = voice_ref.as_deref_mut() { v.on = false; } } }
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    }
                }
                
                if let Some(v) = voice_ref.as_deref_mut() {
                    channel.update_frequency_voice(v, self.rate, false, self.frequency_tables.as_ref());
                }


        }

        // 2. Process all active voices (Envelopes and Final Volume)
        let divisor = if self.song_data.song_type == SongType::IT { 128.0 } else { 64.0 };
        let global_vol_f32 = self.global_volume.volume as f32 / divisor;
        for (v_idx, voice) in self.voices.iter_mut().enumerate() {
            if !voice.on { continue; }
            let channel_vol_f32 = self.channels[voice.channel_idx].channel_volume as f32 / 64.0;
            voice.update_envelopes(instruments, self.rate);
            voice.update_output_volume(global_vol_f32, channel_vol_f32, divisor);
            
            // Deactivate voice if it's silent and has no chance of making sound
            // Background voices (not the current voice of the channel) are killed if silent.
            let is_host_voice = self.channels[voice.channel_idx].voice_idx == Some(v_idx);
            
            if !voice.sustained && (voice.volume.fadeout_vol == 0 || voice.volume.output_volume < 0.00001) {
                voice.on = false;
            } else if !is_host_voice && voice.volume.output_volume < 0.00001 {
                // Background voice is silent - kill it to prevent virtual channel leak
                voice.on = false;
            }
        }
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


    fn output_channels(&mut self, current_buf_position: usize, buf: &mut impl BufferAdapter, ticks_to_generate: usize) {
        let (voices, channels) = (&mut self.voices, &mut self.channels);
        let master_gain = (self.master_volume as f32 / 128.0) * (self.mixing_volume as f32 / 128.0);
        
        for voice in voices {
            if !voice.on { continue; }
            let final_master_gain = if self.song_data.song_type == SongType::IT { master_gain } else { 1.0 };

            let sample = self.song_data.get_sample(voice);

            let vol_right = PANNING_TAB[      voice.panning.final_panning as usize] as f32 / 65536.0;
            let vol_left  = PANNING_TAB[256 - voice.panning.final_panning as usize] as f32 / 65536.0;
            for i in 0..ticks_to_generate as usize {

                if voice.sample_position as u32 >= sample.length {
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
                    FilterType::Sinc => {
                        let pos = voice.sample_position as usize;
                        let phase = (voice.sample_position.fract() * 512.0) as usize;
                        let table = &self.frequency_tables.resampling.sinc_table[phase];
                        let mut result = 0.0;
                        result += sample.data[pos - 3] * table[0];
                        result += sample.data[pos - 2] * table[1];
                        result += sample.data[pos - 1] * table[2];
                        result += sample.data[pos]     * table[3];
                        result += sample.data[pos + 1] * table[4];
                        result += sample.data[pos + 2] * table[5];
                        result += sample.data[pos + 3] * table[6];
                        result += sample.data[pos + 4] * table[7];
                        result
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

                let l = final_sample * vol_left;
                let r = final_sample * vol_right;

                // Collect master for FFT
                self.master_samples[self.master_samples_pos] = (l + r) / 2.0;
                self.master_samples_pos = (self.master_samples_pos + 1) % 8192;

                buf.mix_sample(0, l, current_buf_position + i);
                buf.mix_sample(1, r, current_buf_position + i);

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
