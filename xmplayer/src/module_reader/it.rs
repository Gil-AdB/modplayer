use core::result::Result::{Err, Ok};
use std::io::{Read, Seek, SeekFrom};
use std::io;

use simple_error::SimpleError;

use crate::envelope::{EnvelopePoint, EnvelopePoints};
use crate::instrument::{Instrument, LoopType, Sample};
use binary_reader_io::BinaryReader;
use crate::module_reader::{Patterns, Row, SongData};
use crate::pattern::Pattern;
use crate::simple_error::SimpleResult;

    fn read_patterns<R: Read>(file: &mut R, pattern_count: usize, channel_count: usize) -> Vec<Patterns> {
        let mut patterns: Vec<Patterns> = vec![];
        patterns.reserve_exact(pattern_count as usize);

        for _pattern_idx in 0..pattern_count {
            let _pattern_header_size = file.read_u32().unwrap();
            let _pattern_type = file.read_u8().unwrap();
            let row_count = file.read_u16().unwrap();
            let pattern_size = file.read_u16().unwrap();

            let mut pos = 0usize;
            if pattern_size == 0 {
                patterns.push(Patterns {
                    rows: vec![Row {
                        channels: vec![Pattern {
                            note: 0,
                            instrument: 0,
                            volume: 0,
                            effect: 0,
                            effect_param: 0
                        }; channel_count]
                    }; 64]
                });
                continue;
            }

            let mut rows: Vec<Row> = vec![];
            rows.reserve_exact(row_count as usize);
            for _row_idx in 0..row_count {
                let mut channels: Vec<Pattern> = vec![];
                channels.reserve_exact(channel_count);
                for _channel_idx in 0..channel_count {
                    let flags = file.read_u8().unwrap();
                    channels.push(if flags & 0x80 == 0x80 {
                        pos += 1;
                        let note = if flags & 1 == 1 {
                            pos += 1;
                            file.read_u8().unwrap()
                        } else { 0 };
                        let instrument = if flags & 2 == 2 {
                            pos += 1;
                            file.read_u8().unwrap()
                        } else { 0 };
                        let volume = if flags & 4 == 4 {
                            pos += 1;
                            file.read_u8().unwrap()
                        } else { 0 };
                        let effect = if flags & 8 == 8 {
                            pos += 1;
                            file.read_u8().unwrap()
                        } else { 0 };
                        let effect_param = if flags & 16 == 16 {
                            pos += 1;
                            file.read_u8().unwrap()
                        } else { 0 };
                        Pattern {
                            note,
                            instrument,
                            volume,
                            effect,
                            effect_param
                        }
                    } else {
                        let note = flags;
                        let instrument = file.read_u8().unwrap();
                        let volume = file.read_u8().unwrap();
                        let effect = file.read_u8().unwrap();
                        let effect_param = file.read_u8().unwrap();
                        pos += 5;

                        Pattern {
                            note,
                            instrument,
                            volume,
                            effect,
                            effect_param
                        }
                    });
                }
                rows.push(Row { channels });
            }
            if pattern_size as usize != pos {
                panic!("size {} != pos {}", pattern_size, pos)
            }
            patterns.push(Patterns { rows })
        }

        patterns
    }

    fn read_envelope<R: Read>(file: &mut R) -> EnvelopePoints {
        let mut result = [EnvelopePoint::new(); 12];

        for point in &mut result {
            point.frame = file.read_u16().unwrap();
            point.value = file.read_u16().unwrap();
        }
        result
    }

    fn read_samples<R: Read>(file: &mut R, sample_count: usize) -> Vec<Sample> {
        let mut samples: Vec<Sample> = vec![];
        samples.reserve_exact(sample_count as usize);

        for sample_idx in 0..sample_count {
            println!("Reading sample #{} of {}", sample_idx, sample_count);

            let mut length = file.read_u32().unwrap();
            let mut loop_start = file.read_u32().unwrap();
            let mut loop_len = file.read_u32().unwrap();
            let volume = file.read_u8().unwrap();
            let finetune = file.read_i8().unwrap();
            let flags = file.read_u8().unwrap();
            let panning = file.read_u8().unwrap();
            let relative_note = file.read_i8().unwrap();
            let _reserved = file.read_u8().unwrap();
            let name = file.read_string(22);

            let bitness = if (flags & 16) == 16 { 16 } else { 8 };
            if bitness == 16 { // length is in bits
                length /= 2;
                loop_start /= 2;
                loop_len /= 2;
            }

            let loop_type = LoopType::from_flags(flags);
            match loop_type {
                LoopType::NoLoop => {
                    loop_start = 0;
                    loop_len = length;
                }
                _ => {}
            }

            samples.push(Sample {
                length,
                loop_start,
                loop_end: loop_start + loop_len,
                loop_len,
                volume,
                finetune,
                loop_type,
                bitness,
                panning,
                relative_note,
                name,
                data: vec![],
            })
        }

        for sample in &mut samples {
            sample.read_data(file);
        }

        samples
    }

    fn read_instruments<R: Read + Seek>(file: &mut R, instrument_ptrs: &Vec<u32>) -> SimpleResult<Vec<Instrument>> {
        let mut instruments: Vec<Instrument> = vec![];
        let instrument_count = instrument_ptrs.len();

        instruments.reserve_exact(instrument_count + 1);
        instruments.push(Instrument::new());

        for instrument_ptr in instrument_ptrs {
            let mut instrument = Instrument::new();
            if let Err(_) = file.seek(SeekFrom::Start(*instrument_ptr as u64)) {
                return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Error in reading IT instrument - file offset")));
            }
            let id = file.read_string(4);
            if id != "IMPI" {
                return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Error in reading IT instrument - wrong ID")));
            }

            let _dos_name = file.read_string(12);
            let _zero = file.read_u8().unwrap();
            let _nna = file.read_u8().unwrap();
            let _dct = file.read_u8().unwrap();
            let _dca = file.read_u8().unwrap();
            let _fade_out = file.read_u16().unwrap();
            let _pps = file.read_i8().unwrap();
            let _ppc = file.read_u8().unwrap();
            let _gv = file.read_u8().unwrap();
            let _dfp = file.read_u8().unwrap();
            let _rvv = file.read_u8().unwrap();
            let _rpv = file.read_u8().unwrap();
            let _tv = file.read_u16().unwrap();
            let _nos = file.read_u8().unwrap();
            let _x = file.read_u8().unwrap();
            let name = file.read_string(26);
            let _ifc = file.read_u8().unwrap();
            let _ifr = file.read_u8().unwrap();
            let _mc = file.read_u8().unwrap();
            let _mp = file.read_u8().unwrap();
            let _mb = file.read_u16().unwrap();
            let _nsi = file.read_bytes(240).unwrap();

            instrument.name = name.trim().to_string();
            instruments.push(instrument);
        }
        Ok(instruments)
    }

    fn truncate_patterns(pattern_order: &mut Vec<u8>) {
        let mut write_pos = 0;
        for i in 0..pattern_order.len() {
            if pattern_order[i] < 254 {
                pattern_order[write_pos] = pattern_order[i];
                write_pos += 1;
            } else if pattern_order[i] == 255 {
                break;
            }
        }
        pattern_order.truncate(write_pos);
    }

    fn read_it_header<R: Read + Seek>(file: &mut R) -> SimpleResult<SongData>
    {
        let id = file.read_string(4);
        if id != "IMPM" {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Not an IT module")));
        }

        let name = file.read_string(26);
        let _ = file.read_u16().unwrap();
        let order_count = file.read_u16().unwrap();
        let instrument_count = file.read_u16().unwrap();
        let sample_count = file.read_u16().unwrap();
        let pattern_count = file.read_u16().unwrap();
        let _ = file.read_u16().unwrap();
        let compatible_with_version = file.read_u16().unwrap();

        if compatible_with_version < 0x200 {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "IT module is not in a compatible format")));
        }

        let flags = file.read_u16().unwrap();
        let special = file.read_u16().unwrap();
        let _ = file.read_u8().unwrap();
        let _ = file.read_u8().unwrap();
        let _speed = file.read_u8().unwrap();
        let tempo = file.read_u8().unwrap();
        let _ = file.read_u8().unwrap();
        let _ = file.read_u8().unwrap();
        let message_length = file.read_u16().unwrap();
        let message_offset = file.read_u32().unwrap();
        let _ = file.read_u32().unwrap();
        let _ = file.read_bytes(64).unwrap();
        let _ = file.read_bytes(64).unwrap();

        let mut pattern_order = file.read_bytes(order_count as usize).unwrap();
        truncate_patterns(&mut pattern_order);

        let instrument_ptrs = file.read_u32_vec(instrument_count as usize).map_err(|e| SimpleError::new(format!("{}", e)))?;
        let _sample_ptrs = file.read_u32_vec(sample_count as usize).map_err(|e| SimpleError::new(format!("{}", e)))?; // samples not fully implemented yet
        let pattern_ptrs = file.read_u32_vec(pattern_count as usize).map_err(|e| SimpleError::new(format!("{}", e)))?;

        let instruments = read_instruments(file, &instrument_ptrs)?;

        let mut patterns: Vec<Patterns> = vec![];
        for ptr in pattern_ptrs {
            if ptr == 0 {
                patterns.push(Patterns::new(64, 64));
            } else {
                let _ = file.seek(SeekFrom::Start(ptr as u64));
                patterns.extend(read_patterns(file, 1, 64));
            }
        }

        let mut song_message = String::new();
        if (special & 1) == 1 && message_offset > 0 {
            let _ = file.seek(SeekFrom::Start(message_offset as u64));
            song_message = file.read_string(message_length as usize);
        }

        Ok(SongData {
            id: id.trim().to_string(),
            name: name.trim().to_string(),
            song_type: crate::module_reader::SongType::IT,
            tracker_name: "Impulse Tracker".to_string(),
            song_length: pattern_order.len() as u16,
            restart_position: 0,
            channel_count: 64,
            patterns,
            instrument_count,
            frequency_type: if (flags & 1) == 1 { crate::module_reader::FrequencyType::LINEAR } else { crate::module_reader::FrequencyType::AMIGA },
            tempo: tempo as u16,
            bpm: tempo as u16,
            pattern_order,
            instruments,
            use_amiga: (flags & 1) != 1,
            song_message,
        })
    }

    pub(crate) fn read_it<R: Read + Seek>(file: &mut R) -> SimpleResult<SongData> {
        let _ = file.seek(SeekFrom::Start(0));
        read_it_header(file)
    }
