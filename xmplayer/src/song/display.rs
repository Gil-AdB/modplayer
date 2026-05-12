// Per-tick display/visualizer publish: per-channel scopes, master oscilloscope
// and FFT spectrum. queue_display fills the next PlayData buffer; the consumer
// renders it on the UI thread.

use rustfft::num_complex::Complex;

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use crate::module_reader::FrequencyType;
use crate::song::{ChannelStatus, Song, UserData};

impl Song {
    pub(super) fn recalculate_bin_map(&mut self) {
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

    pub(super) fn queue_display(&mut self) {
        let mut play_data = self.triple_buffer_writer.acquire_buffer();

        play_data.name                      = self.name.clone();
        play_data.file_name                 = self.file_name.clone();
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
        play_data.song_message              = self.song_message.clone();

        play_data.visualizer_enabled        = match self.user_data.get("visualizer_enabled") {
            Some(UserData::USize(v)) => *v % 2 != 0,
            _ => true
        };
        play_data.scopes_enabled            = match self.user_data.get("scopes_enabled") {
            Some(UserData::USize(v)) => *v % 2 != 0,
            _ => true
        };
        play_data.visualizer_mode           = self.visualizer_mode;
        play_data.filter                    = self.filter;
        play_data.theme_id                  = self.theme_id;
        play_data.view_mode                 = self.view_mode;
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
        play_data.dump = crate::song::test_dump::dump_tick(self);

        // Optimized Channel Status (In-place update)
        let num_channels = self.channels.len();
        while play_data.channel_status.len() < num_channels {
            play_data.channel_status.push(ChannelStatus::default());
        }
        play_data.channel_status.truncate(num_channels);

        for (i, channel) in self.channels.iter().enumerate() {
            let status = &mut play_data.channel_status[i];

            let mut instrument_idx = 0;
            let mut sample_idx = 0;
            let mut volume = 0;
            let mut envelope_vol = 0;
            let mut global_vol = 0;
            let mut fadeout_vol = 0;
            let mut sample_position = 0.0;
            let mut note_str = "---".to_string();
            let mut period = 0;
            let mut final_panning = 128;
            let mut instrument_name = "".to_string();
            let mut voice_frequency = 0.0;
            let mut voice_on = false;

            if let Some(v_idx) = channel.voice_idx {
                if self.voices[v_idx].channel_idx == i {
                    let v = &self.voices[v_idx];
                    voice_on = v.on;
                    instrument_idx = v.instrument;
                    sample_idx = v.sample;
                    volume = v.volume.volume;
                    envelope_vol = v.volume.envelope_vol;
                    global_vol = v.volume.global_vol as i32;
                    fadeout_vol = v.volume.fadeout_vol;
                    sample_position = v.sample_position;
                    note_str = channel.note.to_string();
                    period = channel.note.period;
                    final_panning = v.panning.final_panning;
                    voice_frequency = v.frequency;

                    if v.instrument < self.song_data.instruments.len() {
                        instrument_name = self.song_data.instruments[v.instrument].name.clone();

                        let inst = &self.song_data.instruments[v.instrument];
                        if v.sample < inst.samples.len() {
                            let sample = &inst.samples[v.sample];
                            if sample.is_ping_pong && sample_position >= sample.original_loop_end as f32 {
                                let over = sample_position - sample.original_loop_end as f32;
                                sample_position = (sample.original_loop_end as f32 - 1.0) - over;
                            }
                        }
                    }
                }
            }

            // High-fidelity scope normalization
            let mut peak = 0.01f32;
            for &s in channel.last_samples.iter() {
                peak = peak.max(s.abs());
            }
            let gain = if peak > 0.0001 { 0.5 / peak } else { 1.0 };

            if status.oscilloscope.len() != 512 {
                status.oscilloscope = vec![0.0; 512];
            }
            if channel.on && voice_on {
                for j in 0..512 {
                    let idx = (channel.last_samples_pos + j) % 512;
                    status.oscilloscope[j] = channel.last_samples[idx] * gain;
                }
            } else {
                for j in 0..512 {
                    status.oscilloscope[j] = 0.0;
                }
            }

            status.volume             = volume as f32;
            status.envelope_volume    = envelope_vol as f32;
            status.global_volume      = global_vol as f32;
            status.fadeout_volume     = fadeout_vol as f32;
            status.on                 = channel.on && voice_on;
            status.force_off          = channel.force_off;
            status.frequency          = voice_frequency;
            let is_linear = self.song_data.frequency_type == FrequencyType::LINEAR;
            let base_frequency = channel.note.base_frequency(is_linear, self.frequency_tables);
            if base_frequency > 0.0 && voice_frequency > 0.0 {
                status.pitch_shift = (voice_frequency / base_frequency).log2() * 12.0;
            } else {
                status.pitch_shift = 0.0;
            }

            status.instrument         = instrument_idx;
            status.sample             = sample_idx;
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

        let history_len = 8192;
        let start_offset = (self.master_samples_pos as isize - self.visual_latency).rem_euclid(history_len as isize) as usize;

        for i in 0..512 {
            let idx = (start_offset + i) % history_len;
            play_data.master_oscilloscope[i] = self.master_samples[idx];
        }

        // Master FFT
        let fft = self.cached_fft.as_ref().unwrap();
        let mut fft_input_buffer = vec![0.0f32; 2048];
        let base_offset = (start_offset as isize - 512).rem_euclid(history_len as isize) as usize;
        for i in 0..2048 {
            let idx = (base_offset + i) % history_len;
            fft_input_buffer[i] = self.master_samples[idx] * self.hann_window[i];
        }

        let mut fft_buffer: Vec<Complex<f32>> = fft_input_buffer.iter()
            .map(|&s| Complex::new(s, 0.0))
            .collect();
        fft.process(&mut fft_buffer);

        if play_data.master_spectrum.len() != 128 {
            play_data.master_spectrum = vec![0.0; 128];
        }

        let decay = if cfg!(target_arch = "wasm32") { 0.88f32 } else { 0.92f32 };
        let min_f = 20.0f32;
        let max_f = 20000.0f32;
        let log_min_f = min_f.ln();
        let log_max_f = max_f.ln();

        for j in 0..128 {
            let f_start = (log_min_f + (j as f32 / 128.0) * (log_max_f - log_min_f)).exp();
            let f_end   = (log_min_f + ((j as f32 + 1.0) / 128.0) * (log_max_f - log_min_f)).exp();

            let b_start = f_start * 2048.0 / self.rate;
            let b_end   = f_end   * 2048.0 / self.rate;
            let b_center = (b_start + b_end) * 0.5;

            let mut magnitude = 0.0f32;

            if b_end - b_start <= 1.0 {
                let i = b_center.floor() as usize;
                let i = i.clamp(1, 1022);
                let t = b_center - i as f32;
                let m0 = fft_buffer[i].norm() / 20.0;
                let m1 = fft_buffer[i + 1].norm() / 20.0;
                magnitude = m0 * (1.0 - t.clamp(0.0, 1.0)) + m1 * t.clamp(0.0, 1.0);
            } else {
                let i_s = b_start.floor() as usize;
                let i_e = b_end.ceil() as usize;
                for i in i_s..i_e {
                    let i_clamped = i.clamp(1, 1023);
                    let m = fft_buffer[i_clamped].norm() / 20.0;
                    if m > magnitude { magnitude = m; }
                }
            }

            self.spectral_peaks[j] = magnitude.max(self.spectral_peaks[j] * decay);
            play_data.master_spectrum[j] = self.spectral_peaks[j];
        }

        play_data.display_fps = self.fps;
    }
}
