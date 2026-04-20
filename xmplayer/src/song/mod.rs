use crate::channel_state::{ChannelState, Voice};
use crate::channel_state::channel_state::{Note, Panning, Volume};
use crate::instrument::{LoopType, Instrument};
use crate::module_reader::{is_note_valid, Patterns, SongData, SongType};
use crate::tables::{PANNING_TAB, AudioTables};
use shared_sync_primitives::TripleBufferWriter;
use serde::Serialize;
use std::cmp::min;
use std::collections::HashMap;
use std::num::Wrapping;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use rustfft::{Fft, FftPlanner, num_complex::Complex};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
use std::arch::wasm32::*;

#[cfg(all(target_arch = "x86_64", target_feature = "sse"))]
use std::arch::x86_64::*;

// Global Gain Constant (from master)
pub const MASTER_GAIN: f32 = 0.5;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct BPM {
    pub bpm: u32,
    pub tick_duration_in_ms: f32,
    pub tick_duration_in_frames: usize,
}

impl BPM {
    pub fn new(bpm: u32, rate: f32) -> Self {
        let mut b = Self {
            bpm: 0,
            tick_duration_in_ms: 0.0,
            tick_duration_in_frames: 0,
        };
        b.update(bpm, rate);
        b
    }

    pub fn update(&mut self, bpm: u32, rate: f32) {
        if bpm == 0 { return; }
        self.bpm = bpm;
        self.tick_duration_in_ms = 2500.0 / self.bpm as f32;
        self.tick_duration_in_frames = (self.tick_duration_in_ms / 1000.0 * rate) as usize;
    }
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
use core::arch::wasm32::*;

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

#[cfg(target_arch = "aarch64")]
use core::arch::aarch64::*;

#[inline(always)]
fn sinc_dot_product(samples: &[f32], coeffs: &[f32; 8]) -> f32 {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        unsafe {
            let v0 = v128_load(samples.as_ptr() as *const v128);
            let v1 = v128_load(samples.as_ptr().add(4) as *const v128);
            let c0 = v128_load(coeffs.as_ptr() as *const v128);
            let c1 = v128_load(coeffs.as_ptr().add(4) as *const v128);
            
            let m0 = f32x4_mul(v0, c0);
            let m1 = f32x4_mul(v1, c1);
            
            let sum = f32x4_add(m0, m1);
            // Horizontal sum
            let temp = f32x4_add(sum, i32x4_shuffle::<2, 3, 0, 1>(sum, sum));
            let final_sum = f32x4_add(temp, i32x4_shuffle::<1, 0, 3, 2>(temp, temp));
            f32x4_extract_lane::<0>(final_sum)
        }
    }
    #[cfg(all(target_arch = "x86_64", target_feature = "sse"))]
    {
        unsafe {
            let v0 = _mm_loadu_ps(samples.as_ptr());
            let v1 = _mm_loadu_ps(samples.as_ptr().add(4));
            let c0 = _mm_load_ps(coeffs.as_ptr());
            let c1 = _mm_load_ps(coeffs.as_ptr().add(4));
            
            let m0 = _mm_mul_ps(v0, c0);
            let m1 = _mm_mul_ps(v1, c1);
            
            let sum = _mm_add_ps(m0, m1);
            // Horizontal sum
            let temp = _mm_add_ps(sum, _mm_movehl_ps(sum, sum));
            let final_sum = _mm_add_ss(temp, _mm_shuffle_ps(temp, temp, 1));
            _mm_cvtss_f32(final_sum)
        }
    }
    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    {
        unsafe {
            let v0 = vld1q_f32(samples.as_ptr());
            let v1 = vld1q_f32(samples.as_ptr().add(4));
            let c0 = vld1q_f32(coeffs.as_ptr());
            let c1 = vld1q_f32(coeffs.as_ptr().add(4));
            
            let m0 = vmulq_f32(v0, c0);
            let m1 = vmulq_f32(v1, c1);
            
            let sum = vaddq_f32(m0, m1);
            // Horizontal sum (Native hardware reduction on AArch64)
            vaddvq_f32(sum)
        }
    }
    #[cfg(not(any(
        all(target_arch = "wasm32", target_feature = "simd128"),
        all(target_arch = "x86_64", target_feature = "sse"),
        all(target_arch = "aarch64", target_feature = "neon")
    )))]
    {
        let mut result = 0.0;
        result += samples[0] * coeffs[0];
        result += samples[1] * coeffs[1];
        result += samples[2] * coeffs[2];
        result += samples[3] * coeffs[3];
        result += samples[4] * coeffs[4];
        result += samples[5] * coeffs[5];
        result += samples[6] * coeffs[6];
        result += samples[7] * coeffs[7];
        result
    }
}

#[inline(always)]
fn lerp_simd(lo: [f32; 4], hi: [f32; 4], t: [f32; 4]) -> [f32; 4] {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        unsafe {
            let vlo = v128_load(lo.as_ptr() as *const v128);
            let vhi = v128_load(hi.as_ptr() as *const v128);
            let vt  = v128_load(t.as_ptr() as *const v128);
            let diff = f32x4_sub(vhi, vlo);
            let res  = f32x4_add(vlo, f32x4_mul(diff, vt));
            
            let mut result = [0.0f32; 4];
            v128_store(result.as_mut_ptr() as *mut v128, res);
            result
        }
    }
    #[cfg(all(target_arch = "x86_64", target_feature = "sse"))]
    {
        unsafe {
            let vlo = _mm_loadu_ps(lo.as_ptr());
            let vhi = _mm_loadu_ps(hi.as_ptr());
            let vt  = _mm_loadu_ps(t.as_ptr());
            
            let diff = _mm_sub_ps(vhi, vlo);
            let res  = _mm_add_ps(vlo, _mm_mul_ps(diff, vt));
            
            let mut result = [0.0f32; 4];
            _mm_storeu_ps(result.as_mut_ptr(), res);
            result
        }
    }
    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    {
        unsafe {
            let vlo = vld1q_f32(lo.as_ptr());
            let vhi = vld1q_f32(hi.as_ptr());
            let vt  = vld1q_f32(t.as_ptr());
            
            let diff = vsubq_f32(vhi, vlo);
            let res  = vaddq_f32(vlo, vmulq_f32(diff, vt));
            
            let mut result = [0.0f32; 4];
            vst1q_f32(result.as_mut_ptr(), res);
            result
        }
    }
    #[cfg(not(any(
        all(target_arch = "wasm32", target_feature = "simd128"),
        all(target_arch = "x86_64", target_feature = "sse"),
        all(target_arch = "aarch64", target_feature = "neon")
    )))]
    {
        [
            lo[0] + (hi[0] - lo[0]) * t[0],
            lo[1] + (hi[1] - lo[1]) * t[1],
            lo[2] + (hi[2] - lo[2]) * t[2],
            lo[3] + (hi[3] - lo[3]) * t[3],
        ]
    }
}

