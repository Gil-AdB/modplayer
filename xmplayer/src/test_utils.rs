use crate::module_reader::{SongData, SongType, FrequencyType, Patterns};
use crate::pattern::Pattern;
use crate::instrument::{Instrument, Sample};
use crate::song::{Song, PlayData, GlobalVolume};
use shared_sync_primitives::{TripleBuffer};

pub struct MockSongBuilder {
    pub song_type: SongType,
    pub channels: usize,
    pub patterns: Vec<Patterns>,
    pub order: Vec<u8>,
    pub instruments: Vec<Instrument>,
}

impl MockSongBuilder {
    pub fn new(song_type: SongType, channels: usize) -> Self {
        Self {
            song_type,
            channels,
            patterns: vec![],
            order: vec![],
            instruments: vec![Instrument::new()], // Null instrument
        }
    }

    pub fn add_empty_pattern(&mut self, rows: usize) -> &mut Self {
        self.patterns.push(Patterns::new(rows, self.channels));
        self.order.push((self.patterns.len() - 1) as u8);
        self
    }

    pub fn set_pattern_row(&mut self, pat_idx: usize, row_idx: usize, ch_idx: usize, pattern: Pattern) -> &mut Self {
        self.patterns[pat_idx].rows[row_idx].channels[ch_idx] = pattern;
        self
    }

    pub fn set_order(&mut self, order: Vec<u8>) -> &mut Self {
        self.order = order;
        self
    }

    pub fn add_instrument(&mut self, name: &str, sample_data: Vec<f32>) -> &mut Self {
        let mut instrument = Instrument::new();
        instrument.name = name.to_string();
        let mut sample = Sample::new();
        sample.name = name.to_string();
        sample.length = sample_data.len() as u32;
        sample.data = sample_data;
        instrument.samples.push(sample);
        // Map all notes to first sample
        for i in 0..120 {
            instrument.sample_indexes[i] = (i as u8, 1);
        }
        self.instruments.push(instrument);
        self
    }

    pub fn build(&self) -> SongData {
        SongData {
            id: "MOCK".to_string(),
            name: "Mock Song".to_string(),
            song_type: self.song_type,
            tracker_name: "MockTracker".to_string(),
            song_length: self.order.len() as u16,
            restart_position: 0,
            channel_count: self.channels as u16,
            patterns: self.patterns.clone(),
            instrument_count: self.instruments.len() as u16,
            frequency_type: FrequencyType::LINEAR,
            tempo: 6,
            bpm: 125,
            pattern_order: self.order.clone(),
            instruments: self.instruments.clone(),
            use_amiga: false,
            song_message: "".to_string(),
            initial_channel_volume: [64; 64],
            initial_channel_panning: [128; 64],
            global_volume: 128,
            master_volume: 128,
            mixing_volume: 128,
            old_effects: false,
            compatible_g: false,
        }
    }

    pub fn get_tester(&self) -> SongTester {
        SongTester::new(self.build())
    }
}

pub struct SongTester {
    pub song: Song,
}

impl SongTester {
    pub fn new(song_data: SongData) -> Self {
        let (_reader, writer) = TripleBuffer::<PlayData>::new().split();
        let song = Song::new(&song_data, writer, 44100.0);
        Self { song }
    }

    pub fn tick(&mut self) {
        // Match Song::get_next_tick loop: process then next
        self.song.process_tick();
        self.song.next_tick();
    }

    pub fn step_row(&mut self) {
        let start_row = self.song.row;
        let start_pos = self.song.song_position;
        
        // Execute first tick
        self.tick();
        
        // Continue until row or position changes
        while self.song.row == start_row && self.song.song_position == start_pos {
            self.tick();
        }
    }

    pub fn get_pos(&self) -> (usize, usize, u32) {
        (self.song.song_position, self.song.row, self.song.tick)
    }

    pub fn get_active_voices(&self) -> usize {
        self.song.voices.iter().filter(|v| v.on).count()
    }

    pub fn get_voices_for_channel(&self, channel: usize) -> Vec<usize> {
        self.song.voices.iter().enumerate()
            .filter(|(_, v)| v.on && v.channel_idx == channel)
            .map(|(i, _)| i)
            .collect()
    }
}
