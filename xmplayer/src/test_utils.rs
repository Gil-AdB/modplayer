use crate::module_reader::{SongData, SongType, FrequencyType, Patterns};
use crate::pattern::Pattern;
use crate::instrument::{Instrument, Sample};
use crate::song::{Song, PlayData};
use shared_sync_primitives::{TripleBuffer};

pub struct MockSongBuilder {
    pub song_type: SongType,
    pub channels: usize,
    pub patterns: Vec<Patterns>,
    pub order: Vec<u8>,
    pub instruments: Vec<Instrument>,
    pub global_volume: u8,
    pub master_volume: u8,
    pub use_amiga: bool,
}

impl MockSongBuilder {
    pub fn new(song_type: SongType, channels: usize) -> Self {
        Self {
            song_type,
            channels,
            patterns: vec![],
            order: vec![],
            instruments: {
                let mut inst = Instrument::new();
                let mut sample = Sample::new();
                sample.data = vec![0.0; 100];
                sample.length = 100;
                sample.volume = 64;
                inst.samples.push(sample);
                for i in 0..120 { inst.sample_indexes[i] = (i as u8, 1); }
                vec![Instrument::new(), inst]
            },
            global_volume: 64,
            master_volume: 48 | 128, // Default S3M style: Vol 48, Stereo
            use_amiga: false,
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

    pub fn add_pattern_row(&mut self, pat_idx: usize, row_idx: usize, note: u8, inst: u8, vol: u8, eff: u8, eff_param: u8) -> &mut Self {
        if self.patterns.len() <= pat_idx {
            self.add_empty_pattern(64);
        }
        self.patterns[pat_idx].rows[row_idx].channels[0] = Pattern {
            note,
            instrument: inst,
            volume: vol,
            effect: eff,
            effect_param: eff_param,
        };
        self
    }

    pub fn use_amiga_freq(&mut self, on: bool) -> &mut Self {
        self.use_amiga = on;
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

    pub fn set_nna(&mut self, inst_idx: usize, nna: u8) -> &mut Self {
        if inst_idx < self.instruments.len() {
            self.instruments[inst_idx].nna = nna;
        }
        self
    }

    pub fn build(&self) -> SongData {
        SongData {
            id: "MOCK".to_string(),
            name: "Mock Song".to_string(),
            file_name: String::new(),
            song_type: self.song_type,
            tracker_name: "MockTracker".to_string(),
            song_length: self.order.len() as u16,
            restart_position: 0,
            channel_count: self.channels as u16,
            patterns: self.patterns.clone(),
            instrument_count: self.instruments.len() as u16,
            frequency_type: if self.use_amiga { FrequencyType::AMIGA } else { FrequencyType::LINEAR },
            tempo: 6,
            bpm: 125,
            pattern_order: self.order.clone(),
            instruments: self.instruments.clone(),
            use_amiga: self.use_amiga,
            song_message: "".to_string(),
            initial_channel_volume: [64; 64],
            initial_channel_panning: [128; 64],
            global_volume: self.global_volume,
            master_volume: self.master_volume,
            mixing_volume: 128,
            old_effects: false,
            compatible_g: false,
            fast_volume_slides: false,
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
        let song = Song::new(&song_data, writer, 48000.0);
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

    pub fn get_channel_period(&self, channel: usize) -> u16 {
        self.song.channels[channel].note.period
    }

    /// Period after period_shift (arpeggio / vibrato / etc.) is applied. The
    /// raw `note.period` is the trigger-time pitch; effects that nudge pitch
    /// per-tick land in `period_shift`, which `update_frequency_voice` adds
    /// in to drive the voice. Tests that care about the actual playing pitch
    /// should use this.
    pub fn get_channel_effective_period(&self, channel: usize) -> i32 {
        let c = &self.song.channels[channel];
        c.note.period as i32 + c.period_shift as i32
    }

    pub fn get_channel_vibrato(&self, channel: usize) -> (u8, u8) {
        use crate::channel_state::EffectMemorySlot;
        let c = &self.song.channels[channel];
        (c.mem(EffectMemorySlot::VibratoSpeed), c.mem(EffectMemorySlot::VibratoDepth))
    }

    pub fn assert_voice_on(&self, voice_idx: usize, on: bool) {
        assert_eq!(self.song.voices[voice_idx].on, on, "Voice {} on state mismatch", voice_idx);
    }

    pub fn get_voice_du(&self, voice_idx: usize) -> f32 {
        self.song.voices[voice_idx].du
    }

    pub fn assert_voice_du_near(&self, voice_idx: usize, expected: f32, epsilon: f32) {
        let actual = self.get_voice_du(voice_idx);
        assert!((actual - expected).abs() < epsilon, "Voice {} dU mismatch: expected {}, got {}", voice_idx, expected, actual);
    }

    pub fn assert_voice_volume_near(&self, voice_idx: usize, expected: f32, epsilon: f32) {
        let actual = self.song.voices[voice_idx].volume.output_volume;
        assert!((actual - expected).abs() < epsilon, "Voice {} volume mismatch: expected {}, got {}", voice_idx, expected, actual);
    }

    pub fn step_to_row(&mut self, row: usize) {
        while self.song.row < row {
            self.step_row();
        }
    }

    pub fn assert_pitch_near(&self, voice_idx: usize, expected_hz: f32, tolerance: f32) {
        let actual = self.song.voices[voice_idx].frequency;
        assert!((actual - expected_hz).abs() < tolerance, "Voice {} pitch mismatch: expected {} Hz, got {} Hz", voice_idx, expected_hz, actual);
    }

    pub fn run_row(&mut self) {
        let start_row = self.song.row;
        let start_pos = self.song.song_position;
        while self.song.row == start_row && self.song.song_position == start_pos {
            self.tick();
        }
    }

    pub fn run_ticks(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.tick();
        }
    }

    pub fn assert_sample_pos(&self, voice_idx: usize, expected_pos: f32) {
        let actual = self.song.voices[voice_idx].sample_position;
        assert!((actual - expected_pos).abs() < 0.01, "Voice {} sample pos mismatch: expected {}, got {}", voice_idx, expected_pos, actual);
    }

    pub fn assert_voice_playing(&self, voice_idx: usize, is_playing: bool) {
        assert_eq!(self.song.voices[voice_idx].on, is_playing, "Voice {} playing state mismatch: expected {}, got {}", voice_idx, is_playing, self.song.voices[voice_idx].on);
    }
}