#[inline(always)]
fn cubic_simd(p0: [f32; 4], p1: [f32; 4], p2: [f32; 4], p3: [f32; 4], t: [f32; 4]) -> [f32; 4] {
     #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        unsafe {
            let vp0 = v128_load(p0.as_ptr() as *const v128);
            let vp1 = v128_load(p1.as_ptr() as *const v128);
            let vp2 = v128_load(p2.as_ptr() as *const v128);
            let vp3 = v128_load(p3.as_ptr() as *const v128);
            let vt  = v128_load(t.as_ptr() as *const v128);
            
            let three = f32x4_splat(3.0);
            let two   = f32x4_splat(2.0);
            let five  = f32x4_splat(5.0);
            let four  = f32x4_splat(4.0);
            let half  = f32x4_splat(0.5);

            let c3 = f32x4_add(f32x4_sub(f32x4_mul(three, f32x4_sub(vp1, vp2)), vp0), vp3);
            let c2 = f32x4_sub(f32x4_add(f32x4_sub(f32x4_mul(two, vp0), f32x4_mul(five, vp1)), f32x4_mul(four, vp2)), vp3);
            let c1 = f32x4_sub(vp2, vp0);
            let c0 = vp1;

            let res = f32x4_add(f32x4_mul(half, f32x4_mul(f32x4_add(f32x4_mul(f32x4_add(f32x4_mul(c3, vt), c2), vt), c1), vt)), c0);

            let mut result = [0.0f32; 4];
            v128_store(result.as_mut_ptr() as *mut v128, res);
            result
        }
    }
    #[cfg(all(target_arch = "x86_64", target_feature = "sse"))]
    {
        unsafe {
            let vp0 = _mm_loadu_ps(p0.as_ptr());
            let vp1 = _mm_loadu_ps(p1.as_ptr());
            let vp2 = _mm_loadu_ps(p2.as_ptr());
            let vp3 = _mm_loadu_ps(p3.as_ptr());
            let vt  = _mm_loadu_ps(t.as_ptr());
            
            let three = _mm_set1_ps(3.0);
            let two   = _mm_set1_ps(2.0);
            let five  = _mm_set1_ps(5.0);
            let four  = _mm_set1_ps(4.0);
            let half  = _mm_set1_ps(0.5);

            let c3 = _mm_add_ps(_mm_sub_ps(_mm_mul_ps(three, _mm_sub_ps(vp1, vp2)), vp0), vp3);
            let c2 = _mm_sub_ps(_mm_add_ps(_mm_sub_ps(_mm_mul_ps(two, vp0), _mm_mul_ps(five, vp1)), _mm_mul_ps(four, vp2)), vp3);
            let c1 = _mm_sub_ps(vp2, vp0);
            let c0 = vp1;

            let res = _mm_add_ps(_mm_mul_ps(half, _mm_mul_ps(_mm_add_ps(_mm_mul_ps(_mm_add_ps(_mm_mul_ps(c3, vt), c2), vt), c1), vt)), c0);

            let mut result = [0.0f32; 4];
            _mm_storeu_ps(result.as_mut_ptr(), res);
            result
        }
    }
    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    {
        unsafe {
            let vp0 = vld1q_f32(p0.as_ptr());
            let vp1 = vld1q_f32(p1.as_ptr());
            let vp2 = vld1q_f32(p2.as_ptr());
            let vp3 = vld1q_f32(p3.as_ptr());
            let vt  = vld1q_f32(t.as_ptr());
            
            let three = vdupq_n_f32(3.0);
            let two   = vdupq_n_f32(2.0);
            let five  = vdupq_n_f32(5.0);
            let four  = vdupq_n_f32(4.0);
            let half  = vdupq_n_f32(0.5);

            let c3 = vaddq_f32(vsubq_f32(vmulq_f32(three, vsubq_f32(vp1, vp2)), vp0), vp3);
            let c2 = vsubq_f32(vaddq_f32(vsubq_f32(vmulq_f32(two, vp0), vmulq_f32(five, vp1)), vmulq_f32(four, vp2)), vp3);
            let c1 = vsubq_f32(vp2, vp0);
            let c0 = vp1;

            let res = vaddq_f32(vmulq_f32(half, vmulq_f32(vaddq_f32(vmulq_f32(vaddq_f32(vmulq_f32(c3, vt), c2), vt), c1), vt)), c0);

            let mut result = [0.0f32; 4];
            vst1q_f32(result.as_mut_ptr(), res);
            result
        }
    }
    #[cfg(not(any(
        all(target_arch = "wasm32", target_feature = "simd128"),
        all(target_arch = "x86_64", target_feature = "sse"),
        all(target_arch = "aarch64", target_feature = "neon")
    )))]
    {
        let mut result = [0.0f32; 4];
        for i in 0..4 {
            let c3 = -p0[i] + 3.0 * p1[i] - 3.0 * p2[i] + p3[i];
            let c2 = 2.0 * p0[i] - 5.0 * p1[i] + 4.0 * p2[i] - p3[i];
            let c1 = -p0[i] + p2[i];
            let c0 = p1[i];
            result[i] = 0.5 * (((c3 * t[i] + c2) * t[i]) + c1) * t[i] + c0;
        }
        result
    }
}



pub struct PatternChange {
    pattern_break:  bool,
    pattern_jump:   bool,
    is_loop:        bool,
    row:            u8,
    pattern:        u8,
    pattern_delay:  u8,
}

impl PatternChange {
    pub fn new() -> Self {
        Self{
            pattern_break: false,
            pattern_jump: false,
            is_loop: false,
            row: 0,
            pattern: 0,
            pattern_delay: 0,
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
            self.row = (param >> 4) * 10 + (param & 0x0F);
        } else {
            // IT, S3M use Hex
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
    pub(crate) fn new(volume: u32, song_type: SongType) -> GlobalVolume {
        GlobalVolume {
            volume,
            last_volume_slide: 0,
            song_type:         Some(song_type),
        }
    }

    pub(crate) fn get_volume_f32(&self) -> f32 {
        match self.song_type {
            Some(SongType::IT) | Some(SongType::S3M) => self.volume as f32 / 128.0,
            _ => self.volume as f32 / 64.0,
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
    pub song_type:                          SongType,
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
            song_type: SongType::XM,
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
    pub total_duration_ms:          f32,
    pub bpm:                        BPM,
    pub loop_pattern:               bool,
    pub pause:                      bool,
    pub filter:                     FilterType,
    pub display:                    bool,
    pub frequency_tables:           Box<AudioTables>,
    pub is_fast_forwarding:         bool,
    pub is_calculating_duration:    bool,
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
    pub hann_window:                Vec<f32>,
    pub cached_fft:                 Option<Arc<dyn Fft<f32>>>,
    pub bin_map:                    Vec<(usize, usize)>,
    pub stopped:                    bool,
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

        let mut result = Self {
            name: song_data.name.clone(),
            song_position: 0,
            row: 0,
            tick: 0,
            rate: sample_rate,
            original_rate: sample_rate,
            speed: song_data.tempo as u32,
            total_duration_ms: 0.0,
            bpm: BPM::new(song_data.bpm as u32, sample_rate as f32),
            global_volume: GlobalVolume::new(song_data.global_volume as u32, song_data.song_type),
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
            is_fast_forwarding: false,
            is_calculating_duration: false,
            triple_buffer_writer,
            master_samples: [0.0; 8192],
            master_samples_pos: 0,
            visual_latency: 2432,

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
            hann_window: (0..2048).map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / 2047.0).cos())).collect(),
            cached_fft: None,
            bin_map: vec![],
            stopped: false,
        };
        result.cached_fft = Some(result.fft_planner.plan_fft_forward(2048));
        result.recalculate_bin_map();
        result.total_duration_ms = result.compute_total_duration();
        result
    }

    fn compute_total_duration(&mut self) -> f32 {
        let current_display = self.display;
        let current_ff = self.is_fast_forwarding;

        self.reset();
        self.display = false;
        self.is_fast_forwarding = true;
        self.is_calculating_duration = true;
        
        let mut visited_rows = vec![false; 1024 * 512]; // 1024 orders * max 512 rows
        let max_samples = (20.0 * 60.0 * self.original_rate) as u64; // 20 mins max
        
        self.fast_forward_until(|s| {
            if s.total_samples > max_samples { return true; }
            if s.tick == 0 {
                let idx = s.song_position * 512 + s.row;
                if idx < visited_rows.len() {
                    if visited_rows[idx] {
                        return true;
                    }
                    visited_rows[idx] = true;
                }
            }
            false
        });
        
        let duration = (self.total_samples as f32 / self.original_rate) * 1000.0;
        self.reset();

        self.is_calculating_duration = false;
        self.is_fast_forwarding = current_ff;
        self.display = current_display;

        duration
    }


