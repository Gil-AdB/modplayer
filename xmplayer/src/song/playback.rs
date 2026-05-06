// Playback control: tick / row / pattern advancement, seeking, fast-forward,
// and the duration-precompute pass. Mixing happens in song::output;
// effect dispatch happens in song::backend.

use std::cmp::min;
use std::sync::mpsc::Receiver;

use crate::channel_state::{ChannelState, Voice};
use crate::song::backend::SongPlaybackResources;
use crate::song::{
    BPM, BufferAdapter, BufferState, CallbackState, GlobalVolume, InterleavedBufferAdaptar,
    PatternChange, PlaybackCmd, Song, TickState,
};

impl Song {
    pub(super) fn compute_total_duration(&mut self) -> f32 {
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
        self.speed = self.song_data.tempo as u32;
        self.bpm = BPM::new(self.song_data.bpm as u32, self.rate);
        // Mirror Song::new: seed global_volume from the loaded song_data so a
        // reset() during compute_total_duration / PlaybackCmd::Restart doesn't
        // discard the module's authored global volume.
        self.global_volume = GlobalVolume::new();
        self.global_volume.volume = self.song_data.global_volume as u32;
        self.global_volume.song_type = Some(self.song_data.song_type);
        self.pattern_change = PatternChange::new();
        self.total_samples = 0;
        self.last_fps_sample = 0;
        self.last_display_update_sample = 0;

        self.tick_state = TickState {
            state: BufferState::Start,
            current_buf_position: 0,
            current_tick_position: 0,
        };

        // Reset all channels to blank slate, but preserve initial volumes/panning
        for (i, ch) in self.channels.iter_mut().enumerate() {
            *ch = ChannelState::new();
            if i < 64 && i < self.song_data.initial_channel_volume.len() && i < self.song_data.initial_channel_panning.len() {
                let p = self.song_data.initial_channel_panning[i];
                if p == 100 {
                    ch.panning.panning = 128;
                } else {
                    ch.panning.panning = p;
                }
                ch.volume.set_volume(self.song_data.initial_channel_volume[i] as i32);
                ch.channel_volume = self.song_data.initial_channel_volume[i];
            }
        }

        // Reset all voices
        for (i, voice) in self.voices.iter_mut().enumerate() {
            let ch_count = (self.song_data.channel_count as usize).max(1);
            *voice = Voice::new();
            voice.channel_idx = i % ch_count;
        }
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
                    self.tick_state.state = BufferState::FillBuffer;
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
                    self.tick_state.state = BufferState::Start;
                }
            }
        }
    }

    pub fn next_tick(&mut self) -> bool {
        if self.song_position >= self.song_data.song_length as usize {
            return false;
        }

        self.tick += 1;
        if self.tick >= self.speed {
            // Handle Pattern Delay (EEx, S3M/IT equivalents)
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
                // Per ProTracker/OpenMPT: a break-target row past the destination
                // pattern's length wraps to row 0 (e.g. spacedeb.mod uses Dxy=80
                // into a 64-row pattern).
                if self.song_position < self.song_data.pattern_order.len() {
                    let pat_idx = self.song_data.pattern_order[self.song_position] as usize;
                    if pat_idx < self.song_data.patterns.len()
                        && self.row >= self.song_data.patterns[pat_idx].rows.len() {
                        self.row = 0;
                    }
                }
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
        if self.song_position as usize >= self.song_data.pattern_order.len() {
            return;
        }

        // Hyper-optimization for duration calculation:
        // Skip all expensive effect processing and only handle flow control.
        if self.is_calculating_duration {
            let patterns = &self.song_data.patterns[self.song_data.pattern_order[self.song_position] as usize];
            let row = &patterns.rows[self.row];
            let first_tick = self.tick == 0;

            for pattern in &row.channels {
                match pattern.effect {
                    0xB | 2 => { self.pattern_change.set_jump(first_tick, pattern.effect_param); }
                    0xD | 3 => { self.pattern_change.set_break(self.song_data.song_type, first_tick, pattern.effect_param); }
                    0xF => {
                        if first_tick && pattern.effect_param > 0 {
                            if pattern.effect_param <= 0x1f { self.speed = pattern.effect_param as u32; }
                            else { self.bpm.update(pattern.effect_param as u32, self.rate); }
                        }
                    }
                    1 => { // S3M Speed
                        if first_tick && pattern.effect_param > 0 { self.speed = pattern.effect_param as u32; }
                    }
                    20 => { // S3M BPM
                        if first_tick && pattern.effect_param > 0 { self.bpm.update(pattern.effect_param as u32, self.rate); }
                    }
                    0xE | 0x13 => {
                        let x = pattern.get_x();
                        let y = pattern.get_y();
                        match x {
                            0x6 | 0xB => {
                                // Pattern Loop (E6x / S3B)
                                if first_tick && y != 0 {
                                    self.pattern_change.is_loop = true;
                                }
                            }
                            0xE => {
                                // Pattern Delay (EEx / SxE)
                                if first_tick {
                                    self.pattern_change.pattern_delay = y;
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            return;
        }

        let first_row_tick = self.tick == 0;
        let mut r = SongPlaybackResources {
            song_position: &mut self.song_position,
            row: &mut self.row,
            tick: &mut self.tick,
            speed: &mut self.speed,
            global_volume: &mut self.global_volume,
            song_data: &self.song_data,
            channels: &mut self.channels,
            voices: &mut self.voices,
            pattern_change: &mut self.pattern_change,
            bpm: &mut self.bpm,
            frequency_tables: self.frequency_tables,
            rate: self.rate,
            first_row_tick,
            old_effects: self.old_effects,
            compatible_g: self.compatible_g,
        };

        if let Some(backend) = &mut self.backend {
            backend.process_tick(&mut r);
        }
    }
}
