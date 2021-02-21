use std::io::{Cursor, Read};

use byteorder::{ByteOrder, LittleEndian, ReadBytesExt};


pub trait BinaryReader {
    fn read_string  (&mut self, size: usize) -> String;
    fn read_byte    (&mut self) -> u8;
    fn read_u8      (&mut self) -> u8;
    fn read_i8      (&mut self) -> i8;
    fn read_u16     (&mut self) -> u16;
    fn read_u16_be  (&mut self) -> u16;
    fn read_u24     (&mut self) -> u32;
    fn read_u32     (&mut self) -> u32;
    fn read_u32_be  (&mut self) -> u32;
    fn read_bytes   (&mut self, size: usize) -> Vec<u8>;
    fn read_i16_vec (&mut self, size: usize) -> Vec<i16>;
    fn read_u16_vec (&mut self, size: usize) -> Vec<u16>;
    fn read_u32_vec (&mut self, size: usize) -> Vec<u32>;
    fn read_i8_vec  (&mut self, size: usize) -> Vec<i8>;
}

impl<R: Read> BinaryReader for R {
    fn read_string(&mut self, size: usize) -> String {
        read_string(self, size)
    }

    fn read_u8(&mut self) -> u8 {
        read_u8(self)
    }

    fn read_byte(&mut self) -> u8 {
        read_u8(self)
    }

    fn read_i8(&mut self) -> i8 {
        read_i8(self)
    }

    fn read_u16(&mut self) -> u16 {
        read_u16(self)
    }

    fn read_u16_be(&mut self) -> u16 {
        read_u16_be(self)
    }

    fn read_u24(&mut self) -> u32 {
        // read_u24(self)
        panic!("useless");
    }

    fn read_u32(&mut self) -> u32 {
        read_u32(self)
    }

    fn read_u32_be(&mut self) -> u32 {
        read_u32_be(self)
    }

    fn read_bytes(&mut self, size: usize) -> Vec<u8> {
        read_bytes(self, size)
    }

    fn read_i16_vec(&mut self, size: usize) -> Vec<i16> {
        read_i16_vec(self, size)
    }

    fn read_u16_vec(&mut self, size: usize) -> Vec<u16> {
        read_u16_vec(self, size)
    }

    fn read_u32_vec(&mut self, size: usize) -> Vec<u32> {
        read_u32_vec(self, size)
    }

    fn read_i8_vec(&mut self, size: usize) -> Vec<i8> {
        read_i8_vec(self, size)
    }
}

pub(crate) fn read_string<R: Read>(file: &mut R, size: usize) -> String {
    let mut buf = vec!(0u8; size);
    match file.read_exact(&mut buf) {
        Ok(_) => {}
        Err(_) => {dbg!("Read partial vec");}
    }
    for c in &mut buf {
        if *c < 32 || *c > 127 {
            *c = 32;
        }
    }
    String::from_utf8_lossy(&buf).parse().unwrap()
}

pub(crate) fn read_u8<R: Read>(file: &mut R) -> u8 {
    let mut buf = [0u8;1];
    match file.read_exact(&mut buf) {
        Ok(_) => {}
        Err(_) => {dbg!("Read partial vec");}
    }
    buf[0]
}

pub(crate) fn read_i8<R: Read>(file: &mut R) -> i8 {
    let mut buf = [0u8;1];
    match file.read_exact(&mut buf) {
        Ok(_) => {}
        Err(_) => {dbg!("Read partial vec");}
    }
    buf[0] as i8
}

pub(crate) fn read_u16<R: Read>(file: &mut R) -> u16 {
    let mut buf = [0u8;2];
    match file.read_exact(&mut buf) {
        Ok(_) => {}
        Err(_) => {dbg!("Read partial vec");}
    }
    u16::from_le_bytes(buf)
}

pub(crate) fn read_u16_be<R: Read>(file: &mut R) -> u16 {
    let mut buf = [0u8;2];
    match file.read_exact(&mut buf) {
        Ok(_) => {}
        Err(_) => {dbg!("Read partial vec");}
    }
    u16::from_be_bytes(buf)
}

pub(crate) fn read_u24<R: Read>(file: &mut R) -> u32 {
    let mut buf = [0u8;3];
    match file.read_exact(&mut buf) {
        Ok(_) => {}
        Err(_) => {dbg!("Read partial vec");}
    }
    (((buf[0] as u32) << 16) | ((buf[2] as u32) << 8) | (buf[1] as u32)) as u32
}

pub(crate) fn read_u32<R: Read>(file: &mut R) -> u32 {
    let mut buf = [0u8;4];
    match file.read_exact(&mut buf) {
        Ok(_) => {}
        Err(_) => {dbg!("Read partial vec");}
    }
    u32::from_le_bytes(buf)
}

pub(crate) fn read_u32_be<R: Read>(file: &mut R) -> u32 {
    let mut buf = [0u8;4];
    match file.read_exact(&mut buf) {
        Ok(_) => {}
        Err(_) => {dbg!("Read partial vec");}
    }
    u32::from_be_bytes(buf)
}

pub(crate) fn read_bytes<R: Read>(file: &mut R, size: usize) -> Vec<u8> {
    let mut buf = vec!(0u8; size);
    match file.read_exact(&mut buf) {
        Ok(_) => {}
        Err(_) => {dbg!("Read partial vec");}
    }
    buf
}

pub(crate) fn read_i16_vec<R: Read>(file: &mut R, size: usize) -> Vec<i16> {
    let mut result = vec!(0i16; size);
    let mut buf = vec!(0u8; size * 2);
    match file.read_exact(&mut buf) {
        Ok(_) => {}
        Err(_) => {dbg!("Read partial vec");}
    }

    LittleEndian::read_i16_into(buf.as_slice(), result.as_mut_slice());
    result
}

pub(crate) fn read_u16_vec<R: Read>(file: &mut R, size: usize) -> Vec<u16> {
    let mut result = vec!(0u16; size);
    let mut buf = vec!(0u8; size * 2);
    match file.read_exact(&mut buf) {
        Ok(_) => {}
        Err(_) => {dbg!("Read partial vec");}
    }

    LittleEndian::read_u16_into(buf.as_slice(), result.as_mut_slice());
    result
}

pub(crate) fn read_u32_vec<R: Read>(file: &mut R, size: usize) -> Vec<u32> {
    let mut result = vec!(0u32; size);
    let mut buf = vec!(0u8; size * 4);
    match file.read_exact(&mut buf) {
        Ok(_) => {}
        Err(_) => {dbg!("Read partial vec");}
    }

    LittleEndian::read_u32_into(buf.as_slice(), result.as_mut_slice());
    result
}


pub(crate) fn read_i8_vec<R: Read>(file: &mut R, size: usize) -> Vec<i8> {
    let mut result = vec!(0i8; size);
    let mut buf = vec!(0u8; size);
    match file.read_exact(&mut buf) {
        Ok(_) => {}
        Err(_) => {dbg!("Read partial vec");}
    }

    let mut rdr = Cursor::new(buf);
    rdr.read_i8_into(result.as_mut_slice()).unwrap();
    result
}

pub(crate) fn read_u8_vec<R: Read>(file: &mut R, size: usize) -> Vec<u8> {
    let mut buf = vec!(0u8; size);
    match file.read_exact(&mut buf) {
        Ok(_) => {}
        Err(_) => {dbg!("Read partial vec");}
    }

    buf
}