    pub fn reset(&mut self) {
        self.song_position = 0;
        self.row = 0;
        self.tick = 0;
        self.total_samples = 0;
        self.last_fps_sample = 0;
        self.last_display_update_sample = 0;
        self.pattern_change.reset();
        self.global_volume.volume = self.song_data.global_volume as u32;
        self.speed = self.song_data.tempo as u32;
        self.bpm.update(self.song_data.bpm as u32, self.rate);
        
        for ch in self.channels.iter_mut() {
            ch.voice_idx = None;
            ch.on = false;
        }
        for v in self.voices.iter_mut() {
            v.on = false;
        }

        self.tick_state = TickState {
            state: BufferState::Start,
            current_buf_position: 0,
            current_tick_position: 0,
            row_delay: 0,
        };
    }

    pub fn fast_forward_until<F>(&mut self, mut condition: F) 
    where F: FnMut(&Song) -> bool {
        let mut dummy_buf = vec![0.0; 32768];
        let mut adapter = InterleavedBufferAdaptar { buf: &mut dummy_buf };
        let mut rx = std::sync::mpsc::channel().1;
        
        let current_display = self.display;
        let current_ff = self.is_fast_forwarding;
        self.display = false;
        self.is_fast_forwarding = true;

        while !condition(self) {
            if let CallbackState::Complete = self.get_next_tick(&mut adapter, &mut rx) {
                // Reached end of track or loop point
                break;
            }
        }
        
        self.display = current_display;
        self.is_fast_forwarding = current_ff;
    }

    pub fn seek_forward_pattern(&mut self) {
        let current = self.song_position;
        self.fast_forward_until(|s| s.song_position > current);
    }

    pub fn seek_backward_pattern(&mut self) {
        let target = self.song_position.saturating_sub(1);
        self.reset();
        self.fast_forward_until(|s| s.song_position >= target);
    }

    pub fn seek_forward_seconds(&mut self, seconds: f32) {
        let current_frames = self.total_samples;
        let target_frames = current_frames + (seconds * self.rate) as u64;
        self.fast_forward_until(|s| s.total_samples >= target_frames);
    }

    pub fn seek_backward_seconds(&mut self, seconds: f32) {
        let current_frames = self.total_samples;
        let diff = (seconds * self.rate) as u64;
        let target_frames = current_frames.saturating_sub(diff);
        self.reset();
        self.fast_forward_until(|s| s.total_samples >= target_frames);
    }



    // fn get_linear_frequency(note: i16, fine_tune: i32, period_offset: i32) -> f32 {
    //     let period = 10.0 * 12.0 * 16.0 * 4.0 - (note * 16 * 4) as f32  - (fine_tune as f32) / 2.0 + period_offset as f32;
    //     let two = 2.0f32;
    //     let frequency = 8363.0 * two.powf((6.0 * 12.0 * 16.0 * 4.0 - period) / (12.0 * 16.0 * 4.0));
    //     frequency as f32
    // }

    fn recalculate_bin_map(&mut self) {
        let min_f = 20.0f32;
        let max_f = 20000.0f32;
        let log_min_f = min_f.ln();
        let log_max_f = max_f.ln();
        let mut new_map = Vec::with_capacity(128);

        for j in 0..128 {
            let f_start = (log_min_f + (j as f32 / 128.0) * (log_max_f - log_min_f)).exp();
            let f_end = (log_min_f + ((j + 1) as f32 / 128.0) * (log_max_f - log_min_f)).exp();
            
            let i_start = (f_start * 2048.0 / self.rate).floor() as usize;
            let i_end = (f_end * 2048.0 / self.rate).ceil() as usize;
            new_map.push((i_start, i_end));
        }
        self.bin_map = new_map;
    }

    fn queue_display(&mut self) {
        let mut play_data = self.triple_buffer_writer.acquire_buffer();
        
        play_data.name                      = self.name.clone();
        play_data.song_type                 = self.song_data.song_type;
        play_data.total_duration_ms         = self.total_duration_ms;
        play_data.current_duration_ms       = (self.total_samples as f32 / self.rate) * 1000.0;
        play_data.global_volume             = self.global_volume.volume;
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

        // --- INSTANT UI FEEDBACK ---
        play_data.theme_id         = self.theme_id;
        play_data.view_mode        = self.view_mode;
        play_data.user_data        = self.user_data.clone();
        play_data.visualizer_mode  = self.visualizer_mode;
        play_data.filter           = self.filter;

        // IT virtual channel tracking
        let active_voices = self.voices.iter().filter(|v| v.on).count();
        let mut host_voices = 0;
        for channel in &self.channels {
            if let Some(v_idx) = channel.voice_idx {
                if self.voices[v_idx].on { host_voices += 1; }
            }
        }
        play_data.virtual_channels = active_voices.saturating_sub(host_voices);

        // Optimized Channel Status
        let num_channels = self.channels.len();
        while play_data.channel_status.len() < num_channels {
            play_data.channel_status.push(ChannelStatus::default());
        }
        play_data.channel_status.truncate(num_channels);

        for i in 0..num_channels {
            let channel = &mut self.channels[i];
            let status = &mut play_data.channel_status[i];
            
            let mut peak = 0.01f32;
            for &s in channel.last_samples.iter().take(512) { peak = peak.max(s.abs()); }
            let gain = if peak > 0.0001 { 0.5 / peak } else { 1.0 };

            if status.oscilloscope.len() != 512 { status.oscilloscope = vec![0.0; 512]; }
            if channel.on {
                for j in 0..512 { status.oscilloscope[j] = channel.last_samples[j % 4096] * gain; }
            } else {
                status.oscilloscope.fill(0.0);
            }

            status.volume             = channel.volume.get_volume() as f32;
            status.on                 = channel.on;
            status.force_off          = channel.force_off;
            status.frequency          = channel.frequency;
            status.instrument         = channel.last_instrument;
            status.sample             = channel.last_sample;
            status.note               = channel.note.to_string();
            status.period             = channel.note.period;
            status.final_panning      = channel.panning.final_panning;

            // Link to active voice for dynamic visualization
            if let Some(v_idx) = channel.voice_idx {
                let voice = &self.voices[v_idx];
                if voice.on {
                    status.envelope_volume = voice.volume.envelope_vol as f32 / 16384.0;
                    status.fadeout_volume  = voice.volume.fadeout_vol as f32 / 65536.0;
                    status.global_volume   = self.global_volume.get_volume_f32();
                    status.pitch_shift     = voice.frequency;
                    
                    let inst_idx = voice.instrument;
                    let sample_idx = voice.sample;
                    if inst_idx < self.song_data.instruments.len() {
                        let inst = &self.song_data.instruments[inst_idx];
                        if sample_idx < inst.samples.len() {
                            let sample = &inst.samples[sample_idx];
                            if sample.length > 0 {
                                status.sample_position = voice.sample_position / sample.length as f32;
                            }
                        }
                    }
                } else {
                    status.envelope_volume = 1.0;
                    status.fadeout_volume  = 1.0;
                    status.pitch_shift     = 1.0;
                    status.sample_position = 0.0;
                }
            } else {
                status.envelope_volume = 1.0;
                status.fadeout_volume  = 1.0;
                status.pitch_shift     = 1.0;
                status.sample_position = 0.0;
            }
        }

        // FPS Update
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
        play_data.display_fps = self.fps;

        // High-Fidelity Master Visualizer Data with Latency Compensation (Master branch style)
        if play_data.master_oscilloscope.len() != 512 { play_data.master_oscilloscope = vec![0.0; 512]; }
        let history_len = 8192;
        let start_offset = (self.master_samples_pos as isize - self.visual_latency).rem_euclid(history_len as isize) as usize;
        for i in 0..512 {
            let idx = (start_offset + i) % history_len;
            play_data.master_oscilloscope[i] = self.master_samples[idx];
        }
        
        // Master FFT (Optimized with persistent planner)
        if let Some(fft) = self.cached_fft.as_ref() {
            let mut fft_input_buffer = vec![0.0f32; 2048];
            let base_offset = (start_offset as isize - 512).rem_euclid(history_len as isize) as usize;
            for i in 0..2048 {
                let idx = (base_offset + i) % history_len; 
                fft_input_buffer[i] = self.master_samples[idx] * self.hann_window[i];
            }
            
            let mut fft_buffer: Vec<Complex<f32>> = fft_input_buffer.iter().map(|&s| Complex::new(s, 0.0)).collect();
            fft.process(&mut fft_buffer);

            if play_data.master_spectrum.len() != 128 { play_data.master_spectrum = vec![0.0; 128]; }
            let decay = if cfg!(target_arch = "wasm32") { 0.88f32 } else { 0.92f32 };

            for j in 0..128 {
                let (i_s, i_e) = self.bin_map[j];
                let mut magnitude = 0.0f32;
                for i in i_s..i_e {
                    let m = fft_buffer[i.min(1023)].norm() / 20.0;
                    if m > magnitude { magnitude = m; }
                }
                self.spectral_peaks[j] = magnitude.max(self.spectral_peaks[j] * decay);
                play_data.master_spectrum[j] = self.spectral_peaks[j];
            }
        }
    }
    // Song::display(&play_data, 0);

