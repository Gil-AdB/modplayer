use std::io::{Cursor, Read};

use byteorder::{ByteOrder, LittleEndian, ReadBytesExt};

pub(crate) fn read_string<R: Read>(file: &mut R, size: usize) -> String {
    let mut buf = vec!(0u8; size);
    file.read_exact(&mut buf).unwrap();
    for c in &mut buf {
        if *c < 32 || *c > 127 {
            *c = 32;
        }
    }
    String::from_utf8_lossy(&buf).parse().unwrap()
}

pub(crate) fn read_u8<R: Read>(file: &mut R) -> u8 {
    let mut buf = [0u8;1];
    file.read_exact(&mut buf).unwrap();
    buf[0]
}

pub(crate) fn read_i8<R: Read>(file: &mut R) -> i8 {
    let mut buf = [0u8;1];
    file.read_exact(&mut buf).unwrap();
    buf[0] as i8
}

pub(crate) fn read_u16<R: Read>(file: &mut R) -> u16 {
    let mut buf = [0u8;2];
    file.read_exact(&mut buf).unwrap();
    u16::from_le_bytes(buf)
}

pub(crate) fn read_u16_be<R: Read>(file: &mut R) -> u16 {
    let mut buf = [0u8;2];
    file.read_exact(&mut buf).unwrap();
    u16::from_be_bytes(buf)
}


pub(crate) fn read_u32<R: Read>(file: &mut R) -> u32 {
    let mut buf = [0u8;4];
    file.read_exact(&mut buf).unwrap();
    u32::from_le_bytes(buf)
}

pub(crate) fn read_u32_be<R: Read>(file: &mut R) -> u32 {
    let mut buf = [0u8;4];
    file.read_exact(&mut buf).unwrap();
    u32::from_be_bytes(buf)
}

pub(crate) fn read_bytes<R: Read>(file: &mut R, size: usize) -> Vec<u8> {
    let mut buf = vec!(0u8; size);
    file.read_exact(&mut buf).unwrap();
    buf
}

pub(crate) fn read_i16_vec<R: Read>(file: &mut R, size: usize) -> Vec<i16> {
    let mut result = vec!(0i16; size);
    let mut buf = vec!(0u8; size * 2);
    file.read_exact(&mut buf).unwrap();

    LittleEndian::read_i16_into(buf.as_slice(), result.as_mut_slice());
    result
}

pub(crate) fn read_i8_vec<R: Read>(file: &mut R, size: usize) -> Vec<i8> {
    let mut result = vec!(0i8; size);
    let mut buf = vec!(0u8; size);
    file.read_exact(&mut buf).unwrap();

    let mut rdr = Cursor::new(buf);
    rdr.read_i8_into(result.as_mut_slice()).unwrap();
    result
}
