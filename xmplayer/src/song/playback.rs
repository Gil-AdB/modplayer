// Playback control: tick / row / pattern advancement, seeking, fast-forward,
// and the duration-precompute pass. Mixing happens in song::output;
// effect dispatch happens in song::backend.

use std::cmp::min;
use std::sync::mpsc::Receiver;

use crate::channel_state::{ChannelState, Voice};
use crate::module_reader::SongType;
use crate::song::backend::{
    apply_extended, apply_flow_control_effect, EffectCtx, ExtendedCmdKind,
    SongPlaybackResources, IT_S_TABLE, MOD_E_TABLE, S3M_S_TABLE, XM_E_TABLE,
};
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

        // Reset all channels to blank slate, but preserve initial volumes/
        // panning AND the per-format `frequency_scale` (set in Song::new
        // from the VoiceMixFormula table). MOD's Protracker-clock pitch
        // compensation lives there and must survive song-reset.
        let mix = crate::song::backend::voice_mix(self.song_data.song_type);
        for (i, ch) in self.channels.iter_mut().enumerate() {
            *ch = ChannelState::new();
            ch.frequency_scale = mix.freq_scale;
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
        // Suspend pause / loop_pattern for the duration of the seek. Either
        // would deadlock the loop below: pause makes get_next_tick early-return
        // without advancing time, and loop_pattern makes next_pattern() a no-op
        // so position-based conditions never become true. Both states are
        // restored before we return so the user's UI toggle is preserved.
        let current_pause = self.pause;
        let current_loop_pattern = self.loop_pattern;
        self.display = false;
        self.is_fast_forwarding = true;
        self.pause = false;
        self.loop_pattern = false;

        while !condition(self) {
            if let CallbackState::Complete = self.get_next_tick(&mut adapter, &mut rx) {
                // Reached end of track or loop point
                break;
            }
        }

        self.display = current_display;
        self.is_fast_forwarding = current_ff;
        self.pause = current_pause;
        self.loop_pattern = current_loop_pattern;
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

    /// Advance state by exactly one row. Used as the paused-mode response to
    /// PlaybackCmd::Next so the user can frame-step through a song. Walks
    /// process_tick + next_tick directly (no buffer fill, no pause gate),
    /// which is precise to a single tick — fast_forward_until is buffer-
    /// granular and would over-run by tens of ticks.
    pub fn step_forward_row(&mut self) {
        if self.song_position >= self.song_data.song_length as usize { return; }
        let saved_loop = self.loop_pattern;
        self.loop_pattern = false;
        let start_row = self.row;
        let start_pos = self.song_position;
        // Sanity bound: a row can't legitimately span thousands of ticks.
        // Stops a degenerate effect sequence from hanging the UI thread.
        let mut guard = 0u32;
        while self.row == start_row && self.song_position == start_pos {
            self.process_tick();
            if !self.next_tick() { break; }
            guard += 1;
            if guard > 4096 { break; }
        }
        self.loop_pattern = saved_loop;
    }

    /// Step state back by exactly one row. Resets and walks forward to the
    /// previous (position, row), tick-precise. Cheap because we skip buffer
    /// fill — same per-tick cost as compute_total_duration.
    pub fn step_backward_row(&mut self) {
        let (target_pos, target_row) = if self.row > 0 {
            (self.song_position, self.row - 1)
        } else if self.song_position > 0 {
            let prev_pos = self.song_position - 1;
            let pat_idx = self.song_data.pattern_order[prev_pos] as usize;
            let last_row = self.song_data.patterns[pat_idx].rows.len().saturating_sub(1);
            (prev_pos, last_row)
        } else {
            return;
        };
        let saved_loop = self.loop_pattern;
        self.reset();
        self.loop_pattern = false;
        loop {
            if self.song_position == target_pos && self.row == target_row { break; }
            // Bail if pattern_break / jump took us past the target.
            if self.song_position > target_pos { break; }
            if self.song_position == target_pos && self.row > target_row { break; }
            self.process_tick();
            if !self.next_tick() { break; }
        }
        self.loop_pattern = saved_loop;
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
                // Break-target row past pattern length wraps to row 0.
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
            // Paused-mode "play one row" UX: decrement on each row advance
            // and re-pause when the budget runs out. Placed here (after the
            // pattern-delay early-return above) so a delayed row counts as
            // one row of playback, not several.
            if self.play_rows_remaining > 0 {
                self.play_rows_remaining -= 1;
                if self.play_rows_remaining == 0 {
                    self.pause = true;
                }
            }
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
        // Skip all expensive voice/effect processing and only handle the
        // flow-control effects that can change the song's duration.
        if self.is_calculating_duration {
            let pat_idx = self.song_data.pattern_order[self.song_position] as usize;
            let song_type = self.song_data.song_type;
            let first_tick = self.tick == 0;
            let row = self.row;
            let rate = self.rate;

            // Snapshot the row's effect params first; we then mutate
            // pattern_change / speed / bpm / channels[i] without keeping a
            // borrow on self.song_data.
            let row_effects: Vec<(u8, u8)> = self.song_data.patterns[pat_idx].rows[row]
                .channels.iter()
                .map(|c| (c.effect, c.effect_param))
                .collect();

            for (i, &(effect, effect_param)) in row_effects.iter().enumerate() {
                let pat = crate::pattern::Pattern { note: 0, instrument: 0, volume: 255, effect, effect_param };

                if apply_flow_control_effect(
                    &pat, song_type, first_tick,
                    &mut self.pattern_change, &mut self.speed, &mut self.bpm, rate,
                ) {
                    continue;
                }

                // Pattern Loop and Pattern Delay live in the E/S extended
                // command. Route them through apply_extended (with voice =
                // None) so the canonical implementation in backend.rs is
                // the single source of truth.
                let is_extended = match song_type {
                    SongType::XM | SongType::MOD => effect == 0x0E,
                    SongType::S3M | SongType::IT => effect == 0x13,
                    _ => false,
                };
                if is_extended {
                    let x = pat.get_x();
                    let kind = match song_type {
                        SongType::XM  => XM_E_TABLE[x as usize],
                        SongType::MOD => MOD_E_TABLE[x as usize],
                        SongType::S3M => S3M_S_TABLE[x as usize],
                        SongType::IT  => IT_S_TABLE[x as usize],
                        _ => ExtendedCmdKind::None,
                    };
                    // Only flow-affecting subcommands (PatternLoop /
                    // PatternDelay) need to be applied here; the others
                    // touch voices we're not running. apply_extended is
                    // safe to call with voice = None for the flow ones.
                    if matches!(kind, ExtendedCmdKind::PatternLoop | ExtendedCmdKind::PatternDelay) {
                        let mut ctx = EffectCtx {
                            pattern_change: &mut self.pattern_change,
                            global_volume: &mut self.global_volume,
                            instruments: &self.song_data.instruments,
                            frequency_tables: self.frequency_tables,
                            tick: self.tick,
                            row,
                            first_tick,
                            first_row_tick: first_tick,
                            note_delay_first_tick: first_tick,
                            song_type,
                            rate,
                            old_effects: self.old_effects,
                            compatible_g: self.compatible_g,
                            use_amiga: self.song_data.use_amiga,
                            fast_volume_slides: self.song_data.fast_volume_slides,
                        };
                        apply_extended(kind, &mut self.channels[i], None, &mut ctx, pat.get_y());
                    }
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
