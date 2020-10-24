pub(crate) mod s3m {
    use crate::module_reader::{SongData, Patterns, Row, SongType, FrequencyType};
    use std::fs::File;
    use std::io::{BufReader, Read, Seek, SeekFrom};
    use crate::io_helpers::{read_string, read_bytes, read_u8, BinaryReader, read_u32, read_u24};
    use std::iter::FromIterator;
    use crate::pattern::Pattern;
    use crate::instrument::{Instrument, Sample, LoopType};
    use simple_error::{SimpleError, SimpleResult};
    use std::io;
    use crate::module_reader;

    pub fn read_s3m(path: &str) -> SimpleResult<SongData> {
        let f = match File::open(path) {
            Ok(f) => {f}
            Err(_) => {return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "failed to open the file")));}
        };

        let file_len = match f.metadata(){
            Ok(m) => {m.len()}
            Err(_) => {return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Can't read file metadata")));}
        };

        let mut file = BufReader::new(f);

        // println!("file length: {}", file_len);
        if file_len < 1084 {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "File is too small!")));
        }

        let song_data = read_s3m_header(&mut file);

        song_data
    }

    fn read_s3m_header<R: Read + Seek>(file: &mut R) -> SimpleResult<SongData>
    {
        let mut num_channels = 0;

        file.seek(SeekFrom::Start(44)).unwrap();

        let id = read_bytes(file, 4);

        if id != "SCRM".as_bytes() {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Unknown s3m format - signature"))); // Simple how exactly?
        }

        file.seek(SeekFrom::Start(0)).unwrap();

        let name = file.read_string(28);
        dbg!(&name);
        let sig = file.read_u8();
        dbg!(sig);
        let file_type = file.read_u8();
        dbg!(file_type);
        if file_type != 16 {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Unknown s3m format"))); // Simple how exactly?
        }

        let _ = file.read_u16();

        let song_length = file.read_u16();
        dbg!(song_length);

        if song_length > 256 {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Unknown s3m format - song length"))); // Simple how exactly?
        }

        let instrument_count = file.read_u16();

        if instrument_count > 128 {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Unknown s3m format - instruments"))); // Simple how exactly?
        }

        let pattern_count = file.read_u16();

        if pattern_count > 256 {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Unknown s3m format - patterns"))); // Simple how exactly?
        }

        let _flags = file.read_u16();

        let _cwtv = file.read_u16();

        let _signed_samples = file.read_u16();

        let signature = file.read_string(4);

        if signature != "SCRM" {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Unknown s3m format - signature"))); // Simple how exactly?
        }

        let _global_volume = file.read_u8();

        let speed = file.read_u8();

        let bpm = file.read_u8();

        let _master_volume = file.read_u8();

        let _ = file.read_u8();

        let _default_panning = file.read_u8();

        file.seek(SeekFrom::Current(10)).unwrap();

        let channel_data = file.read_bytes(32);
        let mut channel_map = [255u8; 32];

        for i in 0..channel_data.len() {
            if channel_data[i] < 16u8 {
                channel_map[i] = num_channels;
                num_channels += 1;
            }
        }

        let mut pattern_order = file.read_bytes(song_length as usize);
        truncate_patterns(&mut pattern_order);

        let instrument_ptrs = file.read_u16_vec(instrument_count as usize);
        let pattern_ptrs = file.read_u16_vec(pattern_count as usize);

        // Now we should read the panning positions. Or not. Whatever. Maybe some other time.
        let instruments = read_instruments(file, &instrument_ptrs)?;
        let mut patterns = read_patterns(file, &pattern_ptrs, num_channels as usize, &channel_map);



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
            id: String::from_utf8_lossy(id.as_ref()).trim().to_string(),
            name: name.trim().to_string(),
            song_type: SongType::S3M,
            tracker_name: "Unknown".to_string(),
            song_length: pattern_order.len() as u16,
            restart_position: 0u16,
            channel_count: num_channels as u16,
            patterns,
            instrument_count: instruments.len() as u16,
            frequency_type: FrequencyType::AMIGA,
            tempo: speed as u16,
            bpm: bpm as u16,
            pattern_order: Vec::from_iter(pattern_order.iter().cloned()),
            instruments,
            use_amiga: true
        })
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

    fn read_patterns<R: Read + Seek>(file: &mut R, pattern_ptrs: &Vec<u16>, channel_count: usize, channel_map: &[u8; 32]) -> Vec<Patterns> {
        let pattern_count = pattern_ptrs.len();
        let mut patterns: Vec<Patterns> = vec![];
        patterns.reserve_exact(pattern_count);
        let row_count = 64;

        for pattern_ptr in pattern_ptrs.iter().cloned() {
            if pattern_ptr == 0 {continue;}
            file.seek(SeekFrom::Start((pattern_ptr as u64)  * 16)).unwrap();

            let mut pattern = Patterns::new(row_count, channel_count);

            let _size = file.read_u16();

            let mut last_effect_param       = [0u8; 32];
            let mut last_effect             = [0u8; 32];
            let mut last_vibrato_param      = [0u8; 32];
            let mut last_instrument = [0u8; 32];

            for row in pattern.rows.iter_mut() {
                let channels = &mut row.channels;

                loop {
                    let pattern_data = file.read_u8();
                    if pattern_data == 0 { break; }

                    let channel_num = pattern_data & 31;
                    let channel_id = channel_map[channel_num as usize] as usize;

                    let mut note = 0u8;
                    let mut instrument = 0u8;
                    let mut volume = 0u8;
                    let mut effect = 0u8;
                    let mut effect_param = 0u8;

                    if pattern_data & 32 == 32 {
                        note = file.read_u8();
                        instrument = file.read_u8();

                        if note == 255 {
                            note = 0;
                        } else if note == 254 {
                            note = 97;
                        } else {
                            note = 1 + (note >> 4) * 12 + (note & 0xF);
                            if note > 96 {note = 0;}
                        }
                    }

                    if pattern_data & 64 == 64 {
                        volume = file.read_u8();
                        if volume <= 64 {volume += 0x10} else { volume = 0;}
                    }

                    if pattern_data & 128 == 128 {
                        effect = file.read_u8();
                        effect_param = file.read_u8();
                    }

                    if channel_num >= channel_count as u8 { continue; }
                    let channel = &mut channels[channel_id];

                    channel.note = note;
                    channel.instrument = instrument;
                    channel.volume = volume;
                    channel.effect = effect;
                    channel.effect_param = effect_param;
                    
                    if pattern_data & 128 == 128 {
                        fix_effects(
                            channel,
                            &mut last_effect[channel_id],
                            &mut last_effect_param[channel_id],
                            &mut last_vibrato_param[channel_id],
                            &mut last_instrument[channel_id]
                        );
                    }
                }
            }
            patterns.push(pattern)
        }

        patterns
    }

    fn fix_effects(pattern : &mut Pattern, last_effect: &mut u8, last_effect_param: &mut u8, last_vibrato_param: &mut u8,last_instrument: &mut u8) {
        // lifted from FT2 - effect memory handling seems somewhat wrong - it should be handled during effect processing
        //                   Fixing it needs additional work in the player code - seems like this workaround will suffice for now
        if pattern.effect_param > 0 {
            *last_effect_param = pattern.effect_param;
            if pattern.effect == 8 || pattern.effect == 21 {
                *last_vibrato_param = pattern.effect_param;
            }
        }

        if pattern.effect_param == 0 && pattern.effect != 7 {
            if pattern.effect == 8 || pattern.effect == 21 {
                pattern.effect_param = *last_vibrato_param;
            } else if (pattern.effect >= 4 && pattern.effect <= 12) || (pattern.effect >= 17 && pattern.effect <= 19) {
                pattern.effect_param = *last_effect_param;
            }

            if pattern.effect == *last_effect && pattern.effect != 10 && pattern.effect != 19 {
                let extra_fine_pitch_slides = (pattern.effect == 5 || pattern.effect == 6) && ((pattern.effect_param & 0xF0) == 0xE0);
                let fine_vol_slides = (pattern.effect == 4 || pattern.effect == 11) &&
                    ((pattern.effect_param > 0xF0) || (((pattern.effect_param & 0xF) == 0xF) && ((pattern.effect_param & 0xF0) > 0)));

                if !extra_fine_pitch_slides && !fine_vol_slides {
                    pattern.effect_param = 0;
                }
            }
        }
        if pattern.effect > 0 {
            *last_effect = pattern.effect;
        }
        
        match pattern.effect {
            1 => // A - Set speed - don't support speeds > 1F
                {
                    pattern.effect = 0xF;
                    if pattern.effect_param == 0 || pattern.effect_param > 0x1F {
                        pattern.effect = 0;
                        pattern.effect_param = 0;
                    }
                }

            2 => pattern.effect = 0xB,  // B - Pattern Jump
            3 => pattern.effect = 0xD,  // C - Volume slide
            4 => // D
                {
                    if pattern.effect_param > 0xF0 { // fine slide up
                        pattern.effect = 0xE;
                        pattern.effect_param = 0xB0 | (pattern.effect_param & 0xF);
                    } else if (pattern.effect_param & 0x0F) == 0x0F && (pattern.effect_param & 0xF0) > 0 { // fine slide down
                        pattern.effect = 0xE;
                        pattern.effect_param = 0xA0 | (pattern.effect_param >> 4);
                    } else {
                        pattern.effect = 0xA;
                        if (pattern.effect_param & 0x0F) != 0 { // on D/K (Volume slide/Vibrato + Volume slide), last nybble has first priority in ST3
                            pattern.effect_param &= 0x0F;
                        }
                    }
                }

            5 | 6 => { // E, F - porta up/down
                if (pattern.effect_param & 0xF0) >= 0xE0 {
                    // convert to fine slide
                    let new_effect = if (pattern.effect_param & 0xF0) == 0xE0 { 0x21 } else { 0xE };

                    pattern.effect_param &= 0x0F;

                    if pattern.effect == 0x05 {
                        pattern.effect_param |= 0x20;
                    } else {
                        pattern.effect_param |= 0x10;
                    }
                    pattern.effect = new_effect;

                    if pattern.effect == 0x21 && pattern.effect_param == 0 {
                        pattern.effect_param = 0;
                    }
                } else {
                    // convert to normal 1xx/2xx slide
                    pattern.effect = 7 - pattern.effect;
                }
            }

            7 => { // G - Porta to note
                pattern.effect = 0x03;

                // fix illegal slides (to new instruments)
                if pattern.instrument != 0 && pattern.instrument != *last_instrument {
                    pattern.instrument = *last_instrument;
                }
            }

            11 => { // K - Vibrato + volume slide
                if pattern.effect_param > 0xF0 { // fine slide up
                    pattern.effect = 0xE;
                    pattern.effect_param = 0xB0 | (pattern.effect_param & 0xF);

                    // if volume column is unoccupied, set to vibrato
                    if pattern.volume == 0 {
                        pattern.volume = 0xB0;
                    }
                } else if (pattern.effect_param & 0x0F) == 0x0F && (pattern.effect_param & 0xF0) > 0 { // fine slide down
                    pattern.effect = 0xE;
                    pattern.effect_param = 0xA0 | (pattern.effect_param >> 4);

                    // if volume column is unoccupied, set to vibrato
                    if pattern.volume == 0 {
                        pattern.volume = 0xB0;
                    }
                } else {
                    pattern.effect = 0x6;
                    if (pattern.effect_param & 0x0F) != 0 { // on D/K, last nybble has first priority in ST3
                        pattern.effect_param &= 0x0F;
                    }
                }
            }
            8 =>  { pattern.effect = 0x04; } // H - Vibrato
            9 =>  { pattern.effect = 0x1D; } // I - Tremor
            10 => { pattern.effect = 0x00; } // J - Arpeggio
            12 => { pattern.effect = 0x05; } // L - Porta + Volume slide
            15 => { pattern.effect = 0x09; } // O - Sample offset
            17 => { pattern.effect = 0x1B; } // Q - Retrig + Volume slide
            18 => { pattern.effect = 0x07; } // R - Tremolo
            19 => { // S - Extended commands
                pattern.effect = 0xE;
                let subcommand = pattern.get_x();
                pattern.effect_param &= 0x0F;

                match subcommand {
                    0x1 => { pattern.effect_param |= 0x30; } // Glissando
                    0x2 => { pattern.effect_param |= 0x50; } // Set finetune
                    0x3 => { pattern.effect_param |= 0x40; } // Set Vibrato Waveform
                    0x4 => { pattern.effect_param |= 0x70; } // Set Tremolo Waveform (Firelight S3M tutorial is wrong here)
                    0xB => { pattern.effect_param |= 0x60; } // Channel pan position. ignore S8x because it's not compatible with FT2 panning
                    0xC => {
                        pattern.effect_param |= 0xC0;
                        if pattern.effect_param == 0xC0 {
                            // EC0 does nothing in ST3 but cuts voice in FT2, remove effect
                            pattern.effect = 0;
                            pattern.effect_param = 0;
                        }
                    }
                    0xD => { // Note Delay
                        pattern.effect_param |= 0xD0;
                        if pattern.note == 0 || pattern.note == 97 {
                            // EDx without a note does nothing in ST3 but retrigs in FT2, remove effect
                            pattern.effect = 0;
                            pattern.effect_param = 0;
                        } else if pattern.effect_param == 0xD0 {
                            // ED0 prevents note/smp/vol from updating in ST3, remove everything
                            pattern.note = 0;
                            pattern.instrument = 0;
                            pattern.volume = 0;
                            pattern.effect = 0;
                            pattern.effect_param = 0;
                        }
                    }
                    0xE => { pattern.effect_param |= 0xE0; } // Pattern Delay
                    0xF => { pattern.effect_param |= 0xF0; } // Funk Repeat - not supported anyway...
                    _ => {
                        pattern.effect = 0;
                        pattern.effect_param = 0;
                    }
                }
            }

            20 => { // T - Set Tempo/BPM
                pattern.effect = 0x0F;
                if pattern.effect_param < 0x21 {// Txx with a value lower than 33 (0x21) does nothing in ST3, remove effect
                    pattern.effect = 0;
                    pattern.effect_param = 0;
                }
            }
            22 => { // V - Set Global Volume
                pattern.effect = 0x10;
                if pattern.effect_param > 0x40 {
                    // Vxx > 0x40 does nothing in ST3
                    pattern.effect = 0;
                    pattern.effect_param = 0;
                }
            }

            _ => {
                pattern.effect = 0;
                pattern.effect_param = 0;
            }
        }

        if pattern.instrument != 0 && pattern.effect != 0x3 {
            *last_instrument = pattern.instrument;
        }

        if pattern.effect > 35 {
            pattern.effect = 0;
            pattern.effect_param = 0;
        }
    }

    fn read_instruments<R: Read + Seek>(file: &mut R, instrument_ptrs: &Vec<u16>) -> SimpleResult<Vec<Instrument>> {
        let mut instruments: Vec<Instrument> = vec![];
        let instrument_count = instrument_ptrs.len();

        // Instruments are one based, go figure. We'll add an empty instrument as sample 0.
        instruments.reserve_exact(instrument_count + 1 as usize);

        instruments.push(Instrument::new());

        for (instrument_idx, instrument_ptr) in instrument_ptrs.iter().cloned().enumerate() {
            let mut instrument = Instrument::new();
            file.seek(SeekFrom::Start((instrument_ptr as u64)  * 16)).unwrap();
            let _type_ = read_u8(file);
            let _dos_name = read_string(file,12);
            let sample_ptr = read_u24(file);
            let sample_len = read_u32(file) & 0xFFFF;
            let sample_loop_start = read_u32(file) & 0xFFFF;
            let sample_loop_end = read_u32(file) & 0xFFFF;
            let sample_volume = read_u8(file);
            let _ = read_u8(file);
            let sample_packing = read_u8(file);
            if sample_packing != 0 {
                return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other,"Unknown file format")));
            }
            let sample_flags = read_u8(file);
            let c2spd = read_u32(file) & 0xFFFF;
            let _ = read_bytes(file, 12);
            let sample_name = read_string(file, 28);
            let sample_sig = read_string(file, 4);
            if sample_sig != "SCRS" {
//                panic!("unknown sample format!");
            }

            let (finetune, relative_note) = module_reader::c2spd_to_finetune_relnote(c2spd);

            let mut sample = Sample{
                length: sample_len,
                loop_start: sample_loop_start,
                loop_end: sample_loop_end,
                loop_len: sample_loop_end - sample_loop_start,
                volume: sample_volume,
                finetune,
                loop_type: if sample_flags & 1 == 1 {LoopType::ForwardLoop} else {LoopType::NoLoop},
                bitness: 8,
                panning: 128,
                relative_note,
                name: sample_name.clone().to_string(),
                data: vec![]
            };
            sample.read_s3m_sample_data(file, sample_ptr);
            instrument.name = sample.name.clone();
            instrument.idx = instrument_idx as u8;
            instrument.samples = vec![sample];
            instruments.push(instrument);
        }
        Ok(instruments)
    }
}