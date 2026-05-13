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
        // `is_calculating_duration` activates the cheap process_tick fast-
        // path (flow-control effects only, no voice/sample work). The
        // termination path is shared with regular playback — next_tick
        // tags each order's row 0 in `visited_rows` and returns false on
        // the second visit, so fast_forward_until exits via
        // `CallbackState::Complete` at the same song time the real
        // playback would. A 20-minute hard cap stays in place as a
        // safety net for songs that don't terminate naturally.
        self.is_calculating_duration = true;

        let max_samples = (20.0 * 60.0 * self.original_rate) as u64;
        self.fast_forward_until(|s| s.total_samples > max_samples);

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
        // Clear the loop-detection bitset; otherwise a PlaybackCmd::Restart
        // would immediately terminate since every row was already visited.
        for v in self.visited_rows.iter_mut() { *v = 0; }
        // Mark the initial (song_position=0, row=0) entry as visit #1.
        // We only increment visited_rows inside the row-transition path
        // (`if self.tick >= self.speed` in next_tick), so the very first
        // row of the song would otherwise never be counted and the
        // song-level loop-back via Bxx would take an extra iteration
        // to terminate.
        if !self.visited_rows.is_empty() {
            self.visited_rows[0] = 1;
        }

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
        // Stuck-row fallback. Visited-row detection in next_tick only
        // fires on a row advance — if the song freezes on a single row
        // (zero-speed Fxx, infinite pattern delay, etc.) `total_samples`
        // keeps growing but no new row is entered, so the row check never
        // sees a duplicate. Cap actual playback at 1.5× the predicted
        // single-loop duration with a 30s floor for songs whose duration
        // hasn't been computed yet. Bypass while seeking / duration-calc.
        if !self.is_fast_forwarding && !self.is_calculating_duration {
            let predicted_samples = (self.total_duration_ms / 1000.0 * self.rate) as u64;
            let cap = (predicted_samples.saturating_mul(3) / 2)
                .max((30.0 * self.rate) as u64);
            if self.total_samples > cap { return CallbackState::Complete; }
        }
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

            let took_pattern_loop = self.pattern_change.is_loop;
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

            // Loop detection. We just entered a new row at tick 0. If this
            // (song_position, row) was already played in this run, the song
            // has looped — terminate so the playlist advances. The
            // bypassed paths (FF / duration-calc) have their own visited
            // tracking and must run the full loop.
            //
            // When an in-pattern SBxy / E6x pattern loop fires, the rows
            // about to be replayed (loop_row..current_row) need their
            // visited bits cleared so the loop body can run again — and
            // we skip the check on the transition itself. 1_channel_moog.it
            // Song-level loop detection. Tag only the *entry* row of each
            // order (row 0). SBxy / E6x pattern loops within a pattern
            // don't touch row 0 so the loop body can repeat freely; Bxx
            // pattern jumps and natural next_pattern() advance always land
            // at row 0 of the new order, so every song-level pass
            // increments here. Terminate on the first revisit — the
            // initial pos=0 row=0 was tagged in reset() so a Bxx back to
            // pos=0 trips this on its first replay (matches libopenmpt's
            // ~65 s for 1_channel_moog.it).
            //
            // This runs in *all* paths (regular playback, duration calc,
            // user seeks). compute_total_duration and seek operations can
            // ride the same `CallbackState::Complete` exit; no separate
            // termination machinery needed. (User-driven seeks that want
            // to bypass this — e.g. seek-to-start after song-end — should
            // call reset() first to clear visited_rows.)
            if self.row == 0 {
                let idx = self.song_position * 512;
                if idx < self.visited_rows.len() {
                    if self.visited_rows[idx] >= 1 { return false; }
                    self.visited_rows[idx] = self.visited_rows[idx].saturating_add(1);
                }
            }
            let _ = took_pattern_loop;
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
