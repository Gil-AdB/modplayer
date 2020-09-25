use std::io::{Read, Seek, SeekFrom};
use std::num::Wrapping;

use crate::envelope::Envelope;
use crate::io_helpers::{read_i16_vec, read_i8_vec, read_u8_vec};

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
    pub data: Vec<f32>
}

impl Sample {
    fn new() -> Sample {
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
            data: vec![]
        }
    }

    fn unpack_i16(mut data: Vec<i16>) -> Vec<i16> {
        for i in 1..data.len() {
            data[i] = (Wrapping(data[i - 1]) + Wrapping(data[i])).0;
        }
        data
    }

    fn unpack_i8(mut data: Vec<i8>) -> Vec<i8> {
        for i in 1..data.len() {
            data[i] = (Wrapping(data[i - 1]) + Wrapping(data[i])).0;
        }
        data
    }

    fn upsamplei8(data: Vec<i8>) -> Vec<i16> {
        let mut result = vec!(0i16; data.len());
        result.reserve_exact(data.len() as usize);
        for i in 0..data.len() {
            result[i] = (Wrapping((((data[i] as i16) + 128i16) as u16 * 0x0101u16) as u16) + Wrapping((-32768i16) as u16)).0 as i16;
        }
        result
    }

    fn upsampleu8(data: Vec<u8>) -> Vec<i16> {
        let mut result = vec!(0i16; data.len());
        result.reserve_exact(data.len() as usize);
        for i in 0..data.len() {
            result[i] = (Wrapping((data[i]  as u16 * 0x0101u16) as u16) + Wrapping((-32768i16) as u16)).0 as i16;
        }
        result
    }


    fn upsamplei16(data: Vec<i16>) -> Vec<f32> {
        let mut result = vec!(0.0f32; data.len());
        result.reserve_exact(data.len() as usize);
        for i in 0..data.len() {
            result[i] = data[i] as f32 / 32768.0;
        }
        result
    }

    pub(crate) fn read_s3m_sample_data<R: Read + Seek>(&mut self, file: &mut R, sample_ptr: u32) {
        if self.length == 0 { return; }
        file.seek(SeekFrom::Start((sample_ptr as u64)  * 16));

        if self.bitness == 8 {
            self.data = Sample::upsamplei16(Sample::upsampleu8(read_u8_vec(file, self.length as usize)));
        } else {
            panic!("Unknown S3M sample format");
            // self.data = Sample::upsamplei16(read_i16_vec(file, self.length as usize));
        }
        self.data.push(self.data[self.data.len() - 1]);
    }

    pub(crate) fn read_non_packed_data<R: Read>(&mut self, file: &mut R) {
        if self.length == 0 { return; }
        if self.bitness == 8 {
            self.data = Sample::upsamplei16(Sample::upsamplei8(read_i8_vec(file, self.length as usize)));
        } else {
            self.data = Sample::upsamplei16(read_i16_vec(file, self.length as usize));
        }
        self.data.push(self.data[self.data.len() - 1]);
    }

    pub(crate) fn read_data<R: Read>(&mut self, file: &mut R) {
        if self.length == 0 { return; }
        if self.bitness == 8 {
            self.data = Sample::upsamplei16(Sample::upsamplei8(Sample::unpack_i8(read_i8_vec(file, self.length as usize))));
        } else {
            self.data = Sample::upsamplei16(Sample::unpack_i16(read_i16_vec(file, self.length as usize)));
        }
        self.data.push(self.data[self.data.len() - 1]);
    }
}

#[derive(Debug)]
pub struct Instrument {
    pub name: String,
    pub idx: u8,
    pub sample_indexes: Vec<u8>,
    pub volume_envelope: Envelope,
    pub panning_envelope: Envelope,
    pub vibrato_type: u8,
    pub vibrato_sweep: u8,
    pub vibrato_depth: u8,
    pub vibrato_rate: u8,
    pub volume_fadeout: u16,

    pub(crate) samples: Vec<Sample>,
}

impl Instrument {
    pub(crate) fn new() -> Instrument {
        Instrument {
            name: "".to_string(),
            idx: 0,
            sample_indexes: vec![0u8; 96],
            volume_envelope: Envelope::new(),
            panning_envelope: Envelope::new(),
            vibrato_type: 0,
            vibrato_sweep: 0,
            vibrato_depth: 0,
            vibrato_rate: 0,
            volume_fadeout: 0,
            samples: vec![Sample::new(); 1]
        }
    }
}
