// PlaybackCmd dispatch (handle_commands).

use std::num::Wrapping;
use std::sync::mpsc::Receiver;

use crate::tables::{AMIGA_TABLES, LINEAR_TABLES};
use crate::song::{FilterType, PlaybackCmd, Song, UserData};

impl Song {
    pub fn handle_commands(&mut self, rx: & Receiver<PlaybackCmd>) -> bool {
        loop {
            if let Ok(cmd) = rx.try_recv() {
                match cmd {
                    PlaybackCmd::Quit => {
                        return false;
                    }
                    PlaybackCmd::Next => {
                        // Paused: resume for exactly one row of audio, then
                        // auto-pause (handled in next_tick via
                        // play_rows_remaining). Playing: full pattern jump.
                        if self.pause {
                            self.pause = false;
                            self.play_rows_remaining = 1;
                        } else {
                            self.seek_forward_pattern();
                        }
                    }
                    PlaybackCmd::Prev => {
                        // Paused: silent rewind one row (no audio — playing
                        // backward isn't a thing). Use Next afterwards to
                        // re-hear the row. Playing: full pattern jump back.
                        if self.pause { self.step_backward_row(); }
                        else          { self.seek_backward_pattern(); }
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
                    PlaybackCmd::AmigaTable => { self.frequency_tables = AMIGA_TABLES.as_ref(); }
                    PlaybackCmd::LinearTable => { self.frequency_tables = LINEAR_TABLES.as_ref(); }
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
                        self.theme_id = (self.theme_id + 1) % 5;

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
}
