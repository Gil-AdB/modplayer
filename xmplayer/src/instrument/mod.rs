use std::io::{Read, Seek, SeekFrom};
use std::num::Wrapping;

use crate::envelope::Envelope;
use binary_reader_io::BinaryReader;
use crate::{SimpleError, SimpleResult};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopType {
    NoLoop = 0,
    ForwardLoop = 1,
    PingPongLoop = 2,
}

impl LoopType {
    pub(crate) fn from_flags(flags: u8) -> LoopType {
        match flags & 3 {
            0 => LoopType::NoLoop,
            1 => LoopType::ForwardLoop,
            2 => LoopType::PingPongLoop,
            _ => LoopType::NoLoop
        }
    }
}

#[derive(Clone, Debug)]
pub struct Sample {
    pub length: u32,
    pub loop_start: u32,
    pub loop_end: u32,
    pub loop_len: u32,
    pub volume: u8,
    pub finetune: i8,
    pub loop_type: LoopType,
    pub bitness: u8,
    pub panning: u8,
    pub relative_note: i8,
    pub name: String,
    pub global_volume: u8,
    pub surround: bool,
    pub is_ping_pong: bool,
    pub original_loop_end: u32,
    pub data: Vec<f32>
}

impl Sample {
    pub fn new() -> Sample {
        Sample {
            length: 0,
            loop_start: 0,
            loop_end: 0,
            loop_len: 0,
            volume: 0,
            finetune: 0,
            loop_type: LoopType::NoLoop,
            bitness: 0,
            panning: 0,
            relative_note: 0,
            name: "".to_string(),
            global_volume: 64,
            surround: false,
            is_ping_pong: false,
            original_loop_end: 0,
            data: vec![]
        }
    }

    pub fn unpack_i16(mut data: Vec<i16>) -> Vec<i16> {
        for i in 1..data.len() {
            data[i] = (Wrapping(data[i - 1]) + Wrapping(data[i])).0;
        }
        data
    }

    pub fn unpack_i8(mut data: Vec<i8>) -> Vec<i8> {
        for i in 1..data.len() {
            data[i] = (Wrapping(data[i - 1]) + Wrapping(data[i])).0;
        }
        data
    }

    pub fn upsamplei8(data: Vec<i8>) -> Vec<i16> {
        let mut result = vec!(0i16; data.len());
        result.reserve_exact(data.len() as usize);
        for i in 0..data.len() {
            result[i] = (Wrapping((((data[i] as i16) + 128i16) as u16 * 0x0101u16) as u16) + Wrapping((-32768i16) as u16)).0 as i16;
        }
        result
    }

    pub fn upsampleu8(data: Vec<u8>) -> Vec<i16> {
        let mut result = vec!(0i16; data.len());
        result.reserve_exact(data.len() as usize);
        for i in 0..data.len() {
            result[i] = (Wrapping((data[i]  as u16 * 0x0101u16) as u16) + Wrapping((-32768i16) as u16)).0 as i16;
        }
        result
    }


    pub fn upsamplei16(data: Vec<i16>) -> Vec<f32> {
        let mut result = vec!(0.0f32; data.len());
        result.reserve_exact(data.len() as usize);
        for i in 0..data.len() {
            result[i] = data[i] as f32 / 32768.0;
        }
        result
    }

    pub(crate) fn read_s3m_sample_data<R: Read + Seek>(&mut self, file: &mut R, sample_ptr: u32) -> SimpleResult<()> {
        if self.length == 0 { return Ok(()); }
        file.seek(SeekFrom::Start((sample_ptr as u64)  * 16))?;

        if self.bitness == 8 {
            self.data = Sample::upsamplei16(Sample::upsampleu8(file.read_bytes(self.length as usize)?));
        } else {
            return Err(SimpleError::new("Unknown S3M sample format"));
        }
        self.setup_loops_and_padding();
        Ok(())
    }

    pub(crate) fn read_non_packed_data<R: Read>(&mut self, file: &mut R) -> SimpleResult<()> {
        if self.length == 0 { return Ok(()); }
        if self.bitness == 8 {
            self.data = Sample::upsamplei16(Sample::upsamplei8(file.read_i8_vec(self.length as usize)?));
        } else {
            self.data = Sample::upsamplei16(file.read_i16_vec(self.length as usize)?);
        }
        self.setup_loops_and_padding();
        Ok(())
    }