    pub fn get_channel_count(&self) -> usize {
        self.song_data.channel_count as usize
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
                        self.seek_forward_pattern();
                    }
                    PlaybackCmd::Prev => {
                        self.seek_backward_pattern();
                    }
                    PlaybackCmd::SeekForward10s => {
                        self.seek_forward_seconds(10.0);
                    }
                    PlaybackCmd::SeekBackward10s => {
                        self.seek_backward_seconds(10.0);
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
                    PlaybackCmd::ChannelToggle(channel) => {
                        if (channel as usize) < self.channels.len() {
                            self.channels[channel as usize].force_off = !self.channels[channel as usize].force_off;
                        }
                    }
                    PlaybackCmd::ChannelSolo(channel_idx) => {
                        if (channel_idx as usize) < self.channels.len() {
                            for (i, channel) in self.channels.iter_mut().enumerate() {
                                channel.force_off = i != channel_idx as usize;
                            }
                        }
                    }
                    PlaybackCmd::ChannelUnmuteAll => {
                        for channel in self.channels.iter_mut() {
                            channel.force_off = false;
                        }
                    }
                    PlaybackCmd::ChannelMuteAll => {
                        for channel in self.channels.iter_mut() {
                            channel.force_off = true;
                        }
                    }
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
                        self.user_data.entry(key).and_modify(|data| {
                            if let UserData::ISize(x) = data {
                                *x = x.saturating_add(value);
                            }
                        });
                    }
                    PlaybackCmd::ModifyUserDataSubISize(key, value) => {
                        self.user_data.entry(key).and_modify(|data| {
                            if let UserData::ISize(x) = data {
                                *x = x.saturating_sub(value);
                            }
                        });
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
                        self.theme_id = (self.theme_id + 1) % 6;

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
        if self.song_position as usize >= self.song_data.pattern_order.len() {
            return false;
        }
        return true;
    }

