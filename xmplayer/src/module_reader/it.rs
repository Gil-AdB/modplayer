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

        // Instruments are one based, go figure. We'll add an empty instrument as sample 0.
        instruments.reserve_exact(instrument_count + 1 as usize);

        instruments.push(Instrument::new());

        for (_instrument_idx, instrument_ptr) in instrument_ptrs.iter().cloned().enumerate() {
            let mut _instrument = Instrument::new();
            if let Err(_pos) = file.seek(SeekFrom::Start(instrument_ptr as u64)) {return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Error in reading IT instrument - file offset")));}
            let id = read_string(file, 4);
            if id != "IMPI" {
                return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Error in reading IT instrument - wrong ID")));
            }

            let _dos_name = read_string(file, 12);
            let _zero = file.read_u8();
            let _new_note_action = file.read_u8();
            let _duplicate_check_type = file.read_u8();
            let _duplicate_check_action = file.read_u8();
            let _fade_out = file.read_u16();
            let _pitch_pan_separation = file.read_i8();
            let _pitch_pan_center = file.read_u8();
            let _global_volume = file.read_u8();
            let _default_pan = file.read_u8();
            let _random_volume_variation = file.read_u8();
            let _random_panning_variation = file.read_u8();
            let _tracker_version = file.read_u16();
            let _number_of_samples = file.read_u8();
            let _x = file.read_u8();
            let _name = file.read_string(26);
            let _initial_filter_cutoff = file.read_u8();
            let _initial_filter_resonance = file.read_u8();
            let _midi_channel = file.read_u8();
            let _midi_program = file.read_u8();
            let _midi_bank = file.read_u16();
            let _note_sample_indexes = io_helpers::read_bytes(file, 240);

            #[cfg(debug_assertions)]
            {
                eprintln!("IT instrument #{}: name={:?}, nna={}, dct={}, dca={}, fadeout={}, samples={}",
                    _instrument_idx, _name, _new_note_action, _duplicate_check_type,
                    _duplicate_check_action, _fade_out, _number_of_samples);
            }
        }
        return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other,"Unimplemented")));
    }

    fn truncate_patterns(pattern_order: &mut Vec<u8>) {
        let mut write_pos = 0;
        for i in 0..pattern_order.len() as usize {
            if pattern_order[i] < 254 {
                pattern_order[write_pos] = pattern_order[i];
                write_pos += 1;
            } else if pattern_order[i] == 255 {
                break;
            }
        }

        pattern_order.truncate(write_pos as usize);
    }


    fn read_it_header<R: Read + Seek>(mut file: &mut R) -> SimpleResult<SongData>
    {
        let id = io_helpers::read_string(&mut file, 4);
        if id != "IMPM" {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Not an IT module")));
        }

        let name = io_helpers::read_string(&mut file, 26);
        let _philiht = io_helpers::read_u16(file);
        let order_count = io_helpers::read_u16(file);
        let instrument_count = io_helpers::read_u16(file);
        let sample_count = io_helpers::read_u16(file);
        let pattern_count = io_helpers::read_u16(file);
        let created_with_tracker = io_helpers::read_u16(file);
        let compatible_with_version = io_helpers::read_u16(file);

        if compatible_with_version < 0x200 {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "IT module is not in a compatible format")));
        }

        let flags = io_helpers::read_u16(file);
        let special = io_helpers::read_u16(file);
        let global_volume = io_helpers::read_u16(file);
        let mix_volume = io_helpers::read_u16(file);
        let speed = io_helpers::read_u16(file);
        let tempo = io_helpers::read_u16(file);
        let panning_separation = io_helpers::read_u16(file);
        let pitch_wheel_depth = io_helpers::read_u16(file);
        let message_length = io_helpers::read_u16(file);
        let message_offset = io_helpers::read_u32(file);
        let reserved = io_helpers::read_u32(file);
        let channel_panning = io_helpers::read_u8_vec(file, 64);
        let channel_volume = io_helpers::read_u8_vec(file, 64);

        #[cfg(debug_assertions)]
        {
            eprintln!("IT: name={:?}, orders={}, instruments={}, samples={}, patterns={}",
                name, order_count, instrument_count, sample_count, pattern_count);
            eprintln!("  tracker={:#x}, compat={:#x}, flags={:#x}, special={:#x}",
                created_with_tracker, compatible_with_version, flags, special);
            eprintln!("  gvol={}, mvol={}, speed={}, tempo={}, pansep={}, pwd={}",
                global_volume, mix_volume, speed, tempo, panning_separation, pitch_wheel_depth);
        }

        // suppress unused variable warnings in release builds
        let _ = (special, global_volume, mix_volume, panning_separation, pitch_wheel_depth,
                 message_length, message_offset, reserved, channel_panning, channel_volume,
                 name, created_with_tracker, sample_count, pattern_count, speed, tempo, flags);

        let mut pattern_order = io_helpers::read_bytes(file, order_count as usize);

        truncate_patterns(&mut pattern_order);

        let instrument_ptrs = file.read_u32_vec(instrument_count as usize);
        let _sample_ptrs = file.read_u32_vec(sample_count as usize);
        let _pattern_ptrs = file.read_u32_vec(pattern_count as usize);

        let _instruments = read_instruments(file, &instrument_ptrs)?;

        return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other,"Unknown file format")));


        // let stream_position;
        // if let Ok(pos) = file.seek(SeekFrom::Current(0)) { stream_position = pos; } else { stream_position = 20 }
        //
        //
        // let mut patterns = read_patterns(file, pattern_count as usize, channel_count as usize);
        //
        // // fix empty patterns at end
        // for idx in 0..pattern_order.len() {
        //     if pattern_order[idx] >= patterns.len() as u8 {
        //         pattern_order[idx] = patterns.len() as u8;
        //     }
        // }
        // if song_length > pattern_order.len() as u16 {
        //     song_length = pattern_order.len() as u16;
        //     dbg!("Trimming song legth to {}", song_length);
        // }
        // // dbg!(&pattern_order);
        //
        // patterns.push(Patterns {
        //     rows: vec![Row {
        //         channels: vec![Pattern {
        //             note: 0,
        //             instrument: 0,
        //             volume: 0,
        //             effect: 0,
        //             effect_param: 0
        //         }; channel_count as usize]
        //     }; 64]
        // });
        //
        // let instruments = read_instruments(file, instrument_count as usize);
        //
        // Ok(SongData {
        //     id: id.trim().to_string(),
        //     name: name.trim().to_string(),
        //     song_type: SongType::XM,
        //     tracker_name: tracker_name.trim().to_string(),
        //     song_length,
        //     restart_position,
        //     channel_count,
        //     patterns,
        //     instrument_count,
        //     frequency_type: if (flags & 1) == 1 { FrequencyType::LINEAR } else { FrequencyType::AMIGA },
        //     tempo,
        //     bpm,
        //     pattern_order: Vec::from_iter(pattern_order.iter().cloned()),
        //     instruments,
        //     use_amiga: (flags & 1) != 1
        // })
    }

    pub fn read_it<R: Read + Seek>(mut file: &mut R) -> SimpleResult<SongData> {
        let _ = file.seek(SeekFrom::Start(0));

        let file_len = match file.seek(SeekFrom::End(0)) {
            Ok(m) => {
                let _ = file.seek(SeekFrom::Start(0));
                m
            }
            Err(_) => {return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Can't read file metadata")));}
        };

        // println!("file length: {}", file_len);
        if file_len < 0xC0 {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "File is too small!")));
        }

        let song_data = read_it_header(&mut file);

        song_data
    }
}
