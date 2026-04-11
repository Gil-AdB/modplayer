use std::io::{Read, Seek, SeekFrom};
use crate::module_reader::{Patterns, Row, SongData, SongType, FrequencyType};
use binary_reader_io::BinaryReader;
use crate::pattern::Pattern;
use crate::envelope::{EnvelopePoints, EnvelopePoint, Envelope};
use crate::instrument::{Sample, LoopType, Instrument, VibratoEnvelope};
use std::iter::FromIterator;
use crate::{SimpleResult, SimpleError};
use std::io;

fn read_patterns<R: Read>(file: &mut R, pattern_count: usize, channel_count: usize) -> SimpleResult<Vec<Patterns>> {
    let mut patterns: Vec<Patterns> = vec![];
    patterns.reserve_exact(pattern_count as usize);

    for _pattern_idx in 0..pattern_count {
        let _pattern_header_size = file.read_u32()?;
        let _pattern_type = file.read_u8()?;
        let row_count = file.read_u16()?;
        let pattern_size = file.read_u16()?;

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
                let flags = file.read_u8()?;
                channels.push(if flags & 0x80 == 0x80 {
                    pos += 1;
                    let note = if flags & 1 == 1 {
                        pos += 1;
                        file.read_u8()?
                    } else { 0 };
                    let instrument = if flags & 2 == 2 {
                        pos += 1;
                        file.read_u8()?
                    } else { 0 };
                    let volume = if flags & 4 == 4 {
                        pos += 1;
                        file.read_u8()?
                    } else { 0 };
                    let effect = if flags & 8 == 8 {
                        pos += 1;
                        file.read_u8()?
                    } else { 0 };
                    let effect_param = if flags & 16 == 16 {
                        pos += 1;
                        file.read_u8()?
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
                    let instrument = file.read_u8()?;
                    let volume = file.read_u8()?;
                    let effect = file.read_u8()?;
                    let effect_param = file.read_u8()?;
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
            return Err(SimpleError::from(format!("size {} != pos {}", pattern_size, pos)));
        }
        patterns.push(Patterns { rows })
    }

    Ok(patterns)
}

fn read_envelope<R: Read>(file: &mut R) -> SimpleResult<EnvelopePoints> {
    let mut result = [EnvelopePoint::new(); 12];

    for point in &mut result {
        point.frame = file.read_u16()?;
        point.value = file.read_u16()?;
    }
    Ok(result)
}

fn read_samples<R: Read>(file: &mut R, sample_count: usize) -> SimpleResult<Vec<Sample>> {
    let mut samples: Vec<Sample> = vec![];
    samples.reserve_exact(sample_count as usize);

    for _sample_idx in 0..sample_count {
        let mut length = file.read_u32()?;
        let mut loop_start = file.read_u32()?;
        let mut loop_len = file.read_u32()?;
        let volume = file.read_u8()?;
        let finetune = file.read_i8()?;
        let flags = file.read_u8()?;
        let panning = file.read_u8()?;
        let relative_note = file.read_i8()?;
        let _reserved = file.read_u8()?;
        let name = file.read_string(22);

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
        sample.read_data(file)?;
    }

    Ok(samples)
}

fn read_instruments<R: Read + Seek>(file: &mut R, instrument_count: usize) -> SimpleResult<Vec<Instrument>> {
    let mut instruments: Vec<Instrument> = vec![];

    // Instruments are one based, go figure. We'll add an empty instrument as sample 0.
    instruments.reserve_exact(instrument_count + 1 as usize);

    instruments.push(Instrument::new());

    for instrument_idx in 0..instrument_count {
        let instrument_pos = file.seek(SeekFrom::Current(0))?;
        let header_size = file.read_u32()?;
        let name = file.read_string(22);
        let _instrument_type = file.read_u8()?;
        let sample_count = file.read_u16()?;


        if sample_count > 0 {
            let _sample_sig = file.read_string(4);
            let sample_indexes = file.read_bytes(96)?;
            let volume_envelope = read_envelope(file)?;
            let panning_envelope = read_envelope(file)?;
            let volume_points = file.read_u8()?;
            let panning_points = file.read_u8()?;
            let volume_sustain_point = file.read_u8()?;
            let volume_loop_start_point = file.read_u8()?;
            let volume_loop_end_point = file.read_u8()?;
            let panning_sustain_point = file.read_u8()?;
            let panning_loop_start_point = file.read_u8()?;
            let panning_loop_end_point = file.read_u8()?;
            let volume_type = file.read_u8()?;
            let panning_type = file.read_u8()?;
            let vibrato_type = file.read_u8()?;
            let vibrato_sweep = file.read_u8()?;
            let vibrato_depth = file.read_u8()?;
            let vibrato_rate = file.read_u8()?;
            let volume_fadeout = file.read_u16()?;
            let _reserved = file.read_u16()?;

            file.seek(SeekFrom::Start(instrument_pos + header_size as u64))?;
            instruments.push(Instrument {
                name,
                idx: (instrument_idx + 1) as u8,
                sample_indexes,
                volume_envelope: Envelope::create(volume_envelope, volume_points, volume_sustain_point, volume_loop_start_point, volume_loop_end_point, volume_type),
                panning_envelope: Envelope::create(panning_envelope,panning_points, panning_sustain_point, panning_loop_start_point, panning_loop_end_point,panning_type),
                vibrato_envelope: VibratoEnvelope::create(vibrato_type, vibrato_sweep, vibrato_depth, vibrato_rate),
                volume_fadeout,
                samples: read_samples(file, sample_count as usize)?
            });
        } else {
            file.seek(SeekFrom::Start(instrument_pos + header_size as u64))?;
            instruments.push(Instrument::new());
        }
    }
    Ok(instruments)
}

fn read_xm_header<R: Read + Seek>(file: &mut R) -> SimpleResult<SongData>
{
    let id = file.read_string(17);
    if id != "Extended Module: " {
        return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Not an XM module")));
    }

    dbg!(&id);
    let name = file.read_string(20);
    dbg!(&name);
    let sig = file.read_u8()?;
    if sig != 0x1a {
        return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Not an XM module")));
    }

    let tracker_name = file.read_string(20);
    dbg!(&tracker_name);

    let ver = file.read_u16()?;
    dbg!(format!("{:x}", ver));

//    dbg!(file.seek(SeekFrom::Current(0)));

        let header_size = file.read_u32()?;
        dbg!(header_size);

        let mut song_length = file.read_u16()?;
        dbg!(song_length);

        let restart_position = file.read_u16()?;
        dbg!(restart_position);

        let channel_count = file.read_u16()?;
        dbg!(channel_count);

        let pattern_count = file.read_u16()?;
        dbg!(pattern_count);

        let instrument_count = file.read_u16()?;
        dbg!(instrument_count);

        let flags = file.read_u16()?;
        dbg!(flags);

        let tempo = file.read_u16()?;
        dbg!(tempo);

        let bpm = file.read_u16()?;
        dbg!(bpm);
        let stream_position;
        if let Ok(pos) = file.seek(SeekFrom::Current(0)) { stream_position = pos; } else { stream_position = 20 }

        let mut pattern_order = file.read_bytes((60 + header_size - stream_position as u32) as usize)?;

        let mut patterns = read_patterns(file, pattern_count as usize, channel_count as usize)?;

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

        let instruments = read_instruments(file, instrument_count as usize)?;

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
            use_amiga: (flags & 1) != 1,
            song_message: "".to_string(),
        })
    }

    pub fn read_xm<R: Read + Seek>(file: &mut R) -> SimpleResult<SongData> {
        file.seek(SeekFrom::Start(0))?;

        let file_len = file.seek(SeekFrom::End(0))?;
        file.seek(SeekFrom::Start(0))?;

        if file_len < 60 {
            return Err(SimpleError::new("File is too small!"));
        }

        read_xm_header(file)
    }
