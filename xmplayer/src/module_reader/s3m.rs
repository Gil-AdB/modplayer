pub(crate) mod s3m {
    use crate::module_reader::{SongData, Patterns, Row, SongType, FrequencyType};
    use std::fs::File;
    use std::io::{BufReader, Read, Seek, SeekFrom, Error, ErrorKind};
    use crate::io_helpers::{read_string, read_bytes, read_u8, read_u16, read_u16_vec, BinaryReader, read_u32, read_u24};
    use std::iter::FromIterator;
    use crate::pattern::Pattern;
    use crate::instrument::{Instrument, Sample, LoopType};
    use crate::envelope::Envelope;
    use crate::io_helpers as fio;
    use crate::channel_state::channel_state::{clamp};
    use crate::song::TableType::AmigaFrequency;
    use crate::song::PlaybackCmd::AmigaTable;
    use crate::tables::AMIGA_PERIOD;
    use simple_error::{SimpleError, SimpleResult};
    use std::io;
    use byteorder::ReadBytesExt;
    use std::cmp::max;

    pub fn read_s3m(path: &str) -> SimpleResult<SongData> {
        let f = File::open(path).expect("failed to open the file");
        let file_len = f.metadata().expect("Can't read file metadata").len();
        let mut file = BufReader::new(f);


        // println!("file length: {}", file_len);
        if file_len < 1084 {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "File is too small!")));
        }

        let song_data = read_s3m_header(&mut file);

        song_data
    }

    fn read_s3m_header<R: Read + Seek>(mut file: &mut R) -> SimpleResult<SongData>
    {
        let mut num_channels = 0;

        file.seek(SeekFrom::Start(44));

        let id = read_bytes(file, 4);

        if id != "SCRM".as_bytes() {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Unknown s3m format - signature"))); // Simple how exactly?
        }

        file.seek(SeekFrom::Start(0));

        let name = read_string(file, 28);
        dbg!(name);
        let sig = read_u8(file);
        dbg!(sig);
        let file_type = read_u8(file);
        dbg!(file_type);
        if file_type != 16 {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Unknown s3m format"))); // Simple how exactly?
        }

        let _ = read_u16(file);

        let song_length = read_u16(file);
        dbg!(song_length);

        if song_length > 256 {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Unknown s3m format - song length"))); // Simple how exactly?
        }

        let instrument_count = read_u16(file);
        dbg!(instrument_count);

        if instrument_count > 128 {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Unknown s3m format - instruments"))); // Simple how exactly?
        }

        let mut pattern_count = read_u16(file);
        dbg!(pattern_count);

        if pattern_count > 256 {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Unknown s3m format - patterns"))); // Simple how exactly?
        }

        let flags = read_u16(file);
        dbg!(flags);

        let cwtv = read_u16(file);
        dbg!(cwtv);

        let signed_samples = read_u16(file);
        dbg!(signed_samples);

        let signature = read_string(file, 4);
        dbg!(&signature);

        if signature != "SCRM" {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Unknown s3m format - signature"))); // Simple how exactly?
        }

        let global_volume = read_u8(file);
        dbg!(global_volume);

        let speed = read_u8(file);
        dbg!(speed);

        let bpm = read_u8(file);
        dbg!(bpm);

        let master_volume = read_u8(file);
        dbg!(master_volume);

        let _ = read_u8(file);

        let default_panning = read_u8(file);
        dbg!(default_panning);

        file.seek(SeekFrom::Current(10));

        let channel_data = read_bytes(file, 32);
        let mut channel_map = [255u8; 32];

        for i in 0..channel_data.len() {
            if channel_data[i] < 16u8 {
                channel_map[i] = num_channels;
                num_channels += 1;
            }
        }

        let mut pattern_order = read_bytes(file, song_length as usize);
        truncate_patterns(pattern_count, &mut pattern_order);
        dbg!(pattern_order);

        let instrument_ptrs = read_u16_vec(file, instrument_count as usize);
        let pattern_ptrs = read_u16_vec(file, pattern_count as usize);

        // Now we should read the panning positions. Or not. Whatever. Maybe some other time.
        let instruments = read_instruments(file, &instrument_ptrs);
        read_patterns(file, &instrument_ptrs, 255);


        return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Not Implemented"))); // Simple how exactly?


        //
        //
        //
        //
        // file.seek(SeekFrom::Start(1080));
        //
        // let id = read_bytes(file, 4);
        //
        // if id == "M.K.".as_bytes() {
        //     num_channels = 4;
        // } else if id == "6CHN".as_bytes() {
        //     num_channels = 6;
        // } else if id == "8CHN".as_bytes() {
        //     num_channels = 8;
        // } else if id[2] == 'C' as u8 && id[3] == 'H' as u8 {
        //     num_channels = String::from_utf8(id[0..2].to_vec()).unwrap().parse().unwrap();
        //     if num_channels < 10 || num_channels > 32 {
        //         return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Unknown mod format")));
        //     }
        // } else {
        //     return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Not a mod file")));
        // }
        //
        // file.seek(SeekFrom::Start(0));
        //
        // let name = read_string(file, 20);
        //
        // let mut instruments = read_instruments(file);
        //
        // let song_length = fio::read_u8(file);
        // dbg!(song_length);
        //
        // let restart_position = fio::read_u8(file); // unused
        // dbg!(restart_position);
        //
        // let mut pattern_order = fio::read_bytes(file, 128);
        // let pattern_count = *pattern_order.iter().max().unwrap() + 1;
        // dbg!(pattern_count);
        //
        // let id = fio::read_string(file, 4);
        // dbg!(&id);
        //
        // let mut patterns = read_patterns(file, pattern_count as usize, num_channels as usize);
        //
        // read_sample_data(file, &mut instruments);
        //
        // // fix empty patterns at end
        // for idx in 0..pattern_order.len() {
        //     if pattern_order[idx] >= patterns.len() as u8 {
        //         pattern_order[idx] = patterns.len() as u8;
        //     }
        // }
        //
        // patterns.push(Patterns {
        //     rows: vec![Row {
        //         channels: vec![Pattern {
        //             note: 0,
        //             instrument: 0,
        //             volume: 0,
        //             effect: 0,
        //             effect_param: 0
        //         }; num_channels as usize]
        //     }; 64]
        // });
        //
        //
        // Ok(SongData {
        //     id: id.trim().to_string(),
        //     name: name.trim().to_string(),
        //     song_type: SongType::MOD,
        //     tracker_name: "Unknown".to_string(),
        //     song_length: song_length as u16,
        //     restart_position: restart_position as u16,
        //     channel_count: num_channels,
        //     patterns,
        //     instrument_count: instruments.len() as u16,
        //     frequency_type: FrequencyType::AMIGA,
        //     tempo: 6,
        //     bpm: 125,
        //     pattern_order: Vec::from_iter(pattern_order.iter().cloned()),
        //     instruments,
        //     use_amiga: true
        // })
    }

    fn truncate_patterns(pattern_count: u16, pattern_order: &mut Vec<u8>) {
        let mut write_pos = 0;
        for i in 0..pattern_count as usize {
            if pattern_order[i] < 254 {
                pattern_order[write_pos] = pattern_order[i];
                write_pos += 1;
            } else if pattern_order[i] == 255 {
                break;
            }
        }

        pattern_order.truncate(write_pos as usize);
    }

    fn read_sample_data<R: Read + Seek>(f: &mut R, instruments: &mut Vec<Instrument>) {
        for i in 1..instruments.len() {
            instruments[i].samples[0].read_non_packed_data(f);
        }
    }

    fn read_patterns<R: Read>(file: &mut R, pattern_ptrs: &Vec<u16>, channel_count: usize) -> Vec<Patterns> {
        let mut patterns: Vec<Patterns> = vec![];
        // patterns.reserve_exact(pattern_count as usize);
        //
        // for _pattern_idx in 0..pattern_count {
        //     let ROW_COUNT = 64;
        //
        //     let mut rows: Vec<Row> = vec![];
        //     rows.reserve_exact(ROW_COUNT as usize);
        //
        //     for _row_idx in 0..ROW_COUNT {
        //         let mut channels: Vec<Pattern> = vec![];
        //         channels.reserve_exact(channel_count);
        //
        //         for _channel_idx in 0..channel_count {
        //             let data = fio::read_u32_be(file);
        //             let sample = (((data & 0xF0000000) >> 24) | ((data & 0xF000) >> 12)) as u8;
        //             let period = ((data >> 16) & 0x0FFF) as u16;
        //             let mut effect = ((data & 0xF00) >> 8) as u8;
        //             let mut effect_param = (data & 0xFF) as u8;
        //
        //             let mut note = 0u8;
        //             for i in 0..8 * 12usize {
        //                 if period >= AMIGA_PERIOD[i] {
        //                     note = (i + 1) as u8;
        //                     break;
        //                 }
        //             }
        //
        //             let e = fix_effects(effect, effect_param);
        //             effect = e.0;
        //             effect_param = e.1;
        //
        //             channels.push(
        //                 Pattern {
        //                     note,
        //                     instrument: sample,
        //                     volume: 0,
        //                     effect,
        //                     effect_param
        //                 }
        //             );
        //         }
        //         rows.push(Row { channels });
        //     }
        //     patterns.push(Patterns { rows })
        // }

        patterns
    }

    fn fix_effects(e: u8, p: u8) -> (u8, u8) {
        let mut effect = e;
        let mut effect_param = p;

        if effect == 0xC {              // Clamp Volume to 64
            if effect_param > 64 {
                effect_param = 64;
            }
        } else if effect == 0x1 {       // No porta memory
            if effect_param == 0 {
                effect = 0;
            }
        } else if effect == 0x2 {       // No porta memorty
            if effect_param == 0 {
                effect = 0;
            }
        } else if effect == 0x5 {       // No volume slide memory
            if effect_param == 0 {
                effect = 0x3;
            }
        } else if effect == 0x6 {       // No volume slide memory
            if effect_param == 0 {
                effect = 0x4;
            }
        } else if effect == 0xA {       // No volume slide memory
            if effect_param == 0 {
                effect = 0;
            }
        } else if effect == 0xE {       // No porta & volume slide memory
            // check if certain E commands are empty
            if effect_param == 0x10 || effect_param == 0x20 || effect_param == 0xA0 || effect_param == 0xB0
            {
                effect = 0;
                effect_param = 0;
            }
        }
        return (effect, effect_param)
    }

    fn read_sample<R: Read>(file: &mut R) -> Sample {
        let name = fio::read_string(file, 22);
        let mut length = fio::read_u16_be(file) * 2;
        let ft = fio::read_u8(file) & 0xf;
        let sign = ((ft >> 3) * 0xF0) as i8;
        let mut finetune= ft as i8 | sign;
        finetune <<= 1;
        finetune *= 8;

        let ft_ft = 8 * ((2 * ((ft & 0xF) ^ 8)) - 16) as i8;

        if ft_ft != finetune {
            panic!("bugbug");
        }

        let volume = fio::read_u8(file);
        let mut loop_start = fio::read_u16_be(file) * 2;
        let mut loop_len = fio::read_u16_be(file) * 2;

        if loop_len < 2 {
            loop_len = 2;
        }
        // fix overflown loop
        if loop_start+loop_len > length {
            if loop_start >= length {
                loop_start = 0;
                loop_len = 0;
            } else {
                loop_len = length - loop_start;
            }
        }

        if loop_len <= 2 {
            loop_start = 0;
            loop_len = 0;
        }

        Sample {
            length: length as u32,
            loop_start: loop_start as u32,
            loop_end: (loop_start + loop_len) as u32,
            loop_len: loop_len as u32,
            volume: clamp(volume, 0, 64),
            finetune,
            loop_type: if loop_len > 2 {LoopType::ForwardLoop} else {LoopType::NoLoop},
            bitness: 8,
            panning: 128,
            relative_note: 0,
            name,
            data: vec![],
        }
    }

    fn read_instruments<R: Read + Seek>(file: &mut R, instrument_ptrs: &Vec<u16>) -> Vec<Instrument> {
        let mut instruments: Vec<Instrument> = vec![];
        let INSTRUMENT_COUNT = instrument_ptrs.len();

        // Instruments are one based, go figure. We'll add an empty instrument as sample 0.
        instruments.reserve_exact(INSTRUMENT_COUNT + 1 as usize);

        instruments.push(Instrument::new());

        for instrument_ptr in instrument_ptrs.iter().cloned() {
            let mut instrument = Instrument::new();
            file.seek(SeekFrom::Start((instrument_ptr as u64)  * 16));
            let type_ = read_u8(file);
            dbg!(type_);
            let dos_name = read_string(file,12);
            dbg!(dos_name);
            let sample_ptr = read_u24(file);
            dbg!(sample_ptr);
            let sample_len = read_u32(file) & 0xFFFF;
            dbg!(sample_len);
            let sample_loop_start = read_u32(file) & 0xFFFF;
            dbg!(sample_loop_start);
            let sample_loop_end = read_u32(file) & 0xFFFF;
            dbg!(sample_loop_end);
            let sample_volume = read_u8(file);
            dbg!(sample_volume);
            let _ = read_u8(file);
            let sample_packing = read_u8(file);
            dbg!(sample_packing);
            if sample_packing != 0 {
                panic!("Unknown file format");
            }
            let sample_flags = read_u8(file);
            dbg!(sample_flags);
            let c2spd = read_u32(file) & 0xFFFF;
            dbg!(c2spd);
            let _ = read_bytes(file, 12);
            let sample_name = read_string(file, 28);
            dbg!(sample_name);
            let sample_sig = read_string(file, 4);
            if sample_sig != "SCRS" {
                panic!("unknown sample format!");
            }


            // let sample = read_sample(file);
            // instrument.name = sample.name.clone();
            // instrument.idx = instrument_idx as u8;
            // instrument.samples = vec![sample];
            // instruments.push(instrument);
        }
        instruments
    }
}