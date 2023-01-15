use std::{fmt, fs};
use simple_error::{SimpleResult, SimpleError};
use crate::instrument::{Instrument, Sample};
use crate::module_reader::module::module::read_mod;
use crate::module_reader::s3m::s3m::read_s3m;
use crate::module_reader::xm::xm::read_xm;
use crate::pattern::Pattern;
use crate::channel_state::channel_state::clamp;
use crate::module_reader::stm::stm::read_stm;
use crate::module_reader::it::it::read_it;
use crate::channel_state::ChannelState;
use crate::song_state::SongHandle;
use std::io::{Cursor};

mod xm;
mod module;
mod s3m;
mod stm;
mod it;

#[derive(Debug, Copy, Clone)]
enum SongType {
    XM,
    MOD,
    S3M,
    STM
}

#[derive(Debug, Copy, Clone)]
enum FrequencyType {
    AMIGA,
    LINEAR
}
pub(crate) fn is_note_valid(note: u8) -> bool {
    note > 0 && note < 97
}

#[derive(Clone)]
pub struct Row {
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


#[derive(Debug, Clone)]
pub struct Patterns {
    pub rows: Vec<Row>
}

impl Patterns {
    fn new(row_count: usize, channel_count: usize) -> Self {
        Self {
            rows: vec![Row::new(channel_count); row_count],
        }
    }

}

#[derive(Debug, Clone)]
pub struct SongData {
                    id:                 String,
   pub(crate)       name:               String,
                    song_type:          SongType,
                    tracker_name:       String,
    pub(crate)      song_length:        u16,
    pub(crate)      restart_position:   u16,
    pub(crate)      channel_count:      u16,
    pub(crate)      patterns:           Vec<Patterns>,
                    instrument_count:   u16,
                    frequency_type:     FrequencyType,
    pub(crate)      tempo:              u16,
    pub(crate)      bpm:                u16,
    pub(crate)      pattern_order:      Vec<u8>,
    pub(crate)      instruments:        Vec<Instrument>,
    pub(crate)      use_amiga:          bool,
}

impl SongData {
    pub(crate) fn get_sample<>(&self, channel: &ChannelState) -> &Sample {
        &self.get_instrument(channel).samples[channel.voice.sample]
    }

    pub(crate) fn get_instrument(&self, channel: &ChannelState) -> &Instrument {
        &self.instruments[channel.voice.instrument]
    }
}


pub fn read_module(path: &str) -> SimpleResult<SongData> {

    // let f = match File::open(path) {
    //     Ok(f) => {f}
    //     Err(_) => {return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "failed to open the file")));}
    // };

    let data = match fs::read(path) {
        Ok(d) => {d}
        Err(e) => {return Err(SimpleError::from(e));}
    };

    open_module(data.as_slice())
}

pub fn open_module(data: &[u8]) -> SimpleResult<SongData> {
    let mut buf = Cursor::new(data);

    match read_xm(&mut buf) {
        Ok(module) => {return Ok(module)},
        Err(_) => {},
    }

    match read_mod(&mut buf) {
        Ok(module) => {return Ok(module)},
        Err(_) => {},
    }

    match read_stm(&mut buf) {
        Ok(module) => {return Ok(module)},
        Err(_) => {},
    }

    match read_s3m(&mut buf) {
        Ok(module) => {return Ok(module)},
        Err(_) => {},
    }

    read_it(&mut buf)
}


pub fn print_module(handle: &SongHandle, patterns: impl Iterator<Item = String>) {
    let _data = &handle.get().song_data;

    for pattern in patterns {
        dbg!(&_data.patterns[_data.pattern_order[pattern.parse::<usize>().unwrap()] as usize]);
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
