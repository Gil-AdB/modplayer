use std::io::{Read, Result};
use byteorder::{ReadBytesExt, LittleEndian, BigEndian};

/// Extension trait for `std::io::Read` to provide convenient binary reading methods.
pub trait BinaryReader: Read {
    /// Reads a fixed-length string and strips non-printable characters.
    fn read_string(&mut self, size: usize) -> String;

    /// Reads a single byte.
    fn read_u8(&mut self) -> Result<u8> {
        let mut buf = [0u8; 1];
        self.read_exact(&mut buf)?;
        Ok(buf[0])
    }

    /// Reads a signed 8-bit integer.
    fn read_i8(&mut self) -> Result<i8> {
        let mut buf = [0u8; 1];
        self.read_exact(&mut buf)?;
        Ok(buf[0] as i8)
    }

    /// Reads a 16-bit unsigned integer (Little Endian).
    fn read_u16(&mut self) -> Result<u16> {
        ReadBytesExt::read_u16::<LittleEndian>(self)
    }

    /// Reads a 16-bit unsigned integer (Big Endian).
    fn read_u16_be(&mut self) -> Result<u16> {
        ReadBytesExt::read_u16::<BigEndian>(self)
    }

    /// Reads a 24-bit unsigned integer (Little Endian).
    fn read_u24(&mut self) -> Result<u32> {
        ReadBytesExt::read_u24::<LittleEndian>(self)
    }

    /// Reads a 24-bit unsigned integer (Big Endian).
    fn read_u24_be(&mut self) -> Result<u32> {
        ReadBytesExt::read_u24::<BigEndian>(self)
    }

    /// Reads a 32-bit unsigned integer (Little Endian).
    fn read_u32(&mut self) -> Result<u32> {
        ReadBytesExt::read_u32::<LittleEndian>(self)
    }

    /// Reads a 32-bit unsigned integer (Big Endian).
    fn read_u32_be(&mut self) -> Result<u32> {
        ReadBytesExt::read_u32::<BigEndian>(self)
    }

    /// Reads a fixed number of bytes into a Vec.
    fn read_bytes(&mut self, size: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; size];
        self.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// Reads a vector of signed 16-bit integers (little-endian).
    fn read_i16_vec(&mut self, size: usize) -> Result<Vec<i16>> {
        let mut res = vec![0i16; size];
        self.read_i16_into::<LittleEndian>(&mut res)?;
        Ok(res)
    }

    /// Reads a vector of unsigned 16-bit integers (little-endian).
    fn read_u16_vec(&mut self, size: usize) -> Result<Vec<u16>> {
        let mut res = vec![0u16; size];
        self.read_u16_into::<LittleEndian>(&mut res)?;
        Ok(res)
    }

    /// Reads a vector of unsigned 32-bit integers (little-endian).
    fn read_u32_vec(&mut self, size: usize) -> Result<Vec<u32>> {
        let mut res = vec![0u32; size];
        self.read_u32_into::<LittleEndian>(&mut res)?;
        Ok(res)
    }

    /// Reads a vector of signed 8-bit integers.
    fn read_i8_vec(&mut self, size: usize) -> Result<Vec<i8>> {
        let mut buf = vec![0u8; size];
        self.read_exact(&mut buf)?;
        Ok(buf.into_iter().map(|b| b as i8).collect())
    }
}

impl<R: Read> BinaryReader for R {
    fn read_string(&mut self, size: usize) -> String {
        let mut buf = vec![0u8; size];
        if self.read_exact(&mut buf).is_err() {
            return "".to_string();
        }
        for c in &mut buf {
            if *c < 32 || *c > 127 {
                *c = 32;
            }
        }
        String::from_utf8_lossy(&buf).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor};

    #[test]
    fn test_endianness_integers() {
        let data = vec![0x01, 0x02, 0x03, 0x04];
        
        // u8
        assert_eq!(BinaryReader::read_u8(&mut Cursor::new(&data)).unwrap(), 0x01);
        
        // i8
        assert_eq!(BinaryReader::read_i8(&mut Cursor::new(vec![0xFF])).unwrap(), -1);
        
        // u16 LE/BE
        assert_eq!(BinaryReader::read_u16(&mut Cursor::new(&data)).unwrap(), 0x0201);
        assert_eq!(BinaryReader::read_u16_be(&mut Cursor::new(&data)).unwrap(), 0x0102);
        
        // u24 LE/BE
        assert_eq!(BinaryReader::read_u24(&mut Cursor::new(&data)).unwrap(), 0x030201);
        assert_eq!(BinaryReader::read_u24_be(&mut Cursor::new(&data)).unwrap(), 0x010203);
        
        // u32 LE/BE
        assert_eq!(BinaryReader::read_u32(&mut Cursor::new(&data)).unwrap(), 0x04030201);
        assert_eq!(BinaryReader::read_u32_be(&mut Cursor::new(&data)).unwrap(), 0x01020304);
    }

    #[test]
    fn test_vector_reads() {
        let data = vec![0x01, 0x00, 0x02, 0x00, 0x03, 0x00];
        let mut rdr = Cursor::new(&data);
        
        // u16_vec LE
        assert_eq!(rdr.read_u16_vec(3).unwrap(), vec![1, 2, 3]);
        
        let mut rdr = Cursor::new(vec![0xFF, 0xFE]);
        assert_eq!(rdr.read_i8_vec(2).unwrap(), vec![-1, -2]);
    }

    #[test]
    fn test_string_parsing() {
        // Normal string
        assert_eq!(Cursor::new(b"Hello").read_string(5), "Hello");
        
        // Stripping non-printable characters
        let mut rdr = Cursor::new(b"Hi\x07\t\nBye");
        assert_eq!(rdr.read_string(8), "Hi   Bye");
        
        // Non-trimming behavior check (as required by XM loader)
        let mut rdr = Cursor::new(b"Extended Module: ");
        assert_eq!(rdr.read_string(17), "Extended Module: ");
    }

    #[test]
    fn test_eof_handling() {
        let mut rdr = Cursor::new(vec![1, 2]);
        
        // Should succeed
        assert!(BinaryReader::read_u16(&mut rdr).is_ok());
        
        // Should fail with UnexpectedEof
        assert!(BinaryReader::read_u8(&mut rdr).is_err());
        
        let mut rdr = Cursor::new(vec![1, 2]);
        assert!(BinaryReader::read_u32(&mut rdr).is_err());
    }

    #[test]
    fn test_zero_length_reads() {
        let data = vec![1, 2, 3];
        let mut rdr = Cursor::new(&data);
        
        assert_eq!(rdr.read_string(0), "");
        assert_eq!(rdr.position(), 0); // Cursor shouldn't move
        
        assert_eq!(rdr.read_bytes(0).unwrap().len(), 0);
        assert_eq!(rdr.position(), 0);
    }
}
