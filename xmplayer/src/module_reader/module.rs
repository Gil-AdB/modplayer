    use crate::module_reader::{SongData, Patterns, Row, SongType, FrequencyType};
    use std::io::{Read, Seek, SeekFrom};
    use binary_reader_io::BinaryReader;
    use std::iter::FromIterator;
    use crate::pattern::Pattern;
    use crate::instrument::{Instrument, Sample, LoopType};
    use crate::channel_state::channel_state::{clamp};
    use crate::tables::AMIGA_PERIOD;
    use crate::{SimpleError, SimpleResult};

    pub fn read_mod<R: Read + Seek>(mut file: &mut R) -> SimpleResult<SongData> {
        file.seek(SeekFrom::Start(0))?;

        let file_len = file.seek(SeekFrom::End(0))?;
        file.seek(SeekFrom::Start(0))?;

        if file_len < 1084 {
            return Err(SimpleError::new("File is too small!"));
        }

        read_mod_header(&mut file)
    }

    fn read_mod_header<R: Read + Seek>(file: &mut R) -> SimpleResult<SongData>
    {
        file.seek(SeekFrom::Start(1080))?;

        let id = file.read_bytes(4)?;

        let num_channels = if id == "M.K.".as_bytes() {
            4
        } else if id == "6CHN".as_bytes() {
            6
        } else if id == "8CHN".as_bytes() {
            8
        } else if id[2] == 'C' as u8 && id[3] == 'H' as u8 {
            let n: usize = String::from_utf8(id[0..2].to_vec()).map_err(|_e| SimpleError::new("Invalid channel count"))?.parse().map_err(|_e| SimpleError::new("Invalid channel count"))?;
            if n < 10 || n > 32 {
                return Err(SimpleError::new("Unknown mod format"));
            }
            n
        } else {
            return Err(SimpleError::new("Not a mod file"));
        };

        file.seek(SeekFrom::Start(0))?;

        let name = file.read_string(20);

        let mut instruments = read_instruments(file)?;

        let song_length = file.read_u8()?;

        let _restart_position = file.read_u8()?; // unused

        let mut pattern_order = file.read_bytes(128)?;
        let pattern_count = pattern_order.iter().cloned().max().ok_or(SimpleError::new("Unknown pattern count"))? + 1;

        let id_str = file.read_string(4);

        let mut patterns = read_patterns(file, pattern_count as usize, num_channels as usize)?;

        read_sample_data(file, &mut instruments)?;

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
            id: id_str.trim().to_string(),
            name: name.trim().to_string(),
            song_type: SongType::MOD,
            tracker_name: "Unknown".to_string(),
            song_length: song_length as u16,
            restart_position: _restart_position as u16,
            channel_count: num_channels as u16,
            patterns,
            instrument_count: instruments.len() as u16,
            frequency_type: FrequencyType::AMIGA,
            tempo: 6,
            bpm: 125,
            pattern_order: Vec::from_iter(pattern_order.iter().cloned()),
            instruments,
            use_amiga: true,
            song_message: "".to_string(),
            initial_channel_volume: [64; 64],
            initial_channel_panning: [32; 64],
            global_volume:           64,
            master_volume:           128,
            mixing_volume:           128,
            old_effects: false,
            compatible_g: false,
        })
    }

    fn read_sample_data<R: Read + Seek>(f: &mut R, instruments: &mut Vec<Instrument>) -> SimpleResult<()> {
        for i in 1..instruments.len() {
            instruments[i].samples[0].read_non_packed_data(f)?;
        }
        Ok(())
    }

    fn read_patterns<R: Read>(file: &mut R, pattern_count: usize, channel_count: usize) -> SimpleResult<Vec<Patterns>> {
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
                    let data = file.read_u32_be()?;
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

        Ok(patterns)
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

    fn read_sample<R: Read>(file: &mut R) -> SimpleResult<Sample> {
        let name = file.read_string(22);
        let length = file.read_u16_be()? * 2;
        let ft = file.read_u8()? & 0xf;
        let sign = ((ft >> 3) * 0xF0) as i8;
        let mut finetune= ft as i8 | sign;
        finetune <<= 1;
        finetune *= 8;

        let ft_ft = 8 * ((2 * (((ft as i16) & 0xF) ^ 8)) - 16) as i8;

        if ft_ft != finetune {
            return Err(SimpleError::new("Bug in finetune calculation"));
        }

        let volume = file.read_u8()?;
        let mut loop_start = file.read_u16_be()? * 2;
        let mut loop_len = file.read_u16_be()? * 2;

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

        Ok(Sample {
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
            global_volume: 64,
            surround: false,
            is_ping_pong: false,
            original_loop_end: 0,
            data: vec![],
        })
    }

    fn read_instruments<R: Read + Seek>(file: &mut R) -> SimpleResult<Vec<Instrument>> {
        let mut instruments: Vec<Instrument> = vec![];
        let instrument_count = 31;

        // Instruments are one based, go figure. We'll add an empty instrument as sample 0.
        instruments.reserve_exact(instrument_count + 1 as usize);

        instruments.push(Instrument::new());

        for instrument_idx in 1..instrument_count +1 {
            let mut instrument = Instrument::new();
            let sample = read_sample(file)?;
            instrument.name = sample.name.clone();
            instrument.idx = instrument_idx as u8;
            instrument.samples = vec![sample];
            instrument.sample_indexes = (0..120).map(|i| (i + 1, 0)).collect();
            instruments.push(instrument);
        }
        Ok(instruments)
    }