    pub(crate) fn read_data<R: Read>(&mut self, file: &mut R) -> SimpleResult<()> {
        if self.length == 0 { return Ok(()); }
        if self.bitness == 8 {
            self.data = Sample::upsamplei16(Sample::upsamplei8(Sample::unpack_i8(file.read_i8_vec(self.length as usize)?)));
        } else {
            self.data = Sample::upsamplei16(Sample::unpack_i16(file.read_i16_vec(self.length as usize)?));
        }
        self.setup_loops_and_padding();
        Ok(())
    }

    pub(crate) fn setup_loops_and_padding(&mut self) {
        if self.length == 0 || self.data.is_empty() { return; }

        self.original_loop_end = self.loop_end;
        if self.loop_type == LoopType::PingPongLoop {
            self.is_ping_pong = true;
            let mut reversed = Vec::new();
            for i in (self.loop_start..self.loop_end).rev() {
                reversed.push(self.data[i as usize]);
            }
            self.data.splice(self.loop_end as usize..self.loop_end as usize, reversed);
            self.loop_end += self.loop_len;
            self.length += self.loop_len;
            self.loop_len *= 2;
            self.loop_type = LoopType::ForwardLoop;
        }

        // Add 4 samples at the end for suffix padding
        if self.loop_type == LoopType::ForwardLoop {
            for i in 0..4 {
                let idx = (self.loop_start as usize + i).min(self.loop_end as usize - 1);
                self.data.push(self.data[idx]);
            }
        } else {
            let last = *self.data.last().unwrap();
            for _ in 0..4 {
                self.data.push(last);
            }
        }

        // Add 4 samples at the beginning for prefix padding
        let mut prefix = Vec::new();
        if self.loop_type == LoopType::ForwardLoop {
            for i in 1..=4 {
                let idx = if self.loop_end as usize >= i { self.loop_end as usize - i } else { self.loop_start as usize };
                prefix.push(self.data[idx]);
            }
            prefix.reverse();
        } else {
            let first = self.data[0];
            for _ in 0..4 {
                prefix.push(first);
            }
        }
        self.data.splice(0..0, prefix);

        // Offset all loop points and length by the 4-sample prefix
        self.loop_start += 4;
        self.loop_end += 4;
        self.length += 4;
    }
}

#[derive(Debug, Clone)]
pub struct VibratoEnvelope {
    pub vibrato_type: u8,
    pub vibrato_sweep: u8,
    pub vibrato_depth: u8,
    pub vibrato_rate: u8,
}

impl VibratoEnvelope {
    pub(crate) fn new() -> Self {
        Self {
            vibrato_type: 0,
            vibrato_sweep: 0,
            vibrato_depth: 0,
            vibrato_rate: 0,
        }
    }

    pub(crate) fn create(vibrato_type: u8, vibrato_sweep: u8, vibrato_depth: u8, vibrato_rate: u8) -> Self {
        Self {
            vibrato_type,
            vibrato_sweep,
            vibrato_depth,
            vibrato_rate,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Instrument {
    pub name: String,
    pub idx: u8,
    pub sample_indexes: Vec<(u8, u8)>, // (note, sample_idx) - Keyboard mapping
    pub volume_envelope: Envelope,
    pub panning_envelope: Envelope,
    pub pitch_envelope: Envelope,
    pub vibrato_envelope: VibratoEnvelope,
    pub volume_fadeout: u16,
    pub nna: u8,
    pub dct: u8,
    pub dca: u8,
    pub global_volume: u8,
    pub initial_filter_cutoff: u8,
    pub initial_filter_resonance: u8,
    pub is_filter_envelope: bool,
    pub samples: Vec<Sample>,
}

impl Instrument {
    pub(crate) fn new() -> Instrument {
        Instrument {
            name: "".to_string(),
            idx: 0,
            sample_indexes: vec![(0u8, 0u8); 120],
            volume_envelope: Envelope::new(),
            panning_envelope: Envelope::new(),
            pitch_envelope: Envelope::new(),
            vibrato_envelope: VibratoEnvelope::new(),
            volume_fadeout: 0,
            nna: 0,
            dct: 0,
            dca: 0,
            global_volume: 64,
            initial_filter_cutoff: 127,
            initial_filter_resonance: 0,
            is_filter_envelope: false,
            samples: vec![Sample::new(); 1]
        }
    }
}

pub (crate) type Instruments = Vec<Instrument>;
