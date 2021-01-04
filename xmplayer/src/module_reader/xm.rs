pub(crate) mod xm {
    use std::io::{Read, Seek, SeekFrom, BufReader};
    use core::result::Result::{Err, Ok};
    use crate::module_reader::{Patterns, Row, SongData, SongType, FrequencyType};
    use crate::io_helpers;
    use crate::pattern::Pattern;
    use crate::envelope::{EnvelopePoints, EnvelopePoint, Envelope};
    use crate::instrument::{Sample, LoopType, Instrument, VibratoEnvelope};
    use std::iter::FromIterator;
    use std::fs::{File};
    use crate::simple_error::SimpleResult;
    use simple_error::SimpleError;
    use std::io;

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

        for mut point in &mut result {
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

    fn read_instruments<R: Read + Seek>(file: &mut R, instrument_count: usize) -> Vec<Instrument> {
        let mut instruments: Vec<Instrument> = vec![];

        // Instruments are one based, go figure. We'll add an empty instrument as sample 0.
        instruments.reserve_exact(instrument_count + 1 as usize);

        instruments.push(Instrument::new());

        for instrument_idx in 0..instrument_count {
            let instrument_pos = file.seek(SeekFrom::Current(0)).unwrap();
            let header_size = io_helpers::read_u32(file);
            let name = io_helpers::read_string(file, 22);
            let _instrument_type = io_helpers::read_u8(file);
            let sample_count = io_helpers::read_u16(file);


            if sample_count > 0 {
                let _sample_size = io_helpers::read_u32(file);
                let sample_indexes = io_helpers::read_bytes(file, 96);
                let volume_envelope = read_envelope(file);
                let panning_envelope = read_envelope(file);
                let volume_points = io_helpers::read_u8(file);
                let panning_points = io_helpers::read_u8(file);
                let volume_sustain_point = io_helpers::read_u8(file);
                let volume_loop_start_point = io_helpers::read_u8(file);
                let volume_loop_end_point = io_helpers::read_u8(file);
                let panning_sustain_point = io_helpers::read_u8(file);
                let panning_loop_start_point = io_helpers::read_u8(file);
                let panning_loop_end_point = io_helpers::read_u8(file);
                let volume_type = io_helpers::read_u8(file);
                let panning_type = io_helpers::read_u8(file);
                let vibrato_type = io_helpers::read_u8(file);
                let vibrato_sweep = io_helpers::read_u8(file);
                let vibrato_depth = io_helpers::read_u8(file);
                let vibrato_rate = io_helpers::read_u8(file);
                let volume_fadeout = io_helpers::read_u16(file);
                let _reserved = io_helpers::read_u16(file);

                file.seek(SeekFrom::Start(instrument_pos + header_size as u64)).unwrap();
                instruments.push(Instrument {
                    name,
                    idx: (instrument_idx + 1) as u8,
                    sample_indexes,
                    volume_envelope: Envelope::create(volume_envelope, volume_points, volume_sustain_point, volume_loop_start_point, volume_loop_end_point, volume_type),
                    panning_envelope: Envelope::create(panning_envelope,panning_points, panning_sustain_point, panning_loop_start_point, panning_loop_end_point,panning_type),
                    vibrato_envelope: VibratoEnvelope::create(vibrato_type, vibrato_sweep, vibrato_depth, vibrato_rate),
                    volume_fadeout,
                    samples: read_samples(file, sample_count as usize)
                });
            } else {
                if let Err(e) = file.seek(SeekFrom::Start(instrument_pos + header_size as u64)) { panic!(e); }
                instruments.push(Instrument::new());
            }
        }
        instruments
    }

    fn read_xm_header<R: Read + Seek>(mut file: &mut R) -> SimpleResult<SongData>
    {
        let id = io_helpers::read_string(&mut file, 17);
        if id != "Extended Module: " {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Not an XM module")));
        }

        dbg!(&id);
        let name = io_helpers::read_string(&mut file, 20);
        dbg!(&name);
        let sig = io_helpers::read_u8(file);
        if sig != 0x1a {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Not an XM module")));
        }

        let tracker_name = io_helpers::read_string(file, 20);
        dbg!(&tracker_name);

        let ver = io_helpers::read_u16(file);
        dbg!(format!("{:x}", ver));

//    dbg!(file.seek(SeekFrom::Current(0)));

        let header_size = io_helpers::read_u32(file);
        dbg!(header_size);

        let mut song_length = io_helpers::read_u16(file);
        dbg!(song_length);

        let restart_position = io_helpers::read_u16(file);
        dbg!(restart_position);

        let channel_count = io_helpers::read_u16(file);
        dbg!(channel_count);

        let pattern_count = io_helpers::read_u16(file);
        dbg!(pattern_count);

        let instrument_count = io_helpers::read_u16(file);
        dbg!(instrument_count);

        let flags = io_helpers::read_u16(file);
        dbg!(flags);

        let tempo = io_helpers::read_u16(file);
        dbg!(tempo);

        let bpm = io_helpers::read_u16(file);
        dbg!(bpm);
        let stream_position;
        if let Ok(pos) = file.seek(SeekFrom::Current(0)) { stream_position = pos; } else { stream_position = 20 }

        let mut pattern_order = io_helpers::read_bytes(file, (60 + header_size - stream_position as u32) as usize);

        let mut patterns = read_patterns(file, pattern_count as usize, channel_count as usize);

        // fix empty patterns at end
        for idx in 0..pattern_order.len() {
            if pattern_order[idx] >= patterns.len() as u8 {
                pattern_order[idx] = patterns.len() as u8;
            }
        }
        if song_length > pattern_order.len() as u16 {
            song_length = pattern_order.len() as u16;
            dbg!("Trimming song legth to {}", song_length);
        }
        // dbg!(&pattern_order);

        patterns.push(Patterns {
            rows: vec![Row {
                channels: vec![Pattern {
                    note: 0,
                    instrument: 0,
                    volume: 0,
                    effect: 0,
                    effect_param: 0
                }; channel_count as usize]
            }; 64]
        });

        let instruments = read_instruments(file, instrument_count as usize);

        Ok(SongData {
            id: id.trim().to_string(),
            name: name.trim().to_string(),
            song_type: SongType::XM,
            tracker_name: tracker_name.trim().to_string(),
            song_length,
            restart_position,
            channel_count,
            patterns,
            instrument_count,
            frequency_type: if (flags & 1) == 1 { FrequencyType::LINEAR } else { FrequencyType::AMIGA },
            tempo,
            bpm,
            pattern_order: Vec::from_iter(pattern_order.iter().cloned()),
            instruments,
            use_amiga: (flags & 1) != 1
        })
    }

    pub fn read_xm<R: Read + Seek>(mut file: &mut R) -> SimpleResult<SongData> {
        dbg!("read_xm");
        dbg!("seek");
        file.seek(SeekFrom::Start(0));

        dbg!("len");
        let file_len = match file.stream_len() {
            Ok(m) => {m}
            Err(_) => {return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Can't read file metadata")));}
        };

        // println!("file length: {}", file_len);
        if file_len < 60 {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "File is too small!")));
        }

        dbg!("header");
        let song_data = read_xm_header(&mut file);

        song_data
    }
}
