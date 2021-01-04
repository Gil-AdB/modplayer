pub(crate) mod stm {
    use crate::module_reader::{SongData, Patterns, Row, SongType, FrequencyType};
    use std::fs::File;
    use std::io::{BufReader, Read, Seek, SeekFrom};
    use crate::io_helpers::{BinaryReader};
    use std::iter::FromIterator;
    use crate::pattern::Pattern;
    use crate::instrument::{Instrument, Sample, LoopType};
    use crate::{io_helpers as fio, module_reader};
    use simple_error::{SimpleError, SimpleResult};
    use std::io;
    use std::cmp::min;
    use std::num::Wrapping;
    use crate::channel_state::channel_state::clamp;

    const STM_EFFECTS: [u8;16] = [0, 0, 11, 0, 10, 2, 1, 3, 4, 7, 0, 5, 6, 0, 0, 0];


    pub fn read_stm<R: Read + Seek>(mut file: &mut R) -> SimpleResult<SongData> {
        file.seek(SeekFrom::Start(0));

        let file_len = match file.stream_len() {
            Ok(m) => {m}
            Err(_) => {return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Can't read file metadata")));}
        };

        if file_len < 0x3D0  {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "File is too small!")));
        }

        let song_data = read_stm_header(&mut file);

        song_data
    }

    fn to_bcd(num: u8) -> u8 {
        ((num / 10) << 4) + (num % 10)
    }

    fn read_stm_header<R: Read + Seek>(file: &mut R) -> SimpleResult<SongData>
    {
        // Mostly lifted from ft2-clone. Docs are not reliable...
        let num_channels = 4;

        let name              = file.read_string(20);
        let tracker_name      = file.read_string(8);
        dbg!(&tracker_name);
        if tracker_name != "!Scream!" && tracker_name != "BMOD2STM" &&
           tracker_name != "WUZAMOD!" && tracker_name != "SWavePro" {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Unknown stm tracker_name")));
        }

        let id                  = file.read_u8();
        if id != 0x1A {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Unknown stm signature")));
        }

        let file_type = file.read_u8();
        let _major = file.read_u8();
        dbg!(_major);
        let minor = file.read_u8();

        if file_type != 2 || minor == 0 {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Unknown stm version")));
        }

        let mut tempo = file.read_u8();
        let mut bpm = tempo;
        dbg!(bpm);
        if minor < 21 {bpm = to_bcd(tempo);} // to BCD?
        if bpm == 0 {
            bpm = 96;
        }
        bpm = stm_tempo_to_bpm(bpm);

        tempo = clamp(tempo >> 4, 1, 31);

        let pattern_count   = file.read_u8();
        let mut _global_volume = file.read_u8();
        if minor > 10 {
            _global_volume = min(_global_volume, 64);
        }

        if let Err(e) = file.seek(SeekFrom::Current(13)) {
            return Err(SimpleError::from(e));
        }

        let mut instruments = read_instruments(file);

        let mut pattern_order = fio::read_bytes(file, 128);

        let song_length = pattern_order.iter().cloned().position(|x| {x >= 99}).unwrap();

        pattern_order.resize(song_length + 1, 0);

        let mut patterns = read_patterns(file, pattern_count as usize, num_channels as usize, minor);

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
            id: tracker_name,
            name: name.trim().to_string(),
            song_type: SongType::STM,
            tracker_name: "Unknown".to_string(),
            song_length: song_length as u16,
            restart_position: 0,
            channel_count: num_channels,
            patterns,
            instrument_count: instruments.len() as u16,
            frequency_type: FrequencyType::AMIGA,
            tempo: tempo as u16,
            bpm: bpm as u16,
            pattern_order: Vec::from_iter(pattern_order.iter().cloned()),
            instruments,
            use_amiga: true
        })
    }

    fn stm_tempo_to_bpm(tempo: u8) -> u8 {
        const SLOWDOWNS: [u16; 16] = [140, 50, 25, 15, 10, 7, 6, 4, 3, 3, 2, 2, 2, 2, 1, 1];
        let mut hz = 50u16;

        hz = (Wrapping(hz) - Wrapping(((SLOWDOWNS[(tempo >> 4) as usize] * (tempo & 15) as u16) >> 4) as u16)).0; // can and will underflow
        let bpm = (Wrapping(hz << 1) + Wrapping(hz >> 1)).0; // BPM = hz * 2.5
        clamp(bpm as u8, 32, 255) // result can be slightly off, but close enough...
    }

    fn read_sample_data<R: Read + Seek>(f: &mut R, instruments: &mut Vec<Instrument>) {
        for i in 1..instruments.len() {
            instruments[i].samples[0].read_non_packed_data(f);
        }
    }

    fn read_patterns<R: Read>(file: &mut R, pattern_count: usize, channel_count: usize, minor: u8) -> Vec<Patterns> {
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
                    let data = fio::read_u32(file);

                    // 00000000 00000000 00000000 11111111
                    let mut note = (data & 0xFF) as u8;

                    if note == 254 {
                        note = 97;
                    } else if note < 96 {
                        note = (12 * (note >> 4)) + (25 + (note & 0xF));
                        if note > 96 {
                            note = 0;
                        }
                    } else {
                        note = 0;
                    }

                    // 00000000 00000000 11111000 00000000
                    let instrument = ((data & 0xF800) >> 11) as u8;

                    // 00000000 00000000 00000111 00000000 >> 8  ==> 00000000 00000000 00000000 00000111
                    // 00000000 11110000 00000000 00000000 >> 17 ==> 00000000 00000000 00000000 01111000
                    // 7 bits for volume
                    let mut volume = (((data & 0xF00000) >> 17) | (data & 0x700 >> 8)) as u8;
                    volume = if volume <= 64 {volume + 0x10} else {0};

                    let mut effect_param = ((data & 0xFF000000) >> 24) as u8;

                    let mut effect = 0;
                    let tmp = ((data & 0xF0000) >> 16) as u8;
                    if tmp == 1 {
                        effect = 15;
                        if minor < 21 {
                            effect_param = to_bcd(effect_param);
                        }
                        effect_param >>= 4;
                    } else if tmp == 3 {
                        effect = 13;
                        effect_param = 0;
                    } else if tmp == 2 || (tmp >=4 && tmp <= 12) {
                        effect = STM_EFFECTS[tmp as usize];
                        if effect == 0xA {
                            if (effect & 0xF) != 0 { // priority
                                effect &= 0x0F
                            } else {
                                effect &= 0xF0;
                            }
                        }
                    } else {
                        effect_param = 0;
                    }


                    // Shouldn't happen
                    // if effect > 35 {
                    //     effect = 0;
                    //     effect_param = 0;
                    // }

                    channels.push(
                        Pattern {
                            note,
                            instrument,
                            volume,
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

    fn read_instrument<R: Read>(file: &mut R) -> Sample {
        let name          = fio::read_string(file, 12);
        let _id                  = file.read_u8();
        let _instrument_disk     = file.read_u8();  // yeah, whatever...?
        let _                    = file.read_u16(); // reserved

        let length          = file.read_u16();
        let mut loop_start      = file.read_u16();
        let mut loop_end        = file.read_u16();
        let mut loop_len        = loop_end - loop_start;
        let mut loop_type            = LoopType::NoLoop;

        if loop_start < length && loop_end > loop_start && loop_end != 0xffff {
            if loop_start + loop_end > length {
                loop_len = length - loop_start;
            }
            loop_type = LoopType::ForwardLoop;
        } else {
            loop_start  = 0;
            loop_end    = 0;
            loop_len    = 0;
        }

        let volume           = file.read_byte();
        let _                    = file.read_byte(); // reserved

        let c3freq              = file.read_u32();
        let _                        = file.read_u16(); // reserved
        let _length_in_paragraphs    = file.read_u16();

        let (finetune, relative_note) = module_reader::c2spd_to_finetune_relnote(c3freq);

        Sample {
            length: length as u32,
            loop_start: loop_start as u32,
            loop_end: loop_end as u32,
            loop_len: loop_len as u32,
            volume: clamp(volume, 0, 64),
            finetune,
            loop_type,
            bitness: 8,
            panning: 128,
            relative_note,
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

        for instrument_idx in 1..instrument_count + 1 {
            let mut instrument = Instrument::new();
            let sample = read_instrument(file);
            instrument.name = sample.name.clone();
            instrument.idx = instrument_idx as u8;
            instrument.samples = vec![sample];
            instruments.push(instrument);
        }
        instruments
    }
}