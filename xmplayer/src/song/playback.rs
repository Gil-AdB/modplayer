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
    /// Hash every channel's *active* pattern-loop frame into a single u64.
    /// libopenmpt's `RowVisitor::LoopState` tracks the same thing: a song
    /// has "truly looped" once we revisit a row with identical pattern-
    /// loop counters across every channel. Only `loop_count > 0` is
    /// considered active — once an SB / E6 loop finishes its counter
    /// drops to 0 and the lingering `loop_row` is just a memory slot,
    /// not a state that affects future playback, so we ignore it.
    /// Without this, moog's row-93 SB7 leaves `loop_row=84` on every
    /// iteration and the hash never stabilises, so the song-end check
    /// wouldn't fire on the first B00 back to position 0.
    fn channel_loop_state_hash(channels: &[ChannelState]) -> u64 {
        use std::hash::{Hash, Hasher};
        use std::collections::hash_map::DefaultHasher;
        let mut h = DefaultHasher::new();
        for (i, ch) in channels.iter().enumerate() {
            if ch.loop_count > 0 {
                i.hash(&mut h);
                ch.loop_row.hash(&mut h);
                ch.loop_count.hash(&mut h);
            }
        }
        h.finish()
    }

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
        // Clear loop-detection state; otherwise a PlaybackCmd::Restart
        // would immediately terminate since every order was already
        // visited at its current loop state.
        for set in self.visited_rows.iter_mut() { set.clear(); }
        // Record the initial (song_position=0, row=0) entry. We only
        // tag visits inside the row-transition path (`if self.tick >=
        // self.speed` in next_tick), so the very first row of the song
        // would otherwise never be tagged and a Bxx back to position 0
        // would take an extra iteration to terminate. The state hash at
        // song-start is "all channels' loop_row/loop_count = 0".
        if !self.visited_rows.is_empty() {
            let h = Self::channel_loop_state_hash(&self.channels);
            self.visited_rows[0].insert(h);
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
            // IT-specific: see Song::new for the rationale. reset() runs
            // on every Restart / seek / compute_total_duration, so the
            // C-0 init must be reapplied here too.
            if self.song_data.song_type == crate::module_reader::SongType::IT {
                ch.last_played_note = 1;
            }
            if i < 64 && i < self.song_data.initial_channel_volume.len() && i < self.song_data.initial_channel_panning.len() {
                ch.panning.set_panning(self.song_data.initial_channel_panning[i] as i32);
                ch.surround = self.song_data.initial_channel_surround[i];
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
                // Mark this as a delay repeat so first_row_tick won't
                // fire on this tick-0 — only the LITERAL first tick of
                // the row gets the "first tick" treatment.
                self.pattern_change.in_delay_repeat = true;
                return true;
            }

            let took_pattern_loop = self.pattern_change.is_loop;
            // FT2 (XM) and IT (with kITPatternLoopWithJumps) let pattern
            // break/jump take precedence over pattern loop when both
            // fire on the same row. Other formats prefer the loop and
            // ignore the break/jump. OpenMPT test cases:
            // PatLoop-Break.xm, PatLoop-Weird.xm, PatLoop-Jumps.xm,
            // PatLoop-Break.mod (MOD also follows the FT2 rule per
            // OMT's HandleNextRow).
            let break_or_jump_wins = matches!(
                self.song_data.song_type,
                crate::module_reader::SongType::XM | crate::module_reader::SongType::MOD,
            ) || (
                self.song_data.song_type == crate::module_reader::SongType::IT
                && self.pattern_change.pattern_jump
            );
            let do_loop = self.pattern_change.is_loop
                && !(break_or_jump_wins
                     && (self.pattern_change.pattern_break || self.pattern_change.pattern_jump));
            if self.pattern_change.pattern_break || self.pattern_change.pattern_jump || self.pattern_change.is_loop {
                if do_loop {
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
            // IT/S3M can have sentinels embedded in the order list: 254
            // ("+++") means skip to next order, 255 ("---") means end of
            // song. Bxx can jump past these to a valid pattern further on
            // (orbiter.it: order 29 is 255, order 30 is the AFF/T20 slow
            // loop body). After any song_position change, walk past 254s
            // and terminate on 255.
            loop {
                if self.song_position >= self.song_data.song_length as usize { return false; }
                let v = self.song_data.pattern_order[self.song_position];
                if v == 254 { self.song_position += 1; continue; }
                if v == 255 { return false; }
                if (v as usize) >= self.song_data.patterns.len() { return false; }
                break;
            }
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
            // Song-level loop detection (mirrors libopenmpt's
            // RowVisitor::Visit at order-entry granularity).
            //
            // Tag only the entry row of each order (row 0). SBxy / E6x
            // pattern loops within a pattern don't touch row 0 so the
            // loop body can repeat freely; Bxx pattern jumps and natural
            // next_pattern() advance always land at row 0 of the new
            // order, so every song-level pass increments here.
            //
            // The state key combines order with the hash of every
            // channel's open pattern-loop frame (loop_row, loop_count).
            // Identical hash on a revisit means the song would
            // deterministically repeat from this point — terminate.
            // Different hash means SB/E6 counters are in different
            // mid-flight states from the previous visit, so the song
            // hasn't truly looped yet (e.g. orbiter.it spends many
            // iterations in its B28↔B30 loop while SB counters evolve).
            //
            // Runs in every code path — regular playback,
            // compute_total_duration, user seeks — so all of them exit
            // via the same `CallbackState::Complete`.
            //
            // Skip the check entirely when the user has asked us to
            // loop the current pattern (`/` hotkey → loop_pattern=true).
            // Otherwise the second pass through the pattern wraps row
            // back to 0 at the same order, the visited-set already has
            // this hash from the first pass, and we'd terminate
            // immediately — manifesting as "playback quits at end of
            // pattern when looping is on".
            if self.row == 0 && !self.loop_pattern {
                let order = self.song_position;
                if order < self.visited_rows.len() {
                    let h = Self::channel_loop_state_hash(&self.channels);
                    if self.visited_rows[order].contains(&h) {
                        return false;
                    }
                    self.visited_rows[order].insert(h);
                }
            }
            let _ = took_pattern_loop;
            // Row actually advanced — clear the delay-repeat flag so
            // the new row gets its literal first tick treated as such.
            self.pattern_change.in_delay_repeat = false;
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

        // "First tick of the row" in the OMT/wiki sense: only the
        // literal first tick of the original row, NOT the tick-0 of
        // an EEx delay repeat. Effects gated on this won't re-fire on
        // delay repeats (PatternDelay itself, etc).
        let first_row_tick = self.tick == 0 && !self.pattern_change.in_delay_repeat;
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
