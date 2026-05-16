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
    let mut result = [EnvelopePoint::new(); 25];

    for i in 0..12 {
        result[i].frame = file.read_u16()?;
        result[i].value = file.read_u16()?;
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
            c5_speed: 0, // XM uses (relative_note, finetune); not the formula path.
            name,
            global_volume: 64,
            surround: false,
            is_ping_pong: false,
            original_loop_end: 0,
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
                sample_indexes: sample_indexes.iter().enumerate().map(|(n, &s)| (n as u8, s + 1)).collect(),
                volume_envelope: Envelope::create(volume_envelope, volume_points, volume_sustain_point, 0, 0, volume_loop_start_point, volume_loop_end_point, volume_type),
                panning_envelope: Envelope::create(panning_envelope, panning_points, panning_sustain_point, 0, 0, panning_loop_start_point, panning_loop_end_point, panning_type),
                pitch_envelope: Envelope::new(),
                vibrato_envelope: VibratoEnvelope::create(vibrato_type, vibrato_sweep, vibrato_depth, vibrato_rate),
                volume_fadeout,
                nna: 0,
                dct: 0,
                dca: 0,
                global_volume: 64,
                initial_filter_cutoff: 127,
                initial_filter_resonance: 0,
                is_filter_envelope: false,
                samples: read_samples(file, sample_count as usize)?
            });
        } else {
            file.seek(SeekFrom::Start(instrument_pos + header_size as u64))?;
            instruments.push(Instrument::new());
        }
    }
    Ok(instruments)
}

/// Scan from the current position to end-of-file for an OpenMPT STPM
/// extended-song-properties block, locate the "PMM." (MixLevels) chunk
/// within it, and translate the value into our mixing_volume scaling.
/// Returns `Some(mixing_volume)` if a PMM. chunk was found, `None`
/// otherwise (caller keeps the previously-computed value).
///
/// File-format reference: Load_it.cpp:2532::LoadExtendedSongProperties.
/// Chunk layout after the STPM magic: 4-byte LE code + 2-byte LE size
/// + data. Code "PMM." appears in memory as bytes [".","M","M","P"]
/// (the file stores it as the LE-encoded magic of "PMM.").
fn scan_for_mptm_pmm<R: Read + Seek>(file: &mut R) -> Option<u8> {
    // Slurp the rest of the file. XM tail is typically small (the
    // STPM block is hundreds of bytes max), so reading to end is
    // fine.
    let mut rest = Vec::new();
    if file.read_to_end(&mut rest).is_err() { return None; }
    // Find STPM magic.
    let stpm = rest.windows(4).position(|w| w == b"STPM")?;
    let mut cp = stpm + 4;
    while cp + 6 <= rest.len() {
        let code = &rest[cp..cp + 4];
        let size = u16::from_le_bytes([rest[cp + 4], rest[cp + 5]]) as usize;
        // OMT bails when any high bit is set in the code or none of
        // the ASCII printable-marker bits are set — mirror that.
        if code.iter().any(|&b| b & 0x80 != 0) { break; }
        if code.iter().any(|&b| b & 0x60 == 0)  { break; }
        if cp + 6 + size > rest.len()           { break; }
        // ".MMP" in memory == big-endian "PMM." in the OMT source.
        if code == b".MMP" && size >= 1 {
            let mix_levels = rest[cp + 6];
            // MixLevels enum (SoundFilePlayConfig.h):
            //   0 = Original, 1..3 = v1_17RC*, 4 = Compatible,
            //   5 = CompatibleFT2.
            // Our XM_MIX is calibrated to CompatibleFT2 (preamp 192).
            // Compatible / Original render at 192/256 of that.
            // Only the Compatible (= 4) value is reproducible with our
            // single calibration knob (mixing_volume × 0.75). Original
            // (= 0) involves a different gain pipeline in OMT
            // (extraSampleAttenuation = 4 vs Compatible's 1, plus
            // useGlobalPreAmp = true) that we don't model; leave it
            // untouched so we don't accidentally regress files like
            // cerror_-_crack_05.xm (a separate-bug r=0.53 outlier
            // that already renders quieter than OMT does).
            return Some(match mix_levels {
                4 => ((128u32 * 192) / 256) as u8,  // 96
                _ => 128,                            // unchanged
            });
        }
        cp += 6 + size;
    }
    None
}

