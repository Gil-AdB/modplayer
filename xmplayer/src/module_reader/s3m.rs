    use crate::module_reader::{SongData, Patterns, Row, SongType, FrequencyType};
    use std::io::{Read, Seek, SeekFrom};
    use binary_reader_io::BinaryReader;
    use std::iter::FromIterator;
    use crate::pattern::Pattern;
    use crate::instrument::{Instrument, Sample, LoopType};
    use crate::{SimpleError, SimpleResult};
    use std::io;
    use crate::module_reader;

    pub(crate) trait S3MBinaryReader: BinaryReader {
        fn read_u24_s3m(&mut self) -> io::Result<u32>;
    }

    impl<R: BinaryReader> S3MBinaryReader for R {
        fn read_u24_s3m(&mut self) -> io::Result<u32> {
            let mut buf = [0u8; 3];
            self.read_exact(&mut buf)?;
            // S3M mixed endianness: High byte followed by Little-Endian word
            Ok(((buf[0] as u32) << 16) | ((buf[2] as u32) << 8) | (buf[1] as u32))
        }
    }

    pub fn read_s3m<R: Read + Seek>(mut file: &mut R) -> SimpleResult<SongData> {
        file.seek(SeekFrom::Start(0))?;

        let file_len = file.seek(SeekFrom::End(0))?;
        file.seek(SeekFrom::Start(0))?;

        if file_len < 1084 {
            return Err(SimpleError::new("File is too small!"));
        }

        read_s3m_header(&mut file)
    }

    fn read_s3m_header<R: Read + Seek>(file: &mut R) -> SimpleResult<SongData>
    {
        let mut num_channels = 0;

        file.seek(SeekFrom::Start(44))?;

        let id = file.read_bytes(4)?;

        if id != "SCRM".as_bytes() {
            return Err(SimpleError::new("Unknown s3m format - signature"));
        }

        file.seek(SeekFrom::Start(0))?;

        let name = file.read_string(28);
        let _sig = file.read_u8()?;
        let file_type = file.read_u8()?;
        if file_type != 16 {
            return Err(SimpleError::new("Unknown s3m format"));
        }

        let _ = file.read_u16()?;

        let song_length = file.read_u16()?;

        if song_length > 256 {
            return Err(SimpleError::new("Unknown s3m format - song length"));
        }

        let instrument_count = file.read_u16()?;

        if instrument_count > 128 {
            return Err(SimpleError::new("Unknown s3m format - instruments"));
        }

        let pattern_count = file.read_u16()?;

        if pattern_count > 256 {
            return Err(SimpleError::new("Unknown s3m format - patterns"));
        }

        let _flags = file.read_u16()?;

        let _cwtv = file.read_u16()?;

        let _signed_samples = file.read_u16()?;


        let signature = file.read_string(4);

        if signature != "SCRM" {
            return Err(SimpleError::new("Unknown s3m format - signature"));
        }

        let global_volume = file.read_u8()?;
        let mut speed = file.read_u8()?;
        let mut bpm = file.read_u8()?;
        if speed == 0 { speed = 6; }
        if bpm == 0 { bpm = 125; }
        let master_volume = file.read_u8()?; // SCRM Master Volume
        let _ultra_click_removal = file.read_u8()?;
        let default_panning_present = file.read_u8()?;
        file.seek(SeekFrom::Current(10))?; // Skip reserved

        let channel_data = file.read_bytes(32)?;
        let mut channel_map = [255u8; 32];

        for i in 0..channel_data.len() {
            if channel_data[i] < 16u8 {
                channel_map[i] = num_channels;
                num_channels += 1;
            }
        }

        let mut pattern_order = file.read_bytes(song_length as usize)?;
        truncate_patterns(&mut pattern_order);

        let instrument_ptrs = file.read_u16_vec(instrument_count as usize)?;
        let pattern_ptrs = file.read_u16_vec(pattern_count as usize)?;

        let mut initial_channel_panning = [32u8; 64];
        if default_panning_present == 252 {
            let panning_data = file.read_bytes(32)?;
            for i in 0..32 {
                if panning_data[i] & 32 != 0 {
                    let p = panning_data[i] & 15;
                    initial_channel_panning[i] = p * 16 + 8; // Map 0-15 to 0-255 scale
                }
            }
        }

        let instruments = read_instruments(file, &instrument_ptrs)?;
        let mut patterns = read_patterns(file, &pattern_ptrs, num_channels as usize, &channel_map)?;

        patterns.push(Patterns {
            rows: vec![Row {
                channels: vec![Pattern {
                    note: 255,
                    instrument: 255,
                    volume: 255,
                    effect: 255,
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
            pattern_order: {
                let mut order = Vec::from_iter(pattern_order.iter().cloned());
                truncate_patterns(&mut order);
                order
            },
            instruments,
            use_amiga: true,
            song_message: "".to_string(),
            initial_channel_volume: [64; 64],
            initial_channel_panning,
            global_volume:           global_volume,
            master_volume:           master_volume & 0x7F, // Bit 7 is stereo/mono, 0-6 is vol
            mixing_volume:           128, // Default S3M mixing volume (modern tracker style)
            old_effects:             false,
            compatible_g:            true, // S3M always uses compatible G behavior
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

    fn read_patterns<R: Read + Seek>(file: &mut R, pattern_ptrs: &Vec<u16>, channel_count: usize, channel_map: &[u8; 32]) -> SimpleResult<Vec<Patterns>> {
        let pattern_count = pattern_ptrs.len();
        let mut patterns: Vec<Patterns> = vec![];
        patterns.reserve_exact(pattern_count);
        let row_count = 64;

        for pattern_ptr in pattern_ptrs.iter().cloned() {
            if pattern_ptr == 0 {continue;}
            file.seek(SeekFrom::Start((pattern_ptr as u64)  * 16))?;

            let mut pattern = Patterns::new(row_count, channel_count);

            let _size = file.read_u16()?;

            let mut last_instrument = [0u8; 32];

            for row in pattern.rows.iter_mut() {
                let channels = &mut row.channels;

                loop {
                    let pattern_data = file.read_u8()?;
                    if pattern_data == 0 { break; }

                    let channel_num = pattern_data & 31;
                    let channel_id = channel_map[channel_num as usize] as usize;

                    let mut note = 255u8;
                    let mut instrument = 255u8;
                    let mut volume = 255u8;
                    let mut effect = 255u8;
                    let mut effect_param = 0u8;

                    if pattern_data & 32 == 32 {
                        note = file.read_u8()?;
                        instrument = file.read_u8()?;

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
                        volume = file.read_u8()?;
                        if volume > 64 { volume = 64; }
                    }

                    if pattern_data & 128 == 128 {
                        effect = file.read_u8()?;
                        effect_param = file.read_u8()?;
                    }

                    if channel_num >= channel_count as u8 { continue; }
                    let channel = &mut channels[channel_id];

                    channel.note = note;
                    channel.instrument = instrument;
                    channel.volume = volume;

                    if pattern_data & 128 == 128 {
                        map_s3m_effect(channel, effect, effect_param, &mut last_instrument[channel_id]);
                    } else {
                        channel.effect = 255;
                        channel.effect_param = 0;
                    }

                    if channel.instrument != 255 && channel.instrument != 0 {
                        last_instrument[channel_id] = channel.instrument;
                    }

                    if channel.effect != 255 && channel.effect > 35 {
                        channel.effect = 255;
                        channel.effect_param = 0;
                    }
                }
            }
            patterns.push(pattern)
        }

        Ok(patterns)
    }

    fn map_s3m_effect(pattern : &mut Pattern, effect: u8, effect_param: u8, last_instrument: &mut u8) {
        if effect == 255 {
            pattern.effect = 255;
            pattern.effect_param = 0;
            return;
        }
        pattern.effect = 0;
        pattern.effect_param = effect_param;

        match effect {
            1 => // A - Set speed
                {
                    pattern.effect = 1; // A in IT
                }
            2 => pattern.effect = 2,  // B - Pattern Jump
            3 => pattern.effect = 3,  // C - Pattern Break
            4 => pattern.effect = 4,  // D - Volume Slide
            5 => pattern.effect = 5,  // E - Porta Down
            6 => pattern.effect = 6,  // F - Porta Up
            7 => pattern.effect = 7,  // G - Porta to note
            8 => pattern.effect = 8,  // H - Vibrato
            11 => pattern.effect = 11, // K - Vibrato + VolSlide
            12 => pattern.effect = 12, // L - Porta + VolSlide
            15 => pattern.effect = 15, // O - Sample offset
            19 => { // S - Extended commands
                pattern.effect = 19; // S in IT
            }
            20 => pattern.effect = 1,  // T - Set BPM
            22 => pattern.effect = 16, // V - Set Global Volume
            _ => { pattern.effect = 0; }
        }
        if pattern.instrument != 255 && pattern.instrument != 0 && pattern.effect != 0x03 {
            *last_instrument = pattern.instrument;
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
            file.seek(SeekFrom::Start((instrument_ptr as u64)  * 16))?;
            let _type_ = file.read_u8()?;
            let _dos_name = file.read_string(12);
            let sample_ptr = file.read_u24_s3m()?;
            let sample_len = file.read_u32()?;
            let sample_loop_start = file.read_u32()?;
            let sample_loop_end = file.read_u32()?;
            let sample_volume = file.read_u8()?;
            let _ = file.read_u8()?;
            let sample_packing = file.read_u8()?;
            if sample_packing != 0 {
                return Err(SimpleError::new("Unknown file format"));
            }
            let sample_flags = file.read_u8()?;
            let c2spd = file.read_u32()? & 0xFFFF;
            let _ = file.read_bytes(12)?;
            let sample_name = file.read_string(28);
            let _sample_sig = file.read_string(4);

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
                global_volume: 64,
                surround: false,
                is_ping_pong: false,
                original_loop_end: 0,
                data: vec![]
            };
            sample.read_s3m_sample_data(file, sample_ptr)?;
            instrument.samples = vec![sample];
            instrument.sample_indexes = vec![(0, 1); 120]; // Default mapping: All notes to primary sample
            instruments.push(instrument);
        }
        Ok(instruments)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::Cursor;

        #[test]
        fn test_read_u24_s3m() {
            let data = vec![0x01, 0x02, 0x03];
            let mut reader = Cursor::new(&data);
            assert_eq!(reader.read_u24_s3m().unwrap(), 0x010302);
        }
    }