    pub(crate) fn next_tick(&mut self) -> bool {
        if self.song_position >= self.song_data.song_length as usize {
            return false;
        }

        self.tick += 1;
        
        let speed = self.speed * (self.tick_state.row_delay as u32 + 1);

        if self.tick >= speed {
            self.tick_state.row_delay = 0;
            
            // Handle Pattern Delay (EEx)
            if self.pattern_change.pattern_delay > 0 {
                self.pattern_change.pattern_delay -= 1;
                self.tick = 0;
                return true;
            }

            if self.pattern_change.pattern_break || self.pattern_change.pattern_jump || self.pattern_change.is_loop {
                if self.pattern_change.is_loop {
                    // Stay in same pattern
                } else if !self.pattern_change.pattern_jump {
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
        let patterns = &self.song_data.patterns[self.song_data.pattern_order[self.song_position] as usize];
        let row = &patterns.rows[self.row];
        let first_tick = self.tick == 0;

        // Hyper-optimization for duration calculation:
        // Skip all expensive effect processing and only handle flow control.
        if self.is_calculating_duration {
            for pattern in &row.channels {
                match self.song_data.song_type {
                    SongType::IT | SongType::S3M => {
                        match pattern.effect {
                            0x02 => { self.pattern_change.set_jump(first_tick, pattern.effect_param); } // B: Jump
                            0x03 => { self.pattern_change.set_break(self.song_data.song_type, first_tick, pattern.effect_param); } // C: Break
                            0x01 | 0x14 => { // A/T Speed/BPM
                                if first_tick && pattern.effect_param > 0 {
                                    if pattern.effect_param <= 32 { self.speed = pattern.effect_param as u32; }
                                    else { self.bpm.update(pattern.effect_param as u32, self.rate); }
                                }
                            }
                            0x13 => { // S
                                let x = pattern.get_x();
                                let y = pattern.get_y();
                                if x == 0x6 && first_tick && y != 0 { self.pattern_change.is_loop = true; }
                                else if x == 0xE && first_tick { self.pattern_change.pattern_delay = y; }
                            }
                            _ => {}
                        }
                    }
                    _ => { // XM / MOD
                        match pattern.effect {
                            0x0B => { self.pattern_change.set_jump(first_tick, pattern.effect_param); } // B: Jump
                            0x0D => { self.pattern_change.set_break(self.song_data.song_type, first_tick, pattern.effect_param); } // D: Break
                            0x0F => { // F: Speed/BPM (XM/MOD) or A/T (S3M)
                                if first_tick {
                                    if pattern.effect_param > 0 {
                                        if pattern.effect_param <= 32 { self.speed = pattern.effect_param as u32; }
                                        else { self.bpm.update(pattern.effect_param as u32, self.rate); }
                                    } else if self.song_data.song_type == SongType::S3M {
                                        // S3M A00/T00 memory is handled by doing nothing (preserving last value)
                                    }
                                }
                            }
                            0x0E => { // E
                                let x = pattern.get_x();
                                let y = pattern.get_y();
                                if x == 0x6 && first_tick && y != 0 { self.pattern_change.is_loop = true; }
                                else if x == 0xE && first_tick { self.pattern_change.pattern_delay = y; }
                            }
                            _ => {}
                        }
                    }
                }
            }
            return;
        }

        let instruments = &self.song_data.instruments;

        // 1. Process channels (Note trigger and Effects)
        for i in 0..self.channels.len() {
            let pattern = &row.channels[i];
            let channel = &mut self.channels[i];

            // Lazy cleanup: if the voice we were tracking was stolen by another channel, detach it now.
            // Note: In multi-voice trackers (IT/S3M), a channel may have multiple active voices.
            // We no longer perform aggressive 'force detach' logic here to prevent flickering.
            // channel.voice_idx is still used to track the 'main' controllable voice.

            let note_delay_first_tick = if pattern.is_note_delay(self.song_data.song_type) { self.tick == pattern.get_y() as u32 } else { first_tick };

            if pattern.is_porta_to_note(self.song_data.song_type) && first_tick && is_note_valid(pattern.note, self.song_data.song_type) {
                channel.porta_to_note.target_note.period = channel.note.note_to_period(pattern.note, 0, self.frequency_tables.as_ref());
            }

            if !pattern.is_porta_to_note(self.song_data.song_type) &&
                ((pattern.is_note_delay(self.song_data.song_type) && self.tick == pattern.get_y() as u32) ||
                 (!pattern.is_note_delay(self.song_data.song_type) && first_tick)) {
                
                let note = pattern.note;
                let mut inst_idx = channel.last_instrument;
                if pattern.instrument != 255 && pattern.instrument != 0 {
                    inst_idx = if (pattern.instrument as usize) < instruments.len() { pattern.instrument as usize } else { 0 };
                    channel.last_instrument = inst_idx;
                }

                if is_note_valid(note, self.song_data.song_type) {
                    println!("DEBUG: Note Trigger detected: note={}, inst={}", note, pattern.instrument);
                    // IT Duplicate Check (DCT/DCA)
                    if let SongType::IT = self.song_data.song_type {
                        if inst_idx != 0 {
                            let new_inst = &instruments[inst_idx];
                            let mut dca_applied = false;

                            // Check all active voices for duplicates on this host channel
                            for vi in 0..self.voices.len() {
                                let v = &mut self.voices[vi];
                                if !v.on || v.channel_idx != i { continue; }

                                match new_inst.dct {
                                    1 => { // Note match
                                        if v.last_played_note == note {
                                            Self::apply_it_action(&mut self.voices, vi, new_inst.dca, new_inst);
                                            dca_applied = true;
                                        }
                                    }
                                    2 => { // Sample match
                                        let sample_idx = if (note as usize - 1) < new_inst.sample_indexes.len() {
                                            new_inst.sample_indexes[note as usize - 1].1
                                        } else { 0 };
                                        
                                        if sample_idx > 0 && v.sample == (sample_idx - 1) as usize && v.instrument == inst_idx {
                                            Self::apply_it_action(&mut self.voices, vi, new_inst.dca, new_inst);
                                            dca_applied = true;
                                        }
                                    }
                                    3 => { // Instrument match
                                        if v.instrument == inst_idx {
                                            Self::apply_it_action(&mut self.voices, vi, new_inst.dca, new_inst);
                                            dca_applied = true;
                                        }
                                    }
                                    _ => {}
                                }
                            }

                            // If no DCT matched, apply NNA to current voice if it exists
                            if !dca_applied {
                                if let Some(v_idx) = channel.voice_idx {
                                    if self.voices[v_idx].on {
                                        Self::apply_it_action(&mut self.voices, v_idx, new_inst.nna, new_inst);
                                    }
                                }
                            }
                        }
                    } else if let Some(old_v_idx) = channel.voice_idx {
                        // Standard XM/S3M/MOD NNA (Cut/Off/Fade)
                        if self.voices[old_v_idx].instrument < instruments.len() && is_note_valid(note, self.song_data.song_type) {
                            let instrument_ref = &instruments[self.voices[old_v_idx].instrument];
                            match instrument_ref.nna {
                                0 => { self.voices[old_v_idx].on = false; }
                                1 => { self.voices[old_v_idx].key_off(instruments, pattern.is_note_delay(self.song_data.song_type)); }
                                2 => {
                                    self.voices[old_v_idx].sustained = false;
                                    self.voices[old_v_idx].volume.fadeout_speed = (instrument_ref.volume_fadeout as i32) << 6;
                                }
                                _ => { self.voices[old_v_idx].key_off(instruments, pattern.is_note_delay(self.song_data.song_type)); }
                            }
                        }
                    }

                    // Start a new voice
                    let note_idx = (note - 1) as usize;
                    let mut trigger_voice = false;
                    let mut final_sample_idx = 0;
                    let mut mapped_note = note;

                    if inst_idx != 0 && inst_idx < instruments.len() {
                        let instrument = &instruments[inst_idx];
                        let mut mapping_found = false;

                        if note_idx < instrument.sample_indexes.len() {
                            let it_mapping = instrument.sample_indexes[note_idx];
                            let sample_idx = it_mapping.1 as usize;
                            
                            if sample_idx > 0 {
                                final_sample_idx = sample_idx - 1;
                                if final_sample_idx < instrument.samples.len() {
                                    trigger_voice = true;
                                    mapping_found = true;
                                    if let SongType::IT = self.song_data.song_type {
                                        mapped_note = it_mapping.0 + 1;
                                    }
                                }
                            }
                        }
                        
                        if !mapping_found && !instrument.samples.is_empty() {
                            // FALLBACK: Default to first sample if no mapping exists (XM/MOD/S3M)
                            final_sample_idx = 0;
                            trigger_voice = true;
                        }
                    }

                    if trigger_voice && (self.song_data.song_type != SongType::S3M || note < 97) {
                        let sample_len = instruments[inst_idx].samples[final_sample_idx].length;
                        let data_len = instruments[inst_idx].samples[final_sample_idx].data.len();
                        // Find free voice or steal quietest
                        let mut v_idx = 0;
                        let mut found = false;
                        for vi in 0..self.voices.len() {
                            if !self.voices[vi].on { v_idx = vi; found = true; break; }
                        }
                        if !found {
                            let mut min_vol = 1_000_000.0f32;
                            for vi in 0..self.voices.len() {
                                if self.voices[vi].volume.output_volume < min_vol {
                                    min_vol = self.voices[vi].volume.output_volume;
                                    v_idx = vi;
                                }
                            }
                        }
                        
                        let mut clone_voice = None;
                        if (self.song_data.song_type == SongType::IT || self.song_data.song_type == SongType::S3M) && pattern.instrument == 0 {
                            if let Some(old_idx) = channel.voice_idx {
                                clone_voice = Some(self.voices[old_idx].clone());
                            }
                        }
                        
                        let voice = &mut self.voices[v_idx];
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
                            let sample_ref = &instruments[inst_idx].samples[final_sample_idx];
                            voice.volume.retrig(sample_ref.volume as i32);
                            voice.panning.panning = sample_ref.panning;
                        }

                        channel.voice_idx = Some(v_idx);
                        channel.last_played_note = note;
                        channel.on = true;
                        
                        let voice = &mut self.voices[v_idx];
                        let sample_ref = &instruments[inst_idx].samples[final_sample_idx];
                        voice.surround = sample_ref.surround;
                        let real_note = (mapped_note as i16 + sample_ref.relative_note as i16) as u8;
                        channel.note.set_note(real_note, sample_ref.finetune, mapped_note, self.frequency_tables.as_ref());
                        channel.update_frequency_voice(voice, self.rate, false, self.frequency_tables.as_ref());
                    }
                }

                if note == 97 { // Note Off
                    if let Some(v_idx) = channel.voice_idx {
                        self.voices[v_idx].key_off(instruments, pattern.is_note_delay(self.song_data.song_type));
                    }
                } else if note == 121 { // Note Cut
                    if let Some(v_idx) = channel.voice_idx {
                        self.voices[v_idx].on = false;
                        self.voices[v_idx].volume.output_volume = 0.0;
                    }
                } else if note == 122 { // Note Fade
                    if let Some(v_idx) = channel.voice_idx {
                        self.voices[v_idx].sustained = false;
                        let inst_ref = &instruments[self.voices[v_idx].instrument];
                        self.voices[v_idx].volume.fadeout_speed = (inst_ref.volume_fadeout as i32) << 6;
                    }
                }
            }

            // Effects logic
            let mut voice_ptr: *mut Voice = std::ptr::null_mut();
            if let Some(v_idx) = channel.voice_idx {
                if self.voices[v_idx].channel_idx == i {
                    voice_ptr = &mut self.voices[v_idx];
                }
            }
                
            if !first_tick && pattern.has_vibrato(self.song_data.song_type) {
                unsafe { if !voice_ptr.is_null() { channel.vibrato(Some(&mut *voice_ptr), first_tick, pattern.get_vibrato_speed(), pattern.get_vibrato_depth(), self.old_effects, self.frequency_tables.as_ref()); } }
            }

            match self.song_data.song_type {
                SongType::IT | SongType::S3M => {
                    if pattern.volume != 255 {
                        match pattern.volume {
                            0..=64 => { unsafe { channel.set_volume(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, note_delay_first_tick, pattern.volume); } }
                            65..=74 => { unsafe { channel.fine_volume_slide(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, note_delay_first_tick, (pattern.volume - 65) as i8); } }
                            75..=84 => { unsafe { channel.fine_volume_slide(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, note_delay_first_tick, -((pattern.volume - 75) as i8)); } }
                            85..=94 => { unsafe { channel.volume_slide(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, note_delay_first_tick, (pattern.volume - 85) as i8); } }
                            95..=104 => { unsafe { channel.volume_slide(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, note_delay_first_tick, -((pattern.volume - 95) as i8)); } }
                            105..=114 => { channel.porta_up(self.song_data.song_type, first_tick, (pattern.volume - 105) << 2); }
                            115..=124 => { channel.porta_down(self.song_data.song_type, first_tick, (pattern.volume - 115) << 2); }
                            128..=192 => { unsafe { if !voice_ptr.is_null() { (*voice_ptr).panning.set_panning(((pattern.volume - 128) << 2) as i32); } } } 
                            193..=202 => { channel.porta_up(self.song_data.song_type, first_tick, pattern.volume - 192); }
                            203..=212 => { channel.porta_down(self.song_data.song_type, first_tick, pattern.volume - 202); }
                            _ => {}
                        }
                    }

                    match pattern.effect {
                        0x01 => { 
                            if first_tick && pattern.effect_param > 0 { 
                                if pattern.effect_param < 32 { self.speed = pattern.effect_param as u32; }
                                else { self.bpm.update(pattern.effect_param as u32, self.rate); }
                            } 
                        }
                        0x02 => { self.pattern_change.set_jump(first_tick, pattern.effect_param); }
                        0x03 => { self.pattern_change.set_break(self.song_data.song_type, first_tick, pattern.effect_param); }
                        0x04 => { 
                            unsafe { channel.it_volume_slide(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, note_delay_first_tick, pattern.effect_param); }
                        }
                        0x05 => { 
                            let param = if !self.compatible_g && pattern.effect_param == 0 { channel.last_it_slide_speed } else { pattern.effect_param };
                            if !self.compatible_g && pattern.effect_param != 0 { channel.last_it_slide_speed = pattern.effect_param; }
                            channel.porta_down(self.song_data.song_type, first_tick, param); 
                        }
                        0x06 => { 
                            let param = if !self.compatible_g && pattern.effect_param == 0 { channel.last_it_slide_speed } else { pattern.effect_param };
                            if !self.compatible_g && pattern.effect_param != 0 { channel.last_it_slide_speed = pattern.effect_param; }
                            channel.porta_up(self.song_data.song_type, first_tick, param); 
                        }
                        0x07 => { 
                            let param = if !self.compatible_g && pattern.effect_param == 0 { channel.last_it_slide_speed } else { pattern.effect_param };
                            if !self.compatible_g && pattern.effect_param != 0 { channel.last_it_slide_speed = pattern.effect_param; }
                            unsafe { channel.porta_to_note(self.song_data.song_type, if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, first_tick, param, self.compatible_g, self.frequency_tables.as_ref()); }
                        }
                        0x08 => { unsafe { channel.vibrato(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, first_tick, pattern.get_x(), pattern.get_y(), self.old_effects, self.frequency_tables.as_ref()); } }
                        0x0A => { channel.arpeggio(self.tick, pattern.get_x(), pattern.get_y()); }
                        0x0B => { 
                            unsafe { channel.vibrato(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, first_tick, 0, 0, self.old_effects, self.frequency_tables.as_ref()); }
                            unsafe { channel.it_volume_slide(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, note_delay_first_tick, pattern.effect_param); }
                        }
                        0x0C => { 
                            unsafe { channel.porta_to_note(self.song_data.song_type, if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, first_tick, 0, self.compatible_g, self.frequency_tables.as_ref()); }
                            unsafe { channel.it_volume_slide(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, note_delay_first_tick, pattern.effect_param); }
                        }
                        0x0F => {
                            if first_tick && is_note_valid(channel.last_played_note, self.song_data.song_type) {
                                if pattern.effect_param != 0 { channel.last_sample_offset = (pattern.effect_param as u32) << 8; }
                                unsafe { if !voice_ptr.is_null() {
                                    let v = &mut *voice_ptr;
                                    let offset = channel.last_sample_offset;
                                    let sample_ref = &self.song_data.instruments[v.instrument].samples[v.sample];
                                    let orig_length = sample_ref.length.saturating_sub(4);
                                    if offset >= orig_length { v.on = false; } else { v.sample_position = offset as f32 + 4.0; }
                                }}
                            }
                        }
                        0x11 => { unsafe { channel.it_retrig(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, instruments, self.tick, pattern.effect_param); } }
                        0x14 => { if first_tick && pattern.effect_param > 0 { self.bpm.update(pattern.effect_param as u32, self.rate); } }
                        0x16 => { self.global_volume.set_volume(note_delay_first_tick, pattern.effect_param); }
                        0x17 => { self.global_volume.volume_slide(note_delay_first_tick, pattern.effect_param); }
                        0x18 => { if first_tick { unsafe { if !voice_ptr.is_null() { (*voice_ptr).panning.set_panning((pattern.effect_param as i32 * 4).min(255)); } } } }
                        0x13 => {
                            let x = pattern.get_x();
                            match x {
                                0x08 => { if first_tick { unsafe { if !voice_ptr.is_null() { (*voice_ptr).panning.set_panning((pattern.get_y() << 4) as i32); } } } }
                                0x0C => { if self.tick == pattern.get_y() as u32 { channel.on = false; unsafe { if !voice_ptr.is_null() { (*voice_ptr).on = false; } } } }
                                0x0E => { if first_tick { self.tick_state.row_delay = pattern.get_y() as usize; } }
                                _ => {}
                            }
                        }
                        0x0D => { if first_tick { channel.channel_volume = pattern.effect_param.min(64); } }
                        0x0E => { channel.channel_volume_slide(note_delay_first_tick, pattern.effect_param); }
                        0x10 => { unsafe { channel.panning_slide(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, note_delay_first_tick, pattern.effect_param); } }
                        0x1A => {
                            unsafe { if !voice_ptr.is_null() {
                                if pattern.effect_param < 0x80 { (*voice_ptr).filter_cutoff = pattern.effect_param; }
                                else if (0x80..=0x8F).contains(&pattern.effect_param) { (*voice_ptr).filter_resonance = (pattern.effect_param & 0x0F) << 3; }
                            }}
                        }
                        _ => {}
                    }
                }
                _ => { // XM / MOD
                    match pattern.volume {
                        0x10..=0x50 => { unsafe { channel.set_volume(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, note_delay_first_tick, pattern.volume - 0x10); } }
                        0x60..=0x6f => { unsafe { channel.volume_slide(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, note_delay_first_tick, -(pattern.get_volume_param() as i8)); } }
                        0x70..=0x7f => { unsafe { channel.volume_slide(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, note_delay_first_tick, pattern.get_volume_param() as i8); } }
                        0x80..=0x8f => { unsafe { channel.fine_volume_slide(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, note_delay_first_tick, -(pattern.get_volume_param() as i8)); } }
                        0x90..=0x9f => { unsafe { channel.fine_volume_slide(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, note_delay_first_tick, pattern.get_volume_param() as i8); } }
                        0xa0..=0xaf => { unsafe { channel.vibrato(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, first_tick, 0, pattern.get_volume_param(), self.old_effects, self.frequency_tables.as_ref()); } }
                        0xb0..=0xbf => { unsafe { channel.vibrato(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, first_tick, pattern.get_volume_param(), 0, self.old_effects, self.frequency_tables.as_ref()); } }
                        0xd0..=0xdf => { unsafe { channel.panning_slide(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, note_delay_first_tick, pattern.get_volume_param() << 4); } }
                        0xe0..=0xef => { unsafe { channel.panning_slide(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, note_delay_first_tick, pattern.get_volume_param()); } }
                        0xf0..=0xff => { unsafe { channel.porta_to_note(self.song_data.song_type, if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, note_delay_first_tick, pattern.get_volume_param(), self.compatible_g, self.frequency_tables.as_ref()); } }
                        _ => {}
                    }

                    match pattern.effect {
                        0x0 => { channel.arpeggio(self.tick, pattern.get_x(), pattern.get_y()); }
                        0x1 => { channel.porta_up(self.song_data.song_type, first_tick, pattern.effect_param); }
                        0x2 => { channel.porta_down(self.song_data.song_type, first_tick, pattern.effect_param); }
                        0x3 => { unsafe { channel.porta_to_note(self.song_data.song_type, if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, first_tick, pattern.effect_param, self.compatible_g, self.frequency_tables.as_ref()); } }
                        0x4 => { unsafe { channel.vibrato(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, first_tick, pattern.get_x(), pattern.get_y(), self.old_effects, self.frequency_tables.as_ref()); } }
                        0x5 => { 
                            unsafe { channel.porta_to_note(self.song_data.song_type, if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, first_tick, 0, self.compatible_g, self.frequency_tables.as_ref()); }
                            unsafe { channel.volume_slide_main(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, first_tick, pattern.effect_param); }
                        }
                        0x6 => { 
                            unsafe { channel.vibrato(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, first_tick, 0, 0, self.old_effects, self.frequency_tables.as_ref()); }
                            unsafe { channel.volume_slide_main(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, first_tick, pattern.effect_param); }
                        }
                        0x7 => { unsafe { channel.tremolo(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, first_tick, pattern.get_x(), pattern.get_y()); } }
                        0x8 => { unsafe { if !voice_ptr.is_null() { (*voice_ptr).panning.set_panning(pattern.effect_param as i32); } } }
                        0x9 => {
                            if first_tick && is_note_valid(channel.last_played_note, self.song_data.song_type) {
                                if pattern.effect_param != 0 { channel.last_sample_offset = (pattern.effect_param as u32) << 8; }
                                unsafe { if !voice_ptr.is_null() {
                                    let v = &mut *voice_ptr;
                                    let offset = channel.last_sample_offset;
                                    let sample_ref = &self.song_data.instruments[v.instrument].samples[v.sample];
                                    if offset >= sample_ref.length.saturating_sub(4) { v.on = false; } else { v.sample_position = offset as f32 + 4.0; }
                                }}
                            }
                        }
                        0xA => { unsafe { channel.volume_slide_main(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, note_delay_first_tick, pattern.effect_param); } }
                        0xB => { self.pattern_change.set_jump(first_tick, pattern.effect_param); }
                        0xD => { self.pattern_change.set_break(self.song_data.song_type, first_tick, pattern.effect_param); }
                        0x0F => {
                            if first_tick {
                                if pattern.effect_param != 0 {
                                    if pattern.effect_param < 32 { self.speed = pattern.effect_param as u32; }
                                    else { self.bpm.update(pattern.effect_param as u32, self.rate); }
                                }
                            }
                        }
                        0x10 => { self.global_volume.set_volume(note_delay_first_tick, pattern.effect_param); }
                        0x11 => { self.global_volume.volume_slide(first_tick, pattern.effect_param); }
                        0xE => {
                            let sub = pattern.get_x();
                            let p = pattern.get_y();
                            match sub {
                                0x1 => { channel.fine_porta_up(self.song_data.song_type, first_tick, p); }
                                0x2 => { channel.fine_porta_down(self.song_data.song_type, first_tick, p); }
                                0x3 => { channel.glissando = p == 1; }
                                0x4 => { channel.vibrato_control = p; }
                                0x7 => { channel.tremolo_control = p; }
                                0x9 => { unsafe { channel.retrig_note(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, instruments, first_tick, self.tick, p); } }
                                0xA => { unsafe { channel.fine_volume_slide(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, first_tick, p as i8); } }
                                0xB => { unsafe { channel.fine_volume_slide(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, first_tick, -(p as i8)); } }
                                0xC => { if self.tick == p as u32 { unsafe { if !voice_ptr.is_null() { (*voice_ptr).on = false; } } } }
                                _ => {}
                            }
                        }
                        0x1B => { unsafe { channel.multi_retrig(if voice_ptr.is_null() { None } else { Some(&mut *voice_ptr) }, instruments, first_tick, self.tick, pattern.effect_param); } }
                        0x1D => { channel.tremor(self.tick, pattern.effect_param); }
                        _ => {}
                    }
                }
            }
                
            if !first_tick && pattern.effect != 0x0 && pattern.effect != 0x0A {
                channel.period_shift = 0;
            }

            let mut channel_active = false;
            for v in &self.voices {
                if v.on && v.channel_idx == i {
                    channel_active = true;
                    break;
                }
            }
            channel.on = channel_active;
        }

        // 2. Process all active voices (Envelopes and Final Volume)
        // IT uses 0..128, others use 0..64
        // IT uses 0..128, others use 0..64
        let global_divisor = if self.song_data.song_type == SongType::IT { 128.0 } else { 64.0 };
        let global_vol_f32 = self.global_volume.volume as f32 / global_divisor;
        for (v_idx, voice) in self.voices.iter_mut().enumerate() {
            if !voice.on { continue; }
            let channel_vol_f32 = self.channels[voice.channel_idx].channel_volume as f32 / 64.0;
            voice.update_envelopes(instruments, self.rate);
            voice.update_output_volume(global_vol_f32, channel_vol_f32, 1.0);
            
            let is_host_voice = self.channels[voice.channel_idx].voice_idx == Some(v_idx);
            
            if !voice.sustained && (voice.volume.fadeout_vol == 0 || voice.volume.output_volume < 0.00001) {
                voice.on = false;
            } else if !is_host_voice && voice.volume.output_volume < 0.00001 {
                voice.on = false;
            }
        }
    }

    fn lerp(pos: f32, p1: f32, p2: f32) -> f32 {
        let t = pos.fract();
        (1.0 - t) * p1 + t * p2
    }

    fn output_channels(&mut self, current_buf_position: usize, buf: &mut impl BufferAdapter, ticks_to_generate: usize) {
        if self.is_fast_forwarding {
            return;
        }

        let master_gain = (self.master_volume as f32 / 128.0) * (self.mixing_volume as f32 / 128.0) * MASTER_GAIN;
        let frequency_tables = self.frequency_tables.as_ref();
        let song_type = self.song_data.song_type;

        // Clear per-channel last_samples for this buffer block
        for channel in &mut self.channels {
            channel.last_samples.fill(0.0);
        }

        for v_idx in 0..self.voices.len() {
            let voice = &mut self.voices[v_idx];
            if !voice.on { continue; }
            
            let final_master_gain = if song_type == SongType::IT || song_type == SongType::S3M || song_type == SongType::XM { master_gain } else { MASTER_GAIN };
            let sample = &self.song_data.instruments[voice.instrument].samples[voice.sample];

            let vol_right = (PANNING_TAB[      voice.panning.final_panning as usize] as f32 / 65536.0) * final_master_gain;
            let vol_left  = (PANNING_TAB[256 - voice.panning.final_panning as usize] as f32 / 65536.0) * final_master_gain;
            
            let mut i = 0;
            let filter_active = voice.filter_cutoff < 127;

            let channel_volume = self.channels[voice.channel_idx].channel_volume as f32 / 64.0;
            let global_volume  = self.global_volume.get_volume_f32();
            
            voice.update_output_volume(global_volume, channel_volume, 1.0);


            // Paths 1: SIMD-optimized resampling for clean voices (Master branch style)
            if !filter_active && i + 4 <= ticks_to_generate {
                let mut pos = voice.sample_position;
                let du = voice.du;

                while voice.on && i + 4 <= ticks_to_generate {
                    // Check if the next 4 samples will drift out of bounds
                    if pos as u32 >= sample.length || pos + (4.0 * du) >= sample.length as f32 ||
                       (sample.loop_type != LoopType::NoLoop && pos + (4.0 * du) >= sample.loop_end as f32) {
                        break;
                    }

                    let mut out_samples = [0.0f32; 4];
                    match self.filter {
                        FilterType::Linear => {
                            let mut lo = [0.0f32; 4]; let mut hi = [0.0f32; 4]; let mut t = [0.0f32; 4];
                            for j in 0..4 {
                                let p = pos + (j as f32 * du);
                                let idx = p as usize;
                                lo[j] = sample.data[idx];
                                hi[j] = sample.data[idx+1];
                                t[j]  = p.fract();
                            }
                            out_samples = lerp_simd(lo, hi, t);
                        },
                        FilterType::Cubic => {
                            let mut p0 = [0.0f32; 4]; let mut p1 = [0.0f32; 4]; let mut p2 = [0.0f32; 4]; let mut p3 = [0.0f32; 4]; let mut t = [0.0f32; 4];
                            for j in 0..4 {
                                let p = pos + (j as f32 * du);
                                let idx = p as usize;
                                p0[j] = sample.data[idx.saturating_sub(1)];
                                p1[j] = sample.data[idx];
                                p2[j] = sample.data[idx+1];
                                p3[j] = sample.data[if idx+2 < sample.data.len() { idx+2 } else { idx+1 }];
                                t[j]  = p.fract();
                            }
                            out_samples = cubic_simd(p0, p1, p2, p3, t);
                        },
                        FilterType::Sinc => {
                            for j in 0..4 {
                                let p = pos + (j as f32 * du);
                                let idx = p as usize;
                                let phase = (p.fract() * 512.0) as usize;
                                let table = &frequency_tables.resampling.sinc_table[phase];
                                out_samples[j] = sinc_dot_product(&sample.data[idx.saturating_sub(3)..], table);
                            }
                        },
                        FilterType::None => {
                            for j in 0..4 { out_samples[j] = sample.data[(pos + (j as f32 * du)) as usize]; }
                        }
                    }

                    let output_vol = voice.volume.output_volume;
                    let mut left_samples  = [0.0f32; 4];
                    let mut right_samples = [0.0f32; 4];
                    
                    let channel_idx = voice.channel_idx;
                    for j in 0..4 {
                        let final_sample = out_samples[j] * output_vol;
                        left_samples[j] = final_sample * vol_left;
                        right_samples[j] = final_sample * vol_right;
                        
                        // Record for UI
                        self.channels[channel_idx].last_samples[(i + j) % 4096] += final_sample;
                        let m_idx = (self.master_samples_pos + i + j) % 8192;
                        self.master_samples[m_idx] += (left_samples[j] + right_samples[j]) / (2.0 * MASTER_GAIN);
                    }
                    
                    buf.mix_samples(0, &left_samples, current_buf_position + i);
                    buf.mix_samples(1, &right_samples, current_buf_position + i);
                    pos += 4.0 * du;
                    i += 4;
                }
                voice.sample_position = pos;
            }

            // Path 2: Scalar resampling (Legacy / Filtered / Boundary cases)
            while i < ticks_to_generate {
                if voice.sample_position as u32 >= sample.length {
                    voice.on = false;
                    break;
                }

                let pos = voice.sample_position;
                let idx = pos as usize;
                
                let mut out_sample = match self.filter {
                    FilterType::Linear => {
                        let next = if idx + 1 < sample.data.len() { sample.data[idx+1] } else { 0.0 };
                        Self::lerp(pos, sample.data[idx], next)
                    },
                    FilterType::Cubic => {
                        voice.spline_data.p0 = sample.data[idx.saturating_sub(1)];
                        voice.spline_data.p1 = sample.data[idx];
                        voice.spline_data.p2 = sample.data[(idx + 1).min(sample.data.len() - 1)];
                        voice.spline_data.p3 = sample.data[(idx + 2).min(sample.data.len() - 1)];
                        voice.spline_data.interpolate(pos.fract())
                    },
                    FilterType::Sinc => {
                        let phase = (pos.fract() * 512.0) as usize;
                        let table = &frequency_tables.resampling.sinc_table[phase];
                        sinc_dot_product(&sample.data[idx.saturating_sub(3)..], table)
                    },
                    FilterType::None => sample.data[idx]
                };

                if voice.filter_cutoff < 127 {
                    let band = voice.filter_state.history[1];
                    let low = voice.filter_state.history[0];
                    let new_band = band + voice.filter_state.a * (out_sample - low - voice.filter_state.b * band);
                    let new_low = low + voice.filter_state.a * new_band;
                    voice.filter_state.history[0] = new_low;
                    voice.filter_state.history[1] = new_band;
                    out_sample = new_low;
                }

                let final_sample = out_sample * voice.volume.output_volume;
                let l = final_sample * vol_left;
                let r = final_sample * vol_right;

                self.channels[voice.channel_idx].last_samples[i % 4096] += final_sample;
                let m_idx = (self.master_samples_pos + i) % 8192;
                self.master_samples[m_idx] += (l + r) / (2.0 * MASTER_GAIN);

                buf.mix_sample(0, l, current_buf_position + i);
                buf.mix_sample(1, r, current_buf_position + i);

                voice.sample_position += voice.du;
                if voice.sample_position as u32 >= sample.length || (sample.loop_type != LoopType::NoLoop && voice.sample_position >= sample.loop_end as f32) {
                    if sample.loop_type == LoopType::NoLoop { voice.on = false; break; }
                    else { voice.sample_position = (voice.sample_position - sample.loop_end as f32) + sample.loop_start as f32; }
                }
                i += 1;
            }
        }

        // Finalize visualizer pointers and clear master samples for next block
        let next_pos = (self.master_samples_pos + ticks_to_generate) % 8192;
        // Optimization: only clear the part we just advanced over
        let mut p = next_pos;
        for _ in 0..ticks_to_generate {
            self.master_samples[p] = 0.0;
            p = (p + 1) % 8192;
        }
        self.master_samples_pos = next_pos;

        for channel in &mut self.channels {
            channel.last_samples_pos = 0; // Fixed per-tick base for unmerged branch simplicity
        }
    }
}
