use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::iter::FromIterator;

use crate::envelope::{Envelope, EnvelopePoint, EnvelopePoints};
use crate::instrument::{Instrument, LoopType, Sample};
use crate::io_helpers as fio;
use crate::pattern::Pattern;

#[derive(Debug)]
enum SongType {
    XM
}

#[derive(Debug)]
enum FrequencyType {
    AMIGA,
    LINEAR
}
pub(crate) fn is_note_valid(note: u8) -> bool {
    note > 0 && note < 97
}

#[derive(Clone)]
pub(crate) struct Row {
    pub(crate) channels: Vec<Pattern>
}

impl fmt::Debug for Row {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for pattern in &self.channels {
            if first { first = false; } else { write!(f, "|")?; }
            write!(f, "{}", pattern)?;
        }
        Ok(())
    }
}

impl fmt::Display for Row {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for pattern in &self.channels {
            if first { first = false; } else { write!(f, "|")?; }
            write!(f, "{}", pattern)?;
        }
        Ok(())
    }
}


#[derive(Debug)]
pub(crate) struct Patterns {
    pub(crate) rows: Vec<Row>
}


fn read_patterns<R: Read>(file: &mut R, pattern_count: usize, channel_count: usize) -> Vec<Patterns> {
    let mut patterns: Vec<Patterns> = vec![];
    patterns.reserve_exact(pattern_count as usize);

    for _pattern_idx in 0..pattern_count {
        let _pattern_header_size = fio::read_u32(file);
        let _pattern_type = fio::read_u8(file);
        let row_count = fio::read_u16(file);
        let pattern_size = fio::read_u16(file);

        let mut pos = 0usize;
        if pattern_size == 0 {
            patterns.push(Patterns{ rows: vec![Row{ channels: vec![Pattern{
                note: 0,
                instrument: 0,
                volume: 0,
                effect: 0,
                effect_param: 0
            }; channel_count] }; 64] });
            continue;
        }

        let mut rows: Vec<Row> = vec![];
        rows.reserve_exact(row_count as usize);
        for _row_idx in 0..row_count {
            let mut channels: Vec<Pattern> = vec![];
            channels.reserve_exact(channel_count);
            for _channel_idx in 0..channel_count {
                let flags = fio::read_u8(file);
                channels.push(if flags & 0x80 == 0x80 {
                    pos += 1;
                    let note = if flags & 1 == 1 {
                        pos += 1;
                        fio::read_u8(file)
                    } else { 0 };
                    let instrument = if flags & 2 == 2 {
                        pos += 1;
                        fio::read_u8(file)
                    } else { 0 };
                    let volume = if flags & 4 == 4 {
                        pos += 1;
                        fio::read_u8(file)
                    } else { 0 };
                    let effect = if flags & 8 == 8 {
                        pos += 1;
                        fio::read_u8(file)
                    } else { 0 };
                    let effect_param = if flags & 16 == 16 {
                        pos += 1;
                        fio::read_u8(file)
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
                    let instrument = fio::read_u8(file);
                    let volume = fio::read_u8(file);
                    let effect = fio::read_u8(file);
                    let effect_param = fio::read_u8(file);
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
        point.frame = fio::read_u16(file);
        point.value = fio::read_u16(file);
    }
    result
}

fn read_samples<R: Read>(file: &mut R, sample_count: usize) -> Vec<Sample> {
    let mut samples: Vec<Sample> = vec![];
    samples.reserve_exact(sample_count as usize);

    for sample_idx in 0..sample_count {
        println!("Reading sample #{} of {}", sample_idx, sample_count);

        let mut length = fio::read_u32(file);
        let mut loop_start = fio::read_u32(file);
        let mut loop_len = fio::read_u32(file);
        let volume = fio::read_u8(file);
        let finetune = fio::read_i8(file);
        let flags = fio::read_u8(file);
        let panning = fio::read_u8(file);
        let relative_note = fio::read_i8(file);
        let _reserved = fio::read_u8(file);
        let name = fio::read_string(file, 22);

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
        let header_size = fio::read_u32(file);
        let name = fio::read_string(file, 22);
        let _instrument_type = fio::read_u8(file);
        let sample_count = fio::read_u16(file);


        if sample_count > 0 {
            let _sample_size = fio::read_u32(file);
            let sample_indexes = fio::read_bytes(file, 96);
            let volume_envelope = read_envelope(file);
            let panning_envelope = read_envelope(file);
            let volume_points = fio::read_u8(file);
            let panning_points = fio::read_u8(file);
            let volume_sustain_point = fio::read_u8(file);
            let volume_loop_start_point = fio::read_u8(file);
            let volume_loop_end_point = fio::read_u8(file);
            let panning_sustain_point = fio::read_u8(file);
            let panning_loop_start_point = fio::read_u8(file);
            let panning_loop_end_point = fio::read_u8(file);
            let volume_type = fio::read_u8(file);
            let panning_type = fio::read_u8(file);
            let vibrato_type = fio::read_u8(file);
            let vibrato_sweep = fio::read_u8(file);
            let vibrato_depth = fio::read_u8(file);
            let vibrato_rate = fio::read_u8(file);
            let volume_fadeout = fio::read_u16(file);
            let _reserved = fio::read_u16(file);

            file.seek(SeekFrom::Start(instrument_pos + header_size as u64)).unwrap();
            instruments.push(Instrument {
                name,
                idx: (instrument_idx + 1) as u8,
                sample_indexes,
                volume_envelope: Envelope {
                    points: volume_envelope,
                    size: volume_points,
                    sustain_point: volume_sustain_point,
                    loop_start_point: volume_loop_start_point,
                    loop_end_point: volume_loop_end_point,
                    on: (volume_type & 1) == 1,
                    sustain: (volume_type & 2) == 2,
                    has_loop: (volume_type & 4) == 4,
                },
                panning_envelope: Envelope {
                    points: panning_envelope,
                    size: panning_points,
                    sustain_point: panning_sustain_point,
                    loop_start_point: panning_loop_start_point,
                    loop_end_point: panning_loop_end_point,
                    on: (panning_type & 1) == 1,
                    sustain: (panning_type & 2) == 2,
                    has_loop: (panning_type & 4) == 4,
                },
                vibrato_type,
                vibrato_sweep,
                vibrato_depth,
                vibrato_rate,
                volume_fadeout,
                samples: read_samples(file, sample_count as usize)
            });
        } else {
            if let Err(e) = file.seek(SeekFrom::Start(instrument_pos + header_size as u64)) {panic!(e);}
            instruments.push(Instrument::new());
        }
    }
    instruments
}


fn read_xm_header<R: Read + Seek>(mut file: &mut R) -> SongData
{
    let id = fio::read_string(&mut file, 17);
    dbg!(&id);
    let name = fio::read_string(&mut file, 20);
    dbg!(&name);
    let sig = fio::read_u8(file);
    if sig != 0x1a {
        panic!("Wrong Format!")
    }

    let tracker_name = fio::read_string(file, 20);
    dbg!(&tracker_name);

    let ver = fio::read_u16(file);
    dbg!(format!("{:x}", ver));

//    dbg!(file.seek(SeekFrom::Current(0)));

    let header_size = fio::read_u32(file);
    dbg!(header_size);

    let mut song_length = fio::read_u16(file);
    dbg!(song_length);

    let restart_position = fio::read_u16(file);
    dbg!(restart_position);

    let channel_count = fio::read_u16(file);
    dbg!(channel_count);

    let pattern_count = fio::read_u16(file);
    dbg!(pattern_count);

    let instrument_count = fio::read_u16(file);
    dbg!(instrument_count);

    let flags = fio::read_u16(file);
    dbg!(flags);

    let tempo = fio::read_u16(file);
    dbg!(tempo);

    let bpm = fio::read_u16(file);
    dbg!(bpm);
    let mut stream_position = 0;
    if let Ok(pos) = file.stream_position() {stream_position = pos;} else {stream_position = 20}

    let mut pattern_order = fio::read_bytes(file, (60 + header_size - stream_position as u32) as usize);



    let mut patterns = read_patterns(file, pattern_count as usize, channel_count as usize);

    // fix empty patterns at end
    for idx in 0..pattern_order.len() {
        if pattern_order[idx] >= patterns.len() as u8 {
            pattern_order[idx] = patterns.len() as u8;
        }
    }
    if song_length > pattern_order.len() as u16 {
        song_length = pattern_order.len() as u16;
        dbg!("Trimming song lonegth to {}", song_length);
    }
    // dbg!(&pattern_order);

    patterns.push(Patterns{ rows: vec![Row{ channels: vec![Pattern{
        note: 0,
        instrument: 0,
        volume: 0,
        effect: 0,
        effect_param: 0
    }; channel_count as usize] }; 64] });

    let instruments = read_instruments(file, instrument_count as usize);

    SongData {
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
    }
}

#[derive(Debug)]
pub struct SongData {
                    id:                 String,
                    name:               String,
                    song_type:          SongType,
                    tracker_name:       String,
    pub(crate)      song_length:        u16,
    pub(crate)      restart_position:   u16,
                    channel_count:      u16,
    pub(crate)      patterns:           Vec<Patterns>,
                    instrument_count:   u16,
                    frequency_type:     FrequencyType,
    pub(crate)      tempo:              u16,
    pub(crate)      bpm:                u16,
    pub(crate)      pattern_order:      Vec<u8>,
    pub(crate)      instruments:        Vec<Instrument>,
    pub(crate)      use_amiga:          bool,
}

pub fn read_xm(path: &str) -> SongData {
    let f = File::open(path).expect("failed to open the file");
    let file_len = f.metadata().expect("Can't read file metadata").len();
    let mut file = BufReader::new(f);


    // println!("file length: {}", file_len);
    if file_len < 60 {
        panic!("File is too small!")
    }

    let song_data = read_xm_header(&mut file);
    //   dbg!(song_data);

    //  dbg!(file.seek(SeekFrom::Current(0)));

    song_data
}

// pub fn read_mod(path: &str) -> SongData {
//     let f = File::open(path).expect("failed to open the file");
//     let file_len = f.metadata().expect("Can't read file metadata").len();
//     let mut file = BufReader::new(f);
//
//
//     // println!("file length: {}", file_len);
//     if file_len < 1084 {
//         panic!("File is too small!")
//     }
//
//     let song_data = read_mod_header(&mut file);
//     //   dbg!(song_data);
//
//     //  dbg!(file.seek(SeekFrom::Current(0)));
//
//     song_data
// }
//
// fn read_mod_header<R: Read + Seek>(mut file: &mut R) -> SongData
// {
//     let id = fio::read_string(&mut file, 17);
//     dbg!(&id);
//     let name = fio::read_string(&mut file, 20);
//     dbg!(&name);
//     let sig = fio::read_u8(file);
//     if sig != 0x1a {
//         panic!("Wrong Format!")
//     }
//
//     let tracker_name = fio::read_string(file, 20);
//     dbg!(&tracker_name);
//
//     let ver = fio::read_u16(file);
//     dbg!(format!("{:x}", ver));
//
// //    dbg!(file.seek(SeekFrom::Current(0)));
//
//     let header_size = fio::read_u32(file);
//     dbg!(header_size);
//
//     let mut song_length = fio::read_u16(file);
//     dbg!(song_length);
//
//     let restart_position = fio::read_u16(file);
//     dbg!(restart_position);
//
//     let channel_count = fio::read_u16(file);
//     dbg!(channel_count);
//
//     let pattern_count = fio::read_u16(file);
//     dbg!(pattern_count);
//
//     let instrument_count = fio::read_u16(file);
//     dbg!(instrument_count);
//
//     let flags = fio::read_u16(file);
//     dbg!(flags);
//
//     let tempo = fio::read_u16(file);
//     dbg!(tempo);
//
//     let bpm = fio::read_u16(file);
//     dbg!(bpm);
//     let mut stream_position = 0;
//     if let Ok(pos) = file.stream_position() {stream_position = pos;} else {stream_position = 20}
//
//     let mut pattern_order = fio::read_bytes(file, (60 + header_size - stream_position as u32) as usize);
//
//
//
//     let mut patterns = read_patterns(file, pattern_count as usize, channel_count as usize);
//
//     // fix empty patterns at end
//     for idx in 0..pattern_order.len() {
//         if pattern_order[idx] >= patterns.len() as u8 {
//             pattern_order[idx] = patterns.len() as u8;
//         }
//     }
//     if song_length > pattern_order.len() as u16 {
//         song_length = pattern_order.len() as u16;
//         dbg!("Trimming song lonegth to {}", song_length);
//     }
//     // dbg!(&pattern_order);
//
//     patterns.push(Patterns{ rows: vec![Row{ channels: vec![Pattern{
//         note: 0,
//         instrument: 0,
//         volume: 0,
//         effect: 0,
//         effect_param: 0
//     }; channel_count as usize] }; 64] });
//
//     let instruments = read_instruments(file, instrument_count as usize);
//
//     SongData {
//         id: id.trim().to_string(),
//         name: name.trim().to_string(),
//         song_type: SongType::XM,
//         tracker_name: tracker_name.trim().to_string(),
//         song_length,
//         restart_position,
//         channel_count,
//         patterns,
//         instrument_count,
//         frequency_type: if (flags & 1) == 1 { FrequencyType::LINEAR } else { FrequencyType::AMIGA },
//         tempo,
//         bpm,
//         pattern_order: Vec::from_iter(pattern_order.iter().cloned()),
//         instruments,
//         use_amiga: (flags & 1) != 1
//     }
// }


pub fn print_xm(data: &SongData) {
    dbg!(&data.patterns[data.pattern_order[22] as usize]);
    // println!("=====================================================================");
    dbg!(&data.patterns[data.pattern_order[23] as usize]);
}