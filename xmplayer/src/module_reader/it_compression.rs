use crate::{SimpleResult, SimpleError};
use std::io::{Read, Cursor};
use binary_reader_io::BinaryReader;

pub(crate) struct ITDecompressor<'a> {
    reader: Cursor<&'a [u8]>,
    bit_buf: u32,
    bit_count: u32,
}

impl<'a> ITDecompressor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            reader: Cursor::new(data),
            bit_buf: 0,
            bit_count: 0,
        }
    }

    fn read_bits(&mut self, count: u32) -> SimpleResult<u32> {
        while self.bit_count < count {
            let mut buf = [0u8; 1];
            if self.reader.read_exact(&mut buf).is_err() {
                return Ok(0); // EOF
            }
            self.bit_buf |= (buf[0] as u32) << self.bit_count;
            self.bit_count += 8;
        }
        let res = self.bit_buf & ((1 << count) - 1);
        self.bit_buf >>= count;
        self.bit_count -= count;
        Ok(res)
    }

    pub fn decompress_8bit(data: &[u8], out: &mut [i8]) -> SimpleResult<()> {
        let mut decompressor = ITDecompressor::new(data);
        let mut bit_width = 9u32;
        let mut out_pos = 0usize;
        let mut last_val = 0i8;

        while out_pos < out.len() {
            let val = decompressor.read_bits(bit_width)?;
            if bit_width <= 6 {
                if val == (1 << (bit_width - 1)) {
                    let mut new_width = decompressor.read_bits(3)? + 1;
                    if new_width >= bit_width { new_width += 1; }
                    bit_width = new_width;
                    continue;
                }
            } else if bit_width < 9 {
                let lower = (1 << (bit_width - 1)) - (1 << (8 - bit_width));
                let upper = (1 << (bit_width - 1)) + (1 << (8 - bit_width));
                if val >= lower && val <= upper {
                    let mut new_width = decompressor.read_bits(3)? + 1;
                    if new_width >= bit_width { new_width += 1; }
                    bit_width = new_width;
                    continue;
                }
            } else if bit_width == 9 {
                if val & 0x100 != 0 {
                    bit_width = (val & 0xFF) + 1;
                    continue;
                }
            } else {
                return Err(SimpleError::new("Invalid bit width in IT decompression"));
            }

            let mut final_val = val as i32;
            let shift = 32 - bit_width;
            final_val <<= shift;
            final_val >>= shift;
            
            last_val = last_val.wrapping_add(final_val as i8);
            out[out_pos] = last_val;
            out_pos += 1;
        }
        Ok(())
    }

    pub fn decompress_16bit(data: &[u8], out: &mut [i16]) -> SimpleResult<()> {
        let mut decompressor = ITDecompressor::new(data);
        let mut bit_width = 17u32;
        let mut out_pos = 0usize;
        let mut last_val = 0i16;

        while out_pos < out.len() {
            let val = decompressor.read_bits(bit_width)?;
            if bit_width <= 6 {
                if val == (1 << (bit_width - 1)) {
                    let mut new_width = decompressor.read_bits(4)? + 1;
                    if new_width >= bit_width { new_width += 1; }
                    bit_width = new_width;
                    continue;
                }
            } else if bit_width < 17 {
                let lower = (1 << (bit_width - 1)) - (1 << (16 - bit_width));
                let upper = (1 << (bit_width - 1)) + (1 << (16 - bit_width));
                if val >= lower && val <= upper {
                    let mut new_width = decompressor.read_bits(4)? + 1;
                    if new_width >= bit_width { new_width += 1; }
                    bit_width = new_width;
                    continue;
                }
            } else if bit_width == 17 {
                if val & 0x10000 != 0 {
                    bit_width = (val & 0xFFFF) + 1;
                    continue;
                }
            } else {
                 return Err(SimpleError::new("Invalid bit width in IT decompression"));
            }

            let mut final_val = val as i32;
            let shift = 32 - bit_width;
            final_val <<= shift;
            final_val >>= shift;
            
            last_val = last_val.wrapping_add(final_val as i16);
            out[out_pos] = last_val;
            out_pos += 1;
        }
        Ok(())
    }
}

pub(crate) fn decompress_it_block_8bit(data: &[u8], out: &mut [i8]) -> SimpleResult<()> {
    ITDecompressor::decompress_8bit(data, out)
}

pub(crate) fn decompress_it_block_16bit(data: &[u8], out: &mut [i16]) -> SimpleResult<()> {
    ITDecompressor::decompress_16bit(data, out)
}
