use std::{fmt, fs};
use serde::Serialize;
use crate::{SimpleResult};
use crate::instrument::{Instrument, Sample};
use crate::module_reader::module::read_mod;
use crate::module_reader::s3m::read_s3m;
use crate::module_reader::xm::read_xm;
use crate::pattern::Pattern;
use crate::channel_state::channel_state::clamp;
use crate::module_reader::stm::read_stm;
use crate::module_reader::it::read_it;
use crate::channel_state::Voice;
use crate::song_state::SongHandle;
use std::io::{Cursor, Seek, SeekFrom};

mod xm;
mod module;
mod s3m;
mod stm;
mod it;
mod it_compression;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize)]
pub enum SongType {
    XM,
    MOD,
    S3M,
    STM,
    IT,
}

#[derive(Debug, Copy, Clone)]
pub enum FrequencyType {
    AMIGA,
    LINEAR
}
pub(crate) fn is_note_valid(note: u8, song_type: SongType) -> bool {
    match song_type {
        SongType::IT => note > 0 && note <= 120,
        SongType::S3M => note > 0 && note <= 97, // 97 is Note-Off in S3M
        _ => note > 0 && note <= 96,
    }
}

#[derive(Clone)]
pub struct Row {
    pub channels: Vec<Pattern>
}

impl Row {
    fn new(channel_count: usize) -> Self {
        Self { channels: vec![Pattern::new(); channel_count] }
    }
}

impl fmt::Debug for Row {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for pattern in &self.channels {
            if first { first = false; } else { write!(f, "|")?; }
            write!(f, "{}", pattern)?;
        }
        Ok(())
    }
}

impl fmt::Display for Row {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for pattern in &self.channels {
            if first { first = false; } else { write!(f, "|")?; }
            write!(f, "{}", pattern)?;
        }
        Ok(())
    }
}


#[derive(Debug, Clone)]
pub struct Patterns {
    pub rows: Vec<Row>
}

impl Patterns {
    pub(crate) fn new(row_count: usize, channel_count: usize) -> Self {
        Self {
            rows: vec![Row::new(channel_count); row_count],
        }
    }

}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SongData {
    pub                     id:                 String,
    pub                     name:               String,
    pub                     song_type:          SongType,
    pub                     tracker_name:       String,
    pub                     song_length:        u16,
    pub                     restart_position:   u16,
    pub                     channel_count:      u16,
    pub                     patterns:           Vec<Patterns>,
    pub                     instrument_count:   u16,
    pub                     frequency_type:     FrequencyType,
    pub                     tempo:              u16,
    pub                     bpm:                u16,
    pub                     pattern_order:      Vec<u8>,
    pub                     instruments:        Vec<Instrument>,
    pub                     use_amiga:          bool,
    pub                     song_message:       String,
    pub                     initial_channel_volume: [u8; 64],
    pub                     initial_channel_panning: [u8; 64],
    pub                     global_volume:           u8,
    pub                     master_volume:           u8,
    pub                     mixing_volume:           u8,
    pub                     old_effects:             bool,
    pub                     compatible_g:            bool,
}

impl Default for SongData {
    fn default() -> Self {
        Self {
            id: "".to_string(),
            name: "".to_string(),
            song_type: SongType::XM,
            tracker_name: "".to_string(),
            song_length: 0,
            restart_position: 0,
            channel_count: 0,
            patterns: vec![],
            instrument_count: 0,
            frequency_type: FrequencyType::LINEAR,
            tempo: 0,
            bpm: 0,
            pattern_order: vec![],
            instruments: vec![],
            use_amiga: false,
            song_message: "".to_string(),
            initial_channel_volume: [64; 64],
            initial_channel_panning: [32; 64],
            global_volume: 128,
            master_volume: 128,
            mixing_volume: 48,
            old_effects: false,
            compatible_g: false,
        }
    }
}

impl Default for SongType {
    fn default() -> Self { SongType::XM }
}

impl Default for FrequencyType {
    fn default() -> Self { FrequencyType::LINEAR }
}

impl SongData {
    pub(crate) fn get_sample(&self, voice: &Voice) -> &Sample {
        &self.instruments[voice.instrument].samples[voice.sample]
    }

    pub(crate) fn get_instrument(&self, voice: &Voice) -> &Instrument {
        &self.instruments[voice.instrument]
    }
}


pub fn read_module(path: &str) -> SimpleResult<SongData> {
    let data = fs::read(path)?;
    open_module(data.as_slice())
}

pub fn open_module(data: &[u8]) -> SimpleResult<SongData> {
    let mut buf = Cursor::new(data);

    let _ = buf.seek(SeekFrom::Start(0));
    match read_xm(&mut buf) {
        Ok(module) => {return Ok(module)},
        Err(_) => {},
    }

    let _ = buf.seek(SeekFrom::Start(0));
    match read_mod(&mut buf) {
        Ok(module) => {return Ok(module)},
        Err(_) => {},
    }

    let _ = buf.seek(SeekFrom::Start(0));
    match read_stm(&mut buf) {
        Ok(module) => {return Ok(module)},
        Err(_) => {},
    }

    let _ = buf.seek(SeekFrom::Start(0));
    match read_s3m(&mut buf) {
        Ok(module) => {return Ok(module)},
        Err(_) => {},
    }

    let _ = buf.seek(SeekFrom::Start(0));
    read_it(&mut buf)
}


pub fn print_module(handle: &SongHandle, patterns: impl Iterator<Item = String>) {
    let _data = &handle.song_data;

    for pattern in patterns {
        match pattern.parse::<usize>() {
            Ok(idx) => {
                if idx < _data.pattern_order.len() {
                    let order_idx = _data.pattern_order[idx] as usize;
                    if order_idx < _data.patterns.len() {
                        dbg!(&_data.patterns[order_idx]);
                    } else {
                        println!("Pattern index {} out of bounds", order_idx);
                    }
                } else {
                    println!("Order index {} out of bounds", idx);
                }
            }
            Err(_) => {
                println!("'{}' is not a valid pattern index. (Additional arguments after the filename are interpreted as patterns to debug-print).", pattern);
            }
        }
    }
    // println!("=====================================================================");
    // dbg!(&data.patterns[data.pattern_order[1] as usize]);
}


fn c2spd_to_finetune_relnote(c2spd: u32) -> (i8, i8) {
    let finetune;
    let mut relative_note;

    let d_freq = (c2spd as f64 / 8363.0).log2() * (12.0 * 128.0);
    let linear_freq = (d_freq + 0.5) as i32; // rounded
    finetune = (((linear_freq + 128) & 255) - 128) as i8;

    relative_note = ((linear_freq - finetune as i32) >> 7) as i8;
    relative_note = clamp(relative_note, -48, 71);

    (finetune, relative_note)
}
