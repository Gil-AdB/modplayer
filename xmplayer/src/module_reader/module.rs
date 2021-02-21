pub(crate) mod module {
    use crate::module_reader::{SongData, Patterns, Row, SongType, FrequencyType};
    use std::io::{Read, Seek, SeekFrom};
    use crate::io_helpers::{read_string, read_bytes};
    use std::iter::FromIterator;
    use crate::pattern::Pattern;
    use crate::instrument::{Instrument, Sample, LoopType};
    use crate::io_helpers as fio;
    use crate::channel_state::channel_state::{clamp};
    use crate::tables::AMIGA_PERIOD;
    use simple_error::{SimpleError, SimpleResult};
    use std::io;

    pub fn read_mod<R: Read + Seek>(mut file: &mut R) -> SimpleResult<SongData> {
        if let Err(res) = file.seek(SeekFrom::Start(0)) {return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Can't seek"))); }

        let file_len = match file.stream_len() {
            Ok(m) => {m}
            Err(_) => {return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Can't read file metadata")));}
        };

        // println!("file length: {}", file_len);
        if file_len < 1084 {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "File is too small!")));
        }

        let song_data = read_mod_header(&mut file);

        song_data
    }

    fn read_mod_header<R: Read + Seek>(file: &mut R) -> SimpleResult<SongData>
    {
        let num_channels;

        match file.seek(SeekFrom::Start(1080)) {
            Ok(..) => {}
            Err(e) => return Err(SimpleError::from(e))
        }

        let id = read_bytes(file, 4);

        if id == "M.K.".as_bytes() {
            num_channels = 4;
        } else if id == "6CHN".as_bytes() {
            num_channels = 6;
        } else if id == "8CHN".as_bytes() {
            num_channels = 8;
        } else if id[2] == 'C' as u8 && id[3] == 'H' as u8 {
            num_channels = String::from_utf8(id[0..2].to_vec()).unwrap().parse().unwrap();
            if num_channels < 10 || num_channels > 32 {
                return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Unknown mod format")));
            }
        } else {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Not a mod file")));
        }

        match file.seek(SeekFrom::Start(0)) {
            Ok(_) => {}
            Err(e) => return Err(SimpleError::from(e))
        }

        let name = read_string(file, 20);

        let mut instruments = read_instruments(file);

        let song_length = fio::read_u8(file);
        dbg!(song_length);

        let restart_position = fio::read_u8(file); // unused
        dbg!(restart_position);

        let mut pattern_order = fio::read_bytes(file, 128);
        let pattern_count = pattern_order.iter().cloned().max().unwrap() + 1;
        dbg!(pattern_count);

        let id = fio::read_string(file, 4);
        dbg!(&id);

        let mut patterns = read_patterns(file, pattern_count as usize, num_channels as usize);

        read_sample_data(file, &mut instruments);

        // fix empty patterns at end
        for idx in 0..pattern_order.len() {
            if pattern_order[idx] >= patterns.len() as u8 {
                pattern_order[idx] = patterns.len() as u8;
            }
        }

        patterns.push(Patterns {
            rows: vec![Row {
                channels: vec![Pattern {
                    note: 0,
                    instrument: 0,
                    volume: 0,
                    effect: 0,
                    effect_param: 0
                }; num_channels as usize]
            }; 64]
        });


        Ok(SongData {
            id: id.trim().to_string(),
            name: name.trim().to_string(),
            song_type: SongType::MOD,
            tracker_name: "Unknown".to_string(),
            song_length: song_length as u16,
            restart_position: restart_position as u16,
            channel_count: num_channels,
            patterns,
            instrument_count: instruments.len() as u16,
            frequency_type: FrequencyType::AMIGA,
            tempo: 6,
            bpm: 125,
            pattern_order: Vec::from_iter(pattern_order.iter().cloned()),
            instruments,
            use_amiga: true
        })
    }

    fn read_sample_data<R: Read + Seek>(f: &mut R, instruments: &mut Vec<Instrument>) {
        for i in 1..instruments.len() {
            instruments[i].samples[0].read_non_packed_data(f);
        }
    }

    fn read_patterns<R: Read>(file: &mut R, pattern_count: usize, channel_count: usize) -> Vec<Patterns> {
        let mut patterns: Vec<Patterns> = vec![];
        patterns.reserve_exact(pattern_count as usize);

        for _pattern_idx in 0..pattern_count {
            let row_count = 64;

            let mut rows: Vec<Row> = vec![];
            rows.reserve_exact(row_count as usize);

            for _row_idx in 0..row_count {
                let mut channels: Vec<Pattern> = vec![];
                channels.reserve_exact(channel_count);

                for _channel_idx in 0..channel_count {
                    let data = fio::read_u32_be(file);
                    let sample = (((data & 0xF0000000) >> 24) | ((data & 0xF000) >> 12)) as u8;
                    let period = ((data >> 16) & 0x0FFF) as u16;
                    let mut effect = ((data & 0xF00) >> 8) as u8;
                    let mut effect_param = (data & 0xFF) as u8;

                    let mut note = 0u8;
                    for i in 0..8 * 12usize {
                        if period >= AMIGA_PERIOD[i] {
                            note = (i + 1) as u8;
                            break;
                        }
                    }

                    let e = fix_effects(effect, effect_param);
                    effect = e.0;
                    effect_param = e.1;

                    channels.push(
                        Pattern {
                            note,
                            instrument: sample,
                            volume: 0,
                            effect,
                            effect_param
                        }
                    );
                }
                rows.push(Row { channels });
            }
            patterns.push(Patterns { rows })
        }

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
        } else if effect == 0x2 {       // No porta memory
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
        let length = fio::read_u16_be(file) * 2;
        let ft = fio::read_u8(file) & 0xf;
        let sign = ((ft >> 3) * 0xF0) as i8;
        let mut finetune= ft as i8 | sign;
        finetune <<= 1;
        finetune *= 8;

        let ft_ft = 8 * ((2 * (((ft as i16) & 0xF) ^ 8)) - 16) as i8;

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

    fn read_instruments<R: Read + Seek>(file: &mut R) -> Vec<Instrument> {
        let mut instruments: Vec<Instrument> = vec![];
        let instrument_count = 31;

        // Instruments are one based, go figure. We'll add an empty instrument as sample 0.
        instruments.reserve_exact(instrument_count + 1 as usize);

        instruments.push(Instrument::new());

        for instrument_idx in 1..instrument_count +1 {
            let mut instrument = Instrument::new();
            let sample = read_sample(file);
            instrument.name = sample.name.clone();
            instrument.idx = instrument_idx as u8;
            instrument.samples = vec![sample];
            instruments.push(instrument);
        }
        instruments
    }
}