fn read_xm_header<R: Read + Seek>(file: &mut R) -> SimpleResult<SongData>
{
    let id = file.read_string(17);
    if id != "Extended Module: " {
        return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Not an XM module")));
    }

    let name = file.read_string(20);
    let sig = file.read_u8()?;
    if sig != 0x1a {
        return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Not an XM module")));
    }

    let tracker_name = file.read_string(20);
    let _ver = file.read_u16()?;

    // OMT MixLevels detection — see Load_xm.cpp:608-1045. We can't
    // mirror every variant of the playBehaviour matrix, but the rule
    // that affects raw output gain is the normalSamplePreAmp scale:
    //   CompatibleFT2 (our calibration target): preamp 192
    //   Compatible:                              preamp 256
    //   Original:                                preamp 256
    // The 256/192 = 1.333× difference is exactly the outlier ratio
    // we see for several MilkyTracker XM files in the corpus.
    //
    // For MilkyTracker without a version string (pre-0.90.87 — the
    // explicit branch in OMT only sets MixLevels::CompatibleFT2 when
    // bytes 12..20 of the tracker name carry a non-space version),
    // OMT leaves the default MixLevels::Compatible from
    // Sndfile.cpp:211 in place. Detect this case and scale our
    // mixing_volume to land at the same level — without it, files
    // like xem_fdl / xem_po / db_night render 33% too loud.
    let is_milky_tracker_no_version =
        tracker_name.starts_with("MilkyTracker ")
        && tracker_name.len() >= 20
        && tracker_name.as_bytes()[13..20].iter().all(|&b| b == b' ');
    // Sample-preamp ratio scaling: Compatible mode renders at 192/256 of
    // CompatibleFT2 mode. Encode that in mixing_volume (rounded to u8).
    let mixing_volume: u8 = if is_milky_tracker_no_version {
        ((128u32 * 192) / 256) as u8  // = 96
    } else {
        128
    };

    let header_size = file.read_u32()?;
    let mut song_length = file.read_u16()?;
    let restart_position = file.read_u16()?;
    let channel_count = file.read_u16()?;
    let pattern_count = file.read_u16()?;
    let instrument_count = file.read_u16()?;
    let flags = file.read_u16()?;
    let tempo = file.read_u16()?;
    let bpm = file.read_u16()?;
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
    }

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

    // OpenMPT-saved XM files append an "STPM" chunk block after the
    // main XM data (Load_it.cpp:2532::LoadExtendedSongProperties is
    // called from Load_xm.cpp:1096 against the same reader). The
    // block contains modular sub-chunks; the "PMM." one carries
    // m_nMixLevels. We've already seen the tracker-name-based
    // detection above, but PMM. *overrides* it when present.
    //
    // OpenMPT 1.26-1.30 writes Compatible (4) in PMM. for XM files
    // even though the tracker-name path sets CompatibleFT2 — that's
    // the OMT-bug we have to match here. 1.23 and 1.32+ write
    // CompatibleFT2 (5) which is our XM_MIX calibration target.
    //
    // Each chunk: 4-byte LE code, 2-byte LE size, `size` bytes data.
    // PMM. code "PMM." reads in memory as ".MMP" (LE order) — we just
    // match the four bytes literally.
    let mixing_volume = scan_for_mptm_pmm(file).unwrap_or(mixing_volume);

    Ok(SongData {
        id: id.trim().to_string(),
        name: name.trim().to_string(),
        file_name: String::new(),
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
        initial_channel_volume: [64; 64],
        initial_channel_panning: [128; 64],
        initial_channel_surround: [false; 64],
        global_volume:           64,
        master_volume:           128,
        mixing_volume,
        old_effects: false,
        compatible_g: false,
        fast_volume_slides: false,
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
