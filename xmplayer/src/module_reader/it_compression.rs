use crate::SimpleResult;
use std::io::{Read, Cursor};

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

    pub fn decompress_8bit(data: &[u8], out: &mut [i8], it215: bool) -> SimpleResult<()> {
        let mut decompressor = ITDecompressor::new(data);
        let mut bit_width = 9u32;
        let mut out_pos = 0usize;
        // IT215 applies the integrator twice over the same delta stream.
        // delta1 = sum of deltas, delta2 = sum of delta1. IT 2.14 uses only
        // delta1.
        let mut delta1 = 0i8;
        let mut delta2 = 0i8;

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
                // bit_width > 9 means the previous `bit_width = (val & 0xFF) + 1`
                // assignment produced an out-of-spec width — libopenmpt's
                // reference (ITCompression.cpp, Mode C) silently terminates
                // the block here and zero-fills the remainder. Returning
                // an Err would reject otherwise-loadable IT files; break
                // matches reference behavior.
                break;
            }

            let mut final_val = val as i32;
            let shift = 32 - bit_width;
            final_val <<= shift;
            final_val >>= shift;

            delta1 = delta1.wrapping_add(final_val as i8);
            let out_val = if it215 {
                delta2 = delta2.wrapping_add(delta1);
                delta2
            } else {
                delta1
            };
            out[out_pos] = out_val;
            out_pos += 1;
        }
        Ok(())
    }

    pub fn decompress_16bit(data: &[u8], out: &mut [i16], it215: bool) -> SimpleResult<()> {
        let mut decompressor = ITDecompressor::new(data);
        let mut bit_width = 17u32;
        let mut out_pos = 0usize;
        let mut delta1 = 0i16;
        let mut delta2 = 0i16;

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
                // See the 8-bit decoder above — out-of-spec width signals
                // early end-of-block in libopenmpt, not a hard error.
                break;
            }

            let mut final_val = val as i32;
            let shift = 32 - bit_width;
            final_val <<= shift;
            final_val >>= shift;

            delta1 = delta1.wrapping_add(final_val as i16);
            let out_val = if it215 {
                delta2 = delta2.wrapping_add(delta1);
                delta2
            } else {
                delta1
            };
            out[out_pos] = out_val;
            out_pos += 1;
        }
        Ok(())
    }
}

pub(crate) fn decompress_it_block_8bit(data: &[u8], out: &mut [i8], it215: bool) -> SimpleResult<()> {
    ITDecompressor::decompress_8bit(data, out, it215)
}

pub(crate) fn decompress_it_block_16bit(data: &[u8], out: &mut [i16], it215: bool) -> SimpleResult<()> {
    ITDecompressor::decompress_16bit(data, out, it215)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pack a sequence of (value, width-in-bits) into a little-endian bit
    /// stream matching ITDecompressor::read_bits's layout: bits accumulate
    /// least-significant-first within each byte and across bytes.
    fn pack_bits(values: &[(u32, u32)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut buf: u64 = 0;
        let mut count: u32 = 0;
        for &(val, width) in values {
            buf |= (val as u64) << count;
            count += width;
            while count >= 8 {
                bytes.push((buf & 0xFF) as u8);
                buf >>= 8;
                count -= 8;
            }
        }
        if count > 0 {
            bytes.push((buf & 0xFF) as u8);
        }
        bytes
    }

    #[test]
    fn test_decompress_8bit_it214_single_integration() {
        // Four 9-bit deltas, each = 1. it214 (single integration) should
        // produce the running sum: 1, 2, 3, 4.
        let stream = pack_bits(&[(1, 9), (1, 9), (1, 9), (1, 9)]);
        let mut out = [0i8; 4];
        decompress_it_block_8bit(&stream, &mut out, false).unwrap();
        assert_eq!(out, [1, 2, 3, 4]);
    }

    #[test]
    fn test_decompress_8bit_it215_double_integration() {
        // Same input as above, but with the IT 2.15+ second-integration pass:
        // delta1 = 1, 2, 3, 4
        // delta2 = 1, 3, 6, 10  (running sum of delta1)
        let stream = pack_bits(&[(1, 9), (1, 9), (1, 9), (1, 9)]);
        let mut out = [0i8; 4];
        decompress_it_block_8bit(&stream, &mut out, true).unwrap();
        assert_eq!(out, [1, 3, 6, 10]);
    }

    #[test]
    fn test_decompress_16bit_it215_double_integration() {
        // 17-bit deltas of value 1, four samples.
        let stream = pack_bits(&[(1, 17), (1, 17), (1, 17), (1, 17)]);
        let mut out214 = [0i16; 4];
        let mut out215 = [0i16; 4];
        decompress_it_block_16bit(&stream, &mut out214, false).unwrap();
        decompress_it_block_16bit(&stream, &mut out215, true).unwrap();
        assert_eq!(out214, [1, 2, 3, 4]);
        assert_eq!(out215, [1, 3, 6, 10]);
    }
}
