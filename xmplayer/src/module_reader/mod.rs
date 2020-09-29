use std::fmt;
use simple_error::SimpleResult;
use crate::instrument::{Instrument};
use crate::module_reader::module::module::read_mod;
use crate::module_reader::s3m::s3m::read_s3m;
use crate::module_reader::xm::xm::read_xm;
use crate::pattern::Pattern;
use std::path::Iter;

mod xm;
mod module;
mod s3m;

#[derive(Debug)]
enum SongType {
    XM,
    MOD,
    S3M
}

#[derive(Debug)]
enum FrequencyType {
    AMIGA,
    LINEAR
}
pub(crate) fn is_note_valid(note: u8) -> bool {
    note > 0 && note < 97
}

#[derive(Clone)]
pub(crate) struct Row {
    pub(crate) channels: Vec<Pattern>
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


#[derive(Debug)]
pub(crate) struct Patterns {
    pub(crate) rows: Vec<Row>
}

impl Patterns {
    fn new(row_count: usize, channel_count: usize) -> Self {
        Self {
            rows: vec![Row::new(channel_count); row_count],
        }
    }

}

#[derive(Debug)]
pub struct SongData {
                    id:                 String,
                    name:               String,
                    song_type:          SongType,
                    tracker_name:       String,
    pub(crate)      song_length:        u16,
    pub(crate)      restart_position:   u16,
                    channel_count:      u16,
    pub(crate)      patterns:           Vec<Patterns>,
                    instrument_count:   u16,
                    frequency_type:     FrequencyType,
    pub(crate)      tempo:              u16,
    pub(crate)      bpm:                u16,
    pub(crate)      pattern_order:      Vec<u8>,
    pub(crate)      instruments:        Vec<Instrument>,
    pub(crate)      use_amiga:          bool,
}


pub fn read_module(path: &str) -> SimpleResult<SongData> {

    match read_xm(path) {
        Ok(module) => {return Ok(module)},
        Err(_) => {},
    }

    match read_mod(path) {
        Ok(module) => {return Ok(module)},
        Err(_) => {},
    }

    read_s3m(path)
}

pub fn print_module(_data: &SongData, patterns: impl Iterator<Item = String>) {
    for pattern in patterns {
        dbg!(&_data.patterns[_data.pattern_order[pattern.parse::<usize>().unwrap()] as usize]);
    }
    // println!("=====================================================================");
    // dbg!(&data.patterns[data.pattern_order[1] as usize]);
}


