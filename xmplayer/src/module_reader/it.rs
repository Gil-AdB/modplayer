pub(crate) mod it {
    use core::result::Result::{Err, Ok};
    use std::io::{Read, Seek, SeekFrom};
    use std::io;

    use simple_error::SimpleError;

    use crate::envelope::{EnvelopePoint, EnvelopePoints};
    use crate::instrument::{Instrument, LoopType, Sample};
    use crate::io_helpers;
    use crate::io_helpers::{BinaryReader, read_string};
    use crate::module_reader::{Patterns, Row, SongData};
    use crate::pattern::Pattern;
    use crate::simple_error::SimpleResult;

    fn read_patterns<R: Read>(file: &mut R, pattern_count: usize, channel_count: usize) -> Vec<Patterns> {
        let mut patterns: Vec<Patterns> = vec![];
        patterns.reserve_exact(pattern_count as usize);

        for _pattern_idx in 0..pattern_count {
            let _pattern_header_size = io_helpers::read_u32(file);
            let _pattern_type = io_helpers::read_u8(file);
            let row_count = io_helpers::read_u16(file);
            let pattern_size = io_helpers::read_u16(file);

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
                    let flags = io_helpers::read_u8(file);
                    channels.push(if flags & 0x80 == 0x80 {
                        pos += 1;
                        let note = if flags & 1 == 1 {
                            pos += 1;
                            io_helpers::read_u8(file)
                        } else { 0 };
                        let instrument = if flags & 2 == 2 {
                            pos += 1;
                            io_helpers::read_u8(file)
                        } else { 0 };
                        let volume = if flags & 4 == 4 {
                            pos += 1;
                            io_helpers::read_u8(file)
                        } else { 0 };
                        let effect = if flags & 8 == 8 {
                            pos += 1;
                            io_helpers::read_u8(file)
                        } else { 0 };
                        let effect_param = if flags & 16 == 16 {
                            pos += 1;
                            io_helpers::read_u8(file)
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
                        let instrument = io_helpers::read_u8(file);
                        let volume = io_helpers::read_u8(file);
                        let effect = io_helpers::read_u8(file);
                        let effect_param = io_helpers::read_u8(file);
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
            point.frame = io_helpers::read_u16(file);
            point.value = io_helpers::read_u16(file);
        }
        result
    }

    fn read_samples<R: Read>(file: &mut R, sample_count: usize) -> Vec<Sample> {
        let mut samples: Vec<Sample> = vec![];
        samples.reserve_exact(sample_count as usize);

        for sample_idx in 0..sample_count {
            println!("Reading sample #{} of {}", sample_idx, sample_count);

            let mut length = io_helpers::read_u32(file);
            let mut loop_start = io_helpers::read_u32(file);
            let mut loop_len = io_helpers::read_u32(file);
            let volume = io_helpers::read_u8(file);
            let finetune = io_helpers::read_i8(file);
            let flags = io_helpers::read_u8(file);
            let panning = io_helpers::read_u8(file);
            let relative_note = io_helpers::read_i8(file);
            let _reserved = io_helpers::read_u8(file);
            let name = io_helpers::read_string(file, 22);

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
            let id = io_helpers::read_string(file, 4);
            if id != "IMPI" {
                return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Error in reading IT instrument - wrong ID")));
            }

            let _dos_name = io_helpers::read_string(file, 12);
            let _zero = io_helpers::read_u8(file);
            let _nna = io_helpers::read_u8(file);
            let _dct = io_helpers::read_u8(file);
            let _dca = io_helpers::read_u8(file);
            let _fade_out = io_helpers::read_u16(file);
            let _pps = io_helpers::read_i8(file);
            let _ppc = io_helpers::read_u8(file);
            let _gv = io_helpers::read_u8(file);
            let _dfp = io_helpers::read_u8(file);
            let _rvv = io_helpers::read_u8(file);
            let _rpv = io_helpers::read_u8(file);
            let _tv = io_helpers::read_u16(file);
            let _nos = io_helpers::read_u8(file);
            let _x = io_helpers::read_u8(file);
            let name = io_helpers::read_string(file, 26);
            let _ifc = io_helpers::read_u8(file);
            let _ifr = io_helpers::read_u8(file);
            let _mc = io_helpers::read_u8(file);
            let _mp = io_helpers::read_u8(file);
            let _mb = io_helpers::read_u16(file);
            let _nsi = io_helpers::read_bytes(file, 240);

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

    fn read_it_header<R: Read + Seek>(mut file: &mut R) -> SimpleResult<SongData>
    {
        let id = io_helpers::read_string(&mut file, 4);
        if id != "IMPM" {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Not an IT module")));
        }

        let name = io_helpers::read_string(&mut file, 26);
        let _ = io_helpers::read_u16(file);
        let order_count = io_helpers::read_u16(file);
        let instrument_count = io_helpers::read_u16(file);
        let sample_count = io_helpers::read_u16(file);
        let pattern_count = io_helpers::read_u16(file);
        let _ = io_helpers::read_u16(file);
        let compatible_with_version = io_helpers::read_u16(file);

        if compatible_with_version < 0x200 {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "IT module is not in a compatible format")));
        }

        let flags = io_helpers::read_u16(file);
        let special = io_helpers::read_u16(file);
        let _ = io_helpers::read_u8(file);
        let _ = io_helpers::read_u8(file);
        let speed = io_helpers::read_u8(file);
        let tempo = io_helpers::read_u8(file);
        let _ = io_helpers::read_u8(file);
        let _ = io_helpers::read_u8(file);
        let message_length = io_helpers::read_u16(file);
        let message_offset = io_helpers::read_u32(file);
        let _ = io_helpers::read_u32(file);
        let _ = io_helpers::read_u8_vec(file, 64);
        let _ = io_helpers::read_u8_vec(file, 64);

        let mut pattern_order = io_helpers::read_bytes(file, order_count as usize);
        truncate_patterns(&mut pattern_order);

        let instrument_ptrs = file.read_u32_vec(instrument_count as usize);
        let _sample_ptrs = file.read_u32_vec(sample_count as usize); // samples not fully implemented yet
        let pattern_ptrs = file.read_u32_vec(pattern_count as usize);

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
            song_message = io_helpers::read_string(file, message_length as usize);
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

    pub fn read_it<R: Read + Seek>(mut file: &mut R) -> SimpleResult<SongData> {
        let _ = file.seek(SeekFrom::Start(0));
        read_it_header(&mut file)
    }
}
