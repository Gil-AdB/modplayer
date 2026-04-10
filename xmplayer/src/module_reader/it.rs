use std::io::{Read, Seek, SeekFrom};
use binary_reader_io::BinaryReader;
use std::io;
use crate::{SimpleResult, SimpleError};

use crate::module_reader::{Patterns, Row, SongData, it_compression};
use crate::pattern::Pattern;
use crate::envelope::{Envelope, EnvelopePoint, EnvelopePoints};
use crate::instrument::{Instrument, LoopType, Sample, VibratoEnvelope};

    fn read_patterns<R: Read>(file: &mut R, pattern_count: usize, channel_count: usize) -> SimpleResult<Vec<Patterns>> {
        let mut patterns: Vec<Patterns> = vec![];

        for _ in 0..pattern_count {
            let _length = file.read_u16()?;
            let row_count = file.read_u16()?;
            let _reserved = file.read_u32()?;
            
            let mut rows = vec![Row { channels: vec![Pattern::new(); channel_count] }; row_count as usize];
            let mut last_mask = [0u8; 64];
            let mut last_note = [0u8; 64];
            let mut last_instr = [0u8; 64];
            let mut last_vol = [0u8; 64];
            let mut last_effect = [0u8; 64];
            let mut last_effect_param = [0u8; 64];

            for row_idx in 0..row_count {
                loop {
                    let channel_var = file.read_u8()?;
                    if channel_var == 0 { break; }
                    
                    let channel_idx = (channel_var - 1) & 63;
                    if channel_var & 128 != 0 {
                        last_mask[channel_idx as usize] = file.read_u8()?;
                    }
                    
                    let mask = last_mask[channel_idx as usize];
                    let mut p = Pattern::new();
                    
                    if mask & 1 != 0 {
                        last_note[channel_idx as usize] = file.read_u8()?;
                    }
                    if mask & 17 != 0 {
                         p.note = last_note[channel_idx as usize];
                    }

                    if mask & 2 != 0 {
                        last_instr[channel_idx as usize] = file.read_u8()?;
                    }
                    if mask & 34 != 0 {
                        p.instrument = last_instr[channel_idx as usize];
                    }

                    if mask & 4 != 0 {
                        last_vol[channel_idx as usize] = file.read_u8()?;
                    }
                    if mask & 68 != 0 {
                        p.volume = last_vol[channel_idx as usize];
                    }

                    if mask & (8 | 16) != 0 {
                        if mask & 8 != 0 {
                            last_effect[channel_idx as usize] = file.read_u8()?;
                            last_effect_param[channel_idx as usize] = file.read_u8()?;
                        }
                        p.effect = last_effect[channel_idx as usize];
                        p.effect_param = last_effect_param[channel_idx as usize];
                    }
                    
                    if (channel_idx as usize) < channel_count {
                        rows[row_idx as usize].channels[channel_idx as usize] = p;
                    }
                }
            }
            patterns.push(Patterns { rows });
        }

        Ok(patterns)
    }

    #[allow(dead_code)]
    fn read_envelope<R: Read>(file: &mut R) -> EnvelopePoints {
        let mut result = [EnvelopePoint::new(); 25];

        for point in &mut result {
            point.frame = file.read_u16().unwrap();
            point.value = file.read_u16().unwrap();
        }
        result
    }

    #[allow(dead_code)]
    fn read_samples<R: Read + Seek>(file: &mut R, sample_ptrs: &Vec<u32>) -> SimpleResult<Vec<Sample>> {
        let mut samples: Vec<Sample> = vec![];
        samples.reserve_exact(sample_ptrs.len());

        for ptr in sample_ptrs {
            file.seek(SeekFrom::Start(*ptr as u64))?;
            let id = file.read_string(4);
            if id != "IMPS" {
                return Err(SimpleError::new("Error in reading IT sample - wrong ID"));
            }

            let _dos_name = file.read_string(12);
            let _zero = file.read_u8()?;
            let _global_volume = file.read_u8()?;
            let flags = file.read_u8()?;
            let default_volume = file.read_u8()?;
            let name = file.read_string(26);
            let convert = file.read_u8()?;
            let default_panning = file.read_u8()?;
            let length = file.read_u32()?;
            let loop_start = file.read_u32()?;
            let loop_end = file.read_u32()?;
            let c5speed = file.read_u32()?;
            let _sustain_loop_start = file.read_u32()?;
            let _sustain_loop_end = file.read_u32()?;
            let sample_data_ptr = file.read_u32()?;
            let _vibrato_speed = file.read_u8()?;
            let _vibrato_depth = file.read_u8()?;
            let _vibrato_waveform = file.read_u8()?;
            let _vibrato_rate = file.read_u8()?;

            let bitness = if (flags & 2) == 2 { 16 } else { 8 };
            let loop_type = if (flags & 16) == 16 {
                if (flags & 64) == 64 { LoopType::PingPongLoop } else { LoopType::ForwardLoop }
            } else {
                LoopType::NoLoop
            };

            let (finetune, relative_note) = crate::module_reader::c2spd_to_finetune_relnote(c5speed);

            let mut sample = Sample {
                length,
                loop_start,
                loop_end,
                loop_len: if loop_end > loop_start { loop_end - loop_start } else { 0 },
                volume: default_volume,
                finetune,
                loop_type,
                bitness,
                panning: if (default_panning & 128) == 128 { 128 } else { default_panning },
                relative_note,
                name: name.trim().to_string(),
                data: vec![],
            };

            if sample_data_ptr != 0 && length > 0 {
                let current_pos = file.stream_position()?;
                file.seek(SeekFrom::Start(sample_data_ptr as u64))?;
                
                if (flags & 8) == 8 { // Compressed
                    if bitness == 8 {
                        // IT compression blocks are not easily size-predictable without reading headers.
                        // For simplicity, we'll read the whole block-based structure.
                        // Actually, ITTECH says each block has a 16-bit length.
                        let mut decompressed_data = vec![0i8; length as usize];
                        let mut decomp_pos = 0usize;
                        while decomp_pos < length as usize {
                            let block_len = file.read_u16()?;
                            let mut block_data = vec![0u8; block_len as usize];
                            file.read_exact(&mut block_data)?;
                            let todo = std::cmp::min(0x8000, length as usize - decomp_pos);
                            it_compression::decompress_it_block_8bit(&block_data, &mut decompressed_data[decomp_pos..decomp_pos+todo])?;
                            decomp_pos += todo;
                        }
                        sample.data = Sample::upsamplei16(Sample::upsamplei8(decompressed_data));
                    } else {
                        let mut decompressed_data = vec![0i16; length as usize];
                        let mut decomp_pos = 0usize;
                        while decomp_pos < length as usize {
                            let block_len = file.read_u16()?;
                            let mut block_data = vec![0u8; block_len as usize];
                            file.read_exact(&mut block_data)?;
                            let todo = std::cmp::min(0x4000, length as usize - decomp_pos);
                            it_compression::decompress_it_block_16bit(&block_data, &mut decompressed_data[decomp_pos..decomp_pos+todo])?;
                            decomp_pos += todo;
                        }
                        sample.data = Sample::upsamplei16(decompressed_data);
                    }
                } else { // Uncompressed
                    if bitness == 8 {
                        let data = file.read_i8_vec(length as usize)?;
                        sample.data = Sample::upsamplei16(Sample::upsamplei8(if (convert & 1) == 1 { data } else { Sample::unpack_i8(data) }));
                    } else {
                        let data = file.read_i16_vec(length as usize)?;
                        sample.data = Sample::upsamplei16(if (convert & 1) == 1 { data } else { Sample::unpack_i16(data) });
                    }
                }
                file.seek(SeekFrom::Start(current_pos))?;
            }

            sample.setup_loops_and_padding();
            samples.push(sample);
        }

        Ok(samples)
    }

    fn read_instruments<R: Read + Seek>(file: &mut R, instrument_ptrs: &Vec<u32>) -> SimpleResult<Vec<Instrument>> {
        let mut instruments: Vec<Instrument> = vec![];
        let instrument_count = instrument_ptrs.len();

        instruments.reserve_exact(instrument_count + 1);
        instruments.push(Instrument::new());

        for instrument_ptr in instrument_ptrs {
            file.seek(SeekFrom::Start(*instrument_ptr as u64))?;
            let id = file.read_string(4);
            if id != "IMPI" {
                return Err(SimpleError::new("Error in reading IT instrument - wrong ID"));
            }

            let _dos_name = file.read_string(12);
            let _zero = file.read_u8()?;
            let nna = file.read_u8()?;
            let dct = file.read_u8()?;
            let dca = file.read_u8()?;
            let fade_out = file.read_u16()?;
            let _pps = file.read_i8()?;
            let _ppc = file.read_u8()?;
            let global_vol = file.read_u8()?;
            let _dfp = file.read_u8()?;
            let _rvv = file.read_u8()?;
            let _rpv = file.read_u8()?;
            let _version = file.read_u16()?;
            let _nos = file.read_u8()?;
            let _x = file.read_u8()?;
            let name = file.read_string(26);
            let _ifc = file.read_u8()?;
            let _ifr = file.read_u8()?;
            let _mc = file.read_u8()?;
            let _mp = file.read_u8()?;
            let _mb = file.read_u16()?;
            
            let mut sample_indexes = vec![(0u8, 0u8); 120];
            for i in 0..120 {
                let note = file.read_u8()?;
                let sample_idx = file.read_u8()?;
                sample_indexes[i] = (note, sample_idx);
            }

            let mut envelopes = vec![];
            for _ in 0..3 {
                let flags = file.read_u8()?;
                let size = file.read_u8()?;
                let loop_start_point = file.read_u8()?;
                let loop_end_point = file.read_u8()?;
                let sustain_start_point = file.read_u8()?;
                let sustain_end_point = file.read_u8()?;
                
                let mut points = [EnvelopePoint::new(); 25];
                for i in 0..25 {
                    let value = file.read_i8()?;
                    let frame = file.read_u16()?;
                    if i < size as usize {
                        points[i] = EnvelopePoint { frame, value: value as u16 };
                    }
                }
                
                envelopes.push(Envelope::create(
                    points,
                    size,
                    sustain_start_point,
                    loop_start_point,
                    loop_end_point,
                    flags
                ));
            }

            instruments.push(Instrument {
                name: name.trim().to_string(),
                idx: 0, // Will be set later if needed
                sample_indexes,
                volume_envelope: envelopes[0],
                panning_envelope: envelopes[1],
                pitch_envelope: envelopes[2],
                vibrato_envelope: VibratoEnvelope::new(), // To be implemented
                volume_fadeout: fade_out,
                nna,
                dct,
                dca,
                samples: vec![], // Will be set later
            });
        }
        Ok(instruments)
    }

    fn truncate_patterns(pattern_order: &mut Vec<u8>) {
        let mut write_pos = 0;
        for i in 0..pattern_order.len() {
            if pattern_order[i] < 254 {
                pattern_order[write_pos] = pattern_order[i];
                write_pos += 1;
            } else if pattern_order[i] == 255 {
                break;
            }
        }
        pattern_order.truncate(write_pos);
    }

    fn read_it_header<R: Read + Seek>(file: &mut R) -> SimpleResult<SongData>
    {
        let id = file.read_string(4);
        if id != "IMPM" {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "Not an IT module")));
        }

        let name = file.read_string(26);
        let _ = file.read_u16()?;
        let order_count = file.read_u16()?;
        let instrument_count = file.read_u16()?;
        let sample_count = file.read_u16()?;
        let pattern_count = file.read_u16()?;
        let _ = file.read_u16()?;
        let compatible_with_version = file.read_u16()?;

        if compatible_with_version < 0x200 {
            return Err(SimpleError::from(io::Error::new(io::ErrorKind::Other, "IT module is not in a compatible format")));
        }

        let flags = file.read_u16()?;
        let special = file.read_u16()?;
        let _ = file.read_u8()?;
        let _ = file.read_u8()?;
        let speed = file.read_u8()?;
        let tempo = file.read_u8()?;
        let _ = file.read_u8()?;
        let _ = file.read_u8()?;
        let message_length = file.read_u16()?;
        let message_offset = file.read_u32()?;
        let _ = file.read_u32()?;
        let mut initial_channel_panning = [32u8; 64];
        let panning_bytes = file.read_bytes(64)?;
        for i in 0..64 {
            let p = panning_bytes[i];
            if p <= 64 {
                initial_channel_panning[i] = p;
            } else if p == 100 { // Surround
                initial_channel_panning[i] = 32; // Center for now
            } else if p >= 128 { // Mute
                // initial_channel_panning[i] = 32; 
            }
        }

        let mut initial_channel_volume = [64u8; 64];
        let volume_bytes = file.read_bytes(64)?;
        for i in 0..64 {
            let v = volume_bytes[i];
            if v <= 64 {
                initial_channel_volume[i] = v;
            }
        }

        let mut pattern_order = file.read_bytes(order_count as usize)?;
        truncate_patterns(&mut pattern_order);

        let instrument_ptrs = file.read_u32_vec(instrument_count as usize)?;
        let sample_ptrs = file.read_u32_vec(sample_count as usize)?;
        let pattern_ptrs = file.read_u32_vec(pattern_count as usize)?;

        let mut instruments = read_instruments(file, &instrument_ptrs)?;
        let mut samples = read_samples(file, &sample_ptrs)?;

        let mut patterns: Vec<Patterns> = vec![];
        for ptr in pattern_ptrs {
            if ptr == 0 {
                patterns.push(Patterns::new(64, 64));
            } else {
                file.seek(SeekFrom::Start(ptr as u64))?;
                patterns.extend(read_patterns(file, 1, 64)?);
            }
        }

        // Assign samples to instruments based on keyboard mapping if instruments are used
        if (flags & 4) == 4 {
            for instrument in instruments.iter_mut().skip(1) {
                instrument.samples = samples.clone(); // In IT, samples are shared across instruments.
            }
        } else {
            // "Only Samples" mode. We need to create dummy instruments pointing to each sample.
            // IT sample numbers are 1-based.
            instruments = vec![crate::instrument::Instrument::new()]; // null instrument
            for (i, sample) in samples.iter().enumerate() {
                let mut inst = crate::instrument::Instrument::new();
                inst.name = sample.name.clone();
                // Map all 120 notes to this sample (which will be at index 0 in this dummy instrument's samples list)
                for note_idx in 0..120 {
                    inst.sample_indexes[note_idx] = (note_idx as u8 + 1, 1u8); // sample_num 1 means index 0
                }
                inst.samples = vec![sample.clone()];
                instruments.push(inst);
            }
        }

        let mut song_message = String::new();
        if (special & 1) == 1 && message_offset > 0 {
            let _ = file.seek(SeekFrom::Start(message_offset as u64));
            song_message = file.read_string(message_length as usize);
        }

        Ok(SongData {
            id: id.trim().to_string(),
            name: name.trim().to_string(),
            song_type: crate::module_reader::SongType::IT,
            tracker_name: "Impulse Tracker".to_string(),
            song_length: pattern_order.len() as u16,
            restart_position: 0,
            channel_count: 64,
            patterns,
            instrument_count,
            frequency_type: if (flags & 8) == 8 { crate::module_reader::FrequencyType::LINEAR } else { crate::module_reader::FrequencyType::AMIGA },
            tempo: speed as u16,
            bpm: tempo as u16,
            pattern_order,
            instruments,
            use_amiga: (flags & 8) != 8,
            song_message,
            initial_channel_volume,
            initial_channel_panning,
        })
    }

    pub(crate) fn read_it<R: Read + Seek>(file: &mut R) -> SimpleResult<SongData> {
        file.seek(SeekFrom::Start(0))?;
        read_it_header(file)
    }
