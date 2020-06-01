#![feature(generators, generator_trait)]
#![feature(vec_drain_as_slice)]
#![feature(slice_fill)]

extern crate portaudio;

use std::borrow::{Borrow, BorrowMut};
use std::cell::{RefCell, UnsafeCell};
use std::f32::consts::PI;
use std::fmt;
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom};
use std::iter::FromIterator;
use std::num::Wrapping;
use std::ops::{Generator, Deref, DerefMut, GeneratorState};
use std::os::raw::*;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, Condvar};

use byteorder::{ByteOrder, LittleEndian, ReadBytesExt};
use portaudio as pa;

use crate::LoopType::{ForwardLoop, NoLoop, PingPongLoop};
use std::ptr::null;
use std::slice::SliceIndex;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::thread::sleep;
use std::time;
use std::cmp::min;
use crossbeam::thread;
use portaudio::{PortAudio, Error};
use std::fmt::Debug;

#[repr(C)]
#[repr(packed)]
struct NativeXmheader {
    id:                 [c_char;17usize],
    name:               [c_char;20usize],
    sig:                c_char,
    tracker_name:       [c_char;20usize],
    ver:                c_ushort,
    header_size:        c_uint,
    song_length:        c_ushort,
    restart_position:   c_ushort,
    channel_count:      c_ushort,
    pattern_count:      c_ushort,
    instrument_count:   c_ushort,
    flags:              c_ushort,
    tempo:              c_ushort,
    bpm:                c_ushort,
    pattern_order:      [c_uchar;256usize],
}

struct XMPatternHeader {
    header_length:      c_uint,
    packing:            c_uchar,
    row_count:          c_ushort,
    packed_size:        c_ushort,
}

#[derive(Debug)]
enum SongType {
    XM
}

#[derive(Debug)]
enum FrequencyType {
    AMIGA,
    LINEAR
}

//#[derive(Debug)]
struct Pattern {
    note:           u8,
    instrument:     u8,
    volume:         u8,
    effect:         u8,
    effect_param:   u8,
}

impl Pattern {
    const notes: [&'static str;12] = ["C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-"];

    fn get_note(&self) -> String {
        if self.note == 97 || self.note == 0 { "   ".to_string() } else {
            format!("{}{}", Pattern::notes[((self.note - 1) % 12) as usize], (((self.note - 1) / 12) + '0' as u8) as char )
        }
    }
}

//impl fmt::Debug for Pattern {
//    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//        self.Display::fmt(f)
////        write!(f, "note: {} {} {} {} {}", self.get_note(), self.instrument, self.volume, self.effect, self.effect_param)
//    }
//}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {:2} {:2x} {:2x} {:2x}", self.get_note(), self.instrument, self.volume, self.effect, self.effect_param)
    }
}


struct Row {
    channels: Vec<Pattern>
}

impl fmt::Debug for Row {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for pattern in &self.channels {
            if first {first = false;} else {write!(f, "|");}
            write!(f, "{}", pattern);
        }
        Ok(())
    }
}

impl fmt::Display for Row {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for pattern in &self.channels {
            if first {first = false;} else {write!(f, "|");}
            write!(f, "{}", pattern);
        }
        Ok(())
    }
}


#[derive(Debug)]
struct Patterns {
    rows: Vec<Row>
}

#[derive(Debug)]
struct SongData {
    id:                     String,
    name:                   String,
    song_type:              SongType,
    tracker_name:           String,
    song_length:            u16,
    restart_position:       u16,
    channel_count:          u16,
    patterns:               Vec<Patterns>,
    instrument_count:       u16,
    frequency_type:         FrequencyType,
    tempo:                  u16,
    bpm:                    c_ushort,
    pattern_order:          Vec<u8>,
    instruments:            Vec<Instrument>
}

fn read_string<R: Read>(file: &mut R, size: usize) -> String {
    let mut buf = vec!(0u8; size);
    file.read_exact(&mut buf).unwrap();
    String::from_utf8_lossy(&buf).parse().unwrap()
}

fn read_u8<R: Read>(file: &mut R) -> u8 {
    let mut buf = [0u8;1];
    file.read_exact(&mut buf).unwrap();
    buf[0]
}

fn read_i8<R: Read>(file: &mut R) -> i8 {
    let mut buf = [0u8;1];
    file.read_exact(&mut buf).unwrap();
    buf[0] as i8
}

fn read_u16<R: Read>(file: &mut R) -> u16 {
    let mut buf = [0u8;2];
    file.read_exact(&mut buf).unwrap();
    u16::from_le_bytes(buf)
}

fn read_u32<R: Read>(file: &mut R) -> u32 {
    let mut buf = [0u8;4];
    file.read_exact(&mut buf).unwrap();
    u32::from_le_bytes(buf)
}

fn read_bytes<R: Read>(file: &mut R, size: usize) -> Vec<u8> {
    let mut buf = vec!(0u8; size);
    file.read_exact(&mut buf).unwrap();
    buf
}

fn read_i16_vec<R: Read>(file: &mut R, size: usize) -> Vec<i16> {
    let mut result = vec!(0i16; size);
    let mut buf = vec!(0u8; size * 2);
    file.read_exact(&mut buf).unwrap();

    LittleEndian::read_i16_into(buf.as_slice(), result.as_mut_slice());
    result
}

fn read_i8_vec<R: Read>(file: &mut R, size: usize) -> Vec<i8> {
    let mut result = vec!(0i8; size);
    let mut buf = vec!(0u8; size);
    file.read_exact(&mut buf).unwrap();

    let mut rdr = Cursor::new(buf);
    rdr.read_i8_into(result.as_mut_slice());
    result
}


fn read_patterns<R: Read>(file: &mut R, pattern_count: usize, channel_count: usize) -> Vec<Patterns> {

    let mut patterns: Vec<Patterns> = vec![];
    patterns.reserve_exact(pattern_count as usize);

    for _pattern_idx in 0..pattern_count {
        let _pattern_header_size = read_u32(file);
        let _pattern_type         = read_u8(file);
        let row_count           = read_u16(file);
        let pattern_size        = read_u16(file);

        let mut pos = 0usize;

        let mut rows: Vec<Row> = vec![];
        rows.reserve_exact(row_count as usize);
        for _row_idx in 0..row_count {
            let mut channels : Vec<Pattern> = vec![];
            channels.reserve_exact(channel_count);
            for _channel_idx in 0..channel_count {
                let flags = read_u8(file);
                channels.push(if flags & 0x80 == 0x80 {
                    pos += 1;
                    let note        = if flags &  1 == 1  {pos +=1; read_u8(file)} else {0};
                    let instrument  = if flags &  2 == 2  {pos +=1; read_u8(file)} else {0};
                    let volume      = if flags &  4 == 4  {pos +=1; read_u8(file)} else {0};
                    let effect      = if flags &  8 == 8  {pos +=1; read_u8(file)} else {0};
                    let effect_param= if flags & 16 == 16 {pos +=1; read_u8(file)} else {0};
                    Pattern{
                        note,
                        instrument,
                        volume,
                        effect,
                        effect_param
                    }
                } else {
                    let note        = flags;
                    let instrument  = read_u8(file);
                    let volume      = read_u8(file);
                    let effect      = read_u8(file);
                    let effect_param= read_u8(file);
                    pos += 5;

                    Pattern{
                        note,
                        instrument,
                        volume,
                        effect,
                        effect_param
                    }
                });
            }
            rows.push(Row{channels});
        }
        if pattern_size as usize != pos {
            panic!("size {} != pos {}", pattern_size, pos)
        }
        patterns.push(Patterns{rows})
    }

    patterns
}

#[derive(Clone, Debug)]
enum LoopType {
    NoLoop,
    ForwardLoop,
    PingPongLoop

}

impl LoopType {
    fn FromFlags(flags: u8) -> LoopType {
        match flags & 3 {
            0 => NoLoop,
            1 => ForwardLoop,
            2 => PingPongLoop,
            _ => NoLoop
        }
    }
}


#[derive(Clone, Debug)]
struct Sample {
    length:                         u32,
    loop_start:                     u32,
    loop_end:                       u32,
    loop_len:                       u32,
    volume:                         u8,
    finetune:                       i8,
    loop_type:                      LoopType,
    bitness:                        u8,
    panning:                        u8,
    relative_note:                  i8,
    name:                           String,
    data:                           Vec<i16>
}

impl Sample {

    fn new() -> Sample {
        Sample{
            length: 0,
            loop_start: 0,
            loop_end: 0,
            loop_len: 0,
            volume: 0,
            finetune: 0,
            loop_type: LoopType::NoLoop,
            bitness: 0,
            panning: 0,
            relative_note: 0,
            name: "".to_string(),
            data: vec![]
        }
    }

    fn unpack_i16(mut data: Vec<i16>) -> Vec<i16>{
        for i in 1..data.len() {
            data[i] = (Wrapping(data[i-1]) + Wrapping(data[i])).0;
        }
        data
    }

    fn unpack_i8(mut data: Vec<i8>) -> Vec<i8>{
        for i in 1..data.len() {
            data[i] = (Wrapping(data[i-1]) + Wrapping(data[i])).0;
        }
        data
    }

    fn upsample(data: Vec<i8>) -> Vec<i16> {
        let mut result = vec!(0i16;data.len());
        result.reserve_exact(data.len() as usize);
        for i in 0..data.len() {
            result[i] = (Wrapping((((data[i] as i16) + 128i16) as u16 * 0x0101u16) as u16) + Wrapping((-32768i16) as u16)).0 as i16;
        }
        result
    }


    fn ReadData<R: Read>(&mut self, file: &mut R) {
        if self.length == 0 {return;}
        if self.bitness == 8 {
            self.data = Sample::upsample(Sample::unpack_i8(read_i8_vec(file, self.length as usize)));
        } else {
            self.data = Sample::unpack_i16(read_i16_vec(file, self.length as usize));
        }
    }
}

#[derive(Debug, Copy, Clone)]
struct EnvelopePoint {
    frame_number:                   u16,
    value:                          u16
}

fn read_envelope<R: Read>(file: &mut R) -> [EnvelopePoint;12] {
    let mut result = [EnvelopePoint { frame_number: 0, value: 0 }; 12];

    for mut point in &mut result {
        point.frame_number = read_u16(file);
        point.value = read_u16(file);
    }
    result
}

#[derive(Debug)]
struct Instrument {
    name:                           String,
    sample_indexes:                 Vec<u8>,
    volume_envelope:                [EnvelopePoint;12],
    panning_envelope:               [EnvelopePoint;12],
    volume_points:                  u8,
    panning_points:                 u8,
    volume_sustain_point:           u8,
    volume_loop_start_point:        u8,
    volume_loop_end_point:          u8,
    panning_sustain_point:          u8,
    panning_loop_start_point:       u8,
    panning_loop_end_point:         u8,
    volume_type:                    u8,
    panning_type:                   u8,
    vibrato_type:                   u8,
    vibrato_sweep:                  u8,
    vibrato_depth:                  u8,
    vibrato_rate:                   u8,
    volume_fadeout:                 u16,

    samples:                        Vec<Sample>,
}

impl Instrument {
    fn new () -> Instrument {
        Instrument{
            name: "".to_string(),
            sample_indexes: vec![0u8; 96],
            volume_envelope: [EnvelopePoint { frame_number: 0, value: 0 }; 12],
            panning_envelope: [EnvelopePoint { frame_number: 0, value: 0 }; 12],
            volume_points: 0,
            panning_points: 0,
            volume_sustain_point: 0,
            volume_loop_start_point: 0,
            volume_loop_end_point: 0,
            panning_sustain_point: 0,
            panning_loop_start_point: 0,
            panning_loop_end_point: 0,
            volume_type: 0,
            panning_type: 0,
            vibrato_type: 0,
            vibrato_sweep: 0,
            vibrato_depth: 0,
            vibrato_rate: 0,
            volume_fadeout: 0,
            samples: vec![Sample::new(); 1]
        }
    }
}

fn read_samples<R: Read>(file: &mut R, sample_count: usize) -> Vec<Sample> {
    let mut samples: Vec<Sample> = vec![];
    samples.reserve_exact(sample_count as usize);

    for sample_idx in 0..sample_count {
        println!("Reading sample #{} of {}", sample_idx, sample_count);

        let mut length = read_u32(file);
        let mut loop_start = read_u32(file);
        let mut loop_len = read_u32(file);
        let volume = read_u8(file);
        let finetune = read_i8(file);
        let flags = read_u8(file);
        let panning = read_u8(file);
        let relative_note = read_i8(file);
        let _reserved = read_u8(file);
        let name = read_string(file, 22);

        let bitness = if (flags & 16) == 16 {16} else {8};
        if bitness == 16 { // length is in bits
            length      /= 2;
            loop_start  /= 2;
            loop_len    /= 2;
        }

        let loop_type = LoopType::FromFlags(flags);
        match loop_type {
            NoLoop => {
                loop_start = 0;
                loop_len = length;
            }
            _ => {}
        }

        samples.push(Sample{
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
        sample.ReadData(file);
    }

    samples
}

fn read_instruments<R: Read + Seek>(file: &mut R, instrument_count: usize) -> Vec<Instrument> {

    let mut instruments: Vec<Instrument> = vec![];

    // Instruments are one based, go figure. We'll add an empty instrument as sample 0.
    instruments.reserve_exact(instrument_count + 1 as usize);

    instruments.push(Instrument::new());

    for _instrument_idx in 0..instrument_count {

        let instrument_pos = file.seek(SeekFrom::Current(0)).unwrap();
        let header_size          = read_u32(file);
        let name               = read_string(file, 22);
        let _instrument_type           = read_u8(file);
        let sample_count         = read_u16(file);



        if sample_count > 0 {
            let _sample_size                          = read_u32(file);
            let sample_indexes              = read_bytes(file, 96);
            let volume_envelope     = read_envelope(file);
            let panning_envelope    = read_envelope(file);
            let volume_points                    = read_u8(file);
            let panning_points                   = read_u8(file);
            let volume_sustain_point             = read_u8(file);
            let volume_loop_start_point          = read_u8(file);
            let volume_loop_end_point            = read_u8(file);
            let panning_sustain_point            = read_u8(file);
            let panning_loop_start_point         = read_u8(file);
            let panning_loop_end_point           = read_u8(file);
            let volume_type                      = read_u8(file);
            let panning_type                     = read_u8(file);
            let vibrato_type                     = read_u8(file);
            let vibrato_sweep                    = read_u8(file);
            let vibrato_depth                    = read_u8(file);
            let vibrato_rate                     = read_u8(file);
            let volume_fadeout                  = read_u16(file);
            let _reserved                             = read_u16(file);

            file.seek(SeekFrom::Start(instrument_pos + header_size as u64));
            instruments.push(Instrument {
                name,
                sample_indexes,
                volume_envelope,
                panning_envelope,
                volume_points,
                panning_points,
                volume_sustain_point,
                volume_loop_start_point,
                volume_loop_end_point,
                panning_sustain_point,
                panning_loop_start_point,
                panning_loop_end_point,
                volume_type,
                panning_type,
                vibrato_type,
                vibrato_sweep,
                vibrato_depth,
                vibrato_rate,
                volume_fadeout,
                samples: read_samples(file, sample_count as usize)
            });
        } else {
            file.seek(SeekFrom::Start(instrument_pos + header_size as u64));
            instruments.push(Instrument::new());
        }
    }
    instruments

}


fn read_xm_header<R: Read + Seek>(mut file: &mut R) -> SongData
{
    let id = read_string(&mut file, 17);
    dbg!(&id);
    let name = read_string(&mut file, 20);
    dbg!(&name);
    let sig = read_u8(file);
    if sig != 0x1a {
        panic!("Wrong Format!")
    }

    let tracker_name= read_string(file, 20);
    dbg!(&tracker_name);

    let ver = read_u16(file);
    dbg!(format!("{:x?}", ver));

//    dbg!(file.seek(SeekFrom::Current(0)));

    let header_size = read_u32(file);
    dbg!(header_size);

    let song_length = read_u16(file);
    dbg!(song_length);

    let restart_position = read_u16(file);
    dbg!(restart_position);

    let channel_count = read_u16(file);
    dbg!(channel_count);

    let pattern_count = read_u16(file);
    dbg!(pattern_count);

    let instrument_count = read_u16(file);
    dbg!(instrument_count);

    let flags = read_u16(file);
    dbg!(flags);

    let tempo = read_u16(file);
    dbg!(tempo);

    let bpm = read_u16(file);
    dbg!(bpm);

    let pattern_order = read_bytes(file, 256);
    dbg!(&pattern_order);

    let patterns = read_patterns(file, pattern_count as usize, channel_count as usize);

    let instruments = read_instruments(file, instrument_count as usize);

    SongData{
        id: id.trim().to_string(),
        name: name.trim().to_string(),
        song_type: SongType::XM,
        tracker_name: tracker_name.trim().to_string(),
        song_length,
        restart_position,
        channel_count,
        patterns,
        instrument_count,
        frequency_type: if (flags & 1) == 1 {FrequencyType::LINEAR} else {FrequencyType::AMIGA},
        tempo,
        bpm,
        pattern_order: Vec::from_iter(pattern_order[0..pattern_count as usize].iter().cloned()),
        instruments
    }
}

fn read_xm(path: &str) -> SongData {
    let f = File::open(path).expect("failed to open the file");
    let file_len = f.metadata().expect("Can't read file metadata").len();
    let mut file = BufReader::new(f);


    println!("file length: {}", file_len);
    if file_len < 60 {
        panic!("File is too small!")
    }

    let song_data = read_xm_header(&mut file);
 //   dbg!(song_data);

  //  dbg!(file.seek(SeekFrom::Current(0)));

    song_data
}


struct Semaphore {
    condvar: Arc<(Mutex<usize>, Condvar)>,
}

impl Semaphore {
    fn new(initial: usize) -> Semaphore {
        return Semaphore {
            condvar: Arc::new((Mutex::new(initial), Condvar::new())),
        };
    }

    fn signal(&mut self) {
        let (lock, cvar) = &*self.condvar;
        let mut count = lock.lock().unwrap();
        *count += 1;
        cvar.notify_one();
    }

    fn wait(&mut self) {
        let (lock, cvar) = &*self.condvar;
        let mut count = lock.lock().unwrap();
        while *count == 0 {
            count = cvar.wait(count).unwrap();
        }
        *count -= 1;
    }

}

const audio_buf_size: usize = 4096*2;
const audio_num_buffers: usize = 3;

struct ConsumerProducerQueue {
    full_count: Semaphore,
    empty_count: Semaphore,
    buf: [[f32; audio_buf_size]; audio_num_buffers],
    front: usize,
    back: usize,
}

impl ConsumerProducerQueue {
    fn new() -> ConsumerProducerQueue {
        ConsumerProducerQueue {
            full_count: Semaphore::new(0),
            empty_count: Semaphore::new(audio_num_buffers - 1),
            // consumer: Arc::new((Mutex::new(false), Default::default())),
            buf: [[0.0f32; audio_buf_size]; audio_num_buffers],
            front: 0,
            back: 0
        }
    }

    fn produce<F: FnMut(&mut[f32;audio_buf_size])>(&mut self, mut f: F) {
        loop {
            self.empty_count.wait();
            let my_buf = &mut self.buf[self.front];
            self.front = (self.front + 1) % audio_num_buffers;
            f(my_buf);
            self.full_count.signal()
        }
    }

    fn consume<F: FnMut(&[f32;audio_buf_size])>(&mut self, mut f: F) {
        self.full_count.wait();
        let my_buf = &self.buf[self.back];
        self.back = (self.back + 1) % audio_num_buffers;
        f(my_buf);
        self.empty_count.signal();
    }
}


unsafe fn worker(r: Arc<AtomicPtr<i32>>) {
    let moshe = r.load(Ordering::Acquire);
    for _ in 1..100000 {
        *moshe += 1;
    }
}

fn main() {
    let path = "Revival.XM";
    //let file = File::open(path).expect("failed to open the file");

    run(read_xm(path));
//    let mmap = unsafe { Mmap::map(&file).expect("failed to map the file") };
//
//    println!("File Size: {}", mmap.len());
//
//    let _header =  unsafe {&*(mmap.as_ptr() as * const XMHeader)};
//
//    let mut _pattern_offset = mem::size_of::<XMHeader>() as isize;
//    for pattern_idx in 0.._header.pattern_count {
//        let _pattern = unsafe {{&*(mmap.as_ptr().offset(_pattern_offset) as * const XMPatternHeader)}};
//        _pattern_offset = _pattern_offset + _pattern.packed_size as isize;
//    }
//
//    let _banana = 1;

}

#[derive(Clone,Copy,Debug)]
struct ChannelData<'a> {
    instrument:         &'a Instrument,
    sample:             &'a Sample,
    note:               u8,
    frequency:          f32,
    du:                 f32,
    volume:             u32,
    sample_position:    f32,
    loop_started:       bool,
    on:                 bool,
}

enum Op {
}

const buffer_size: usize = 4096;
struct Song<'a> {
    song_position:      usize,
    row:                usize,
    tick:               u32,
    rate:               f32,
    speed:              u32,
    bpm:                u32,
    song_data:          &'a SongData,
    channels:           [ChannelData<'a>;32],
    deferred_ops:       Vec<Op>,
    internal_buffer:    Vec<f32>
}

impl<'a> Song<'a> {
    // fn get_buffer(&mut self) -> Vec<f32> {
    //     let mut result: Vec<f32> = vec![];
    //     result.reserve_exact(buffer_size);
    //     while result.len() < buffer_size {
    //         if !self.internal_buffer.is_empty() {
    //             let copy_size = std::cmp::min(buffer_size - result.len(), self.internal_buffer.len());
    //             result.extend(self.internal_buffer.drain(0..copy_size));
    //         }
    //         if !self.internal_buffer.is_empty() {
    //             return result;
    //         }
    //         self.get_next_tick();
    //     }
    //
    //     return result;
    // }

    fn get_linear_frequency(note: i16, fine_tune: i32) -> f32 {
        let period = 10.0 * 12.0 * 16.0 * 4.0 - (note as f32) * 16.0 * 4.0 - (fine_tune as f32) / 2.0;
        let two = 2.0f32;
        let frequency = 8363.0 * two.powf((6.0 * 12.0 * 16.0 * 4.0 - period) / (12.0 * 16.0 * 4.0));
        frequency as f32
    }

    fn get_next_tick_callback(&'a mut self, buffer: Arc<AtomicPtr<[f32; audio_buf_size]>>) -> impl Generator<Yield=(), Return=()> + 'a {
        move || {
            let tick_duration_in_ms = 2500.0 / self.bpm as f32;
            let tick_duration_in_frames = (tick_duration_in_ms / 1000.0 * self.rate as f32) as usize;

            let instruments = &self.song_data.instruments;

            let mut current_buf_position = 0;
            let mut buf = &mut unsafe { *buffer.load(Ordering::Acquire) };
            loop {
                if self.tick == 0 { // new row, set instruments
                    let pattern = &self.song_data.patterns[self.song_data.pattern_order[self.song_position] as usize];
                    let row = &pattern.rows[self.row];

                    println!("{} {} {} {}", self.speed, self.bpm, self.row, row);
                    for (i, channel) in row.channels.iter().enumerate() {
                        let output_channel = &mut self.channels[i];
                        if channel.note == 97 {
                            output_channel.on = false;
                            continue;
                        }

                        if channel.instrument != 0 {
                            let instrument = &instruments[channel.instrument as usize];
                            output_channel.instrument = instrument;
                            output_channel.sample = &instrument.samples[instrument.sample_indexes[channel.note as usize] as usize];
                        }

                        if channel.note >= 1 && channel.note < 97 {
                            output_channel.on = true;
                            output_channel.sample_position = 0.0;
                            output_channel.loop_started = false;
                            self.channels[i].frequency = Song::get_linear_frequency((channel.note as i8 + output_channel.sample.relative_note) as i16, output_channel.sample.finetune as i32);
                            self.channels[i].du = self.rate / self.channels[i].frequency;
                        }
                    }
//            row
                } else {
                    // handle effects
                }

//            self.internal_buffer.resize((tick_duration_in_frames * 2) as usize, 0.0);

                let mut current_tick_position = 0usize;

                while current_tick_position < tick_duration_in_frames {
                    let ticks_to_generate = min(tick_duration_in_frames, audio_buf_size / 2 - current_buf_position);
                    self.output_channels(current_buf_position, buf, ticks_to_generate);
                    current_tick_position += ticks_to_generate;
                    current_buf_position += ticks_to_generate;
                    // println!("tick: {}, buf: {}, row: {}", self.tick, current_buf_position, self.row);
                    if current_buf_position == audio_buf_size / 2 {
                        yield;
                        //let temp_buf = &mut unsafe { *buffer.load(Ordering::Acquire) };
                        unsafe { buf = &mut *buffer.load(Ordering::Acquire); }
                        buf.fill(0.0);

                        current_buf_position = 0;
                    }
                }

                self.tick += 1;
                if self.tick > self.speed {
                    self.row = self.row + 1;
                    if self.row > 63 {
                        self.row = 0;
                        self.song_position = self.song_position + 1;
                    }
                    self.tick = 0;
                }
            }
        }
    }

    fn output_channels(&mut self, current_buf_position: usize, buf: &mut [f32; audio_buf_size], ticks_to_generate: usize) {
        for channel in &mut self.channels {
            if !channel.on {
                continue;
            }

            for i in 0..ticks_to_generate as usize {
                buf[(current_buf_position + i) * 2] += channel.sample.data[channel.sample_position as usize] as f32 / 32768.0;
                buf[(current_buf_position + i) * 2 + 1] += channel.sample.data[channel.sample_position as usize] as f32 / 32768.0;

                channel.sample_position += channel.du;
                if channel.sample_position as u32 >= channel.sample.length {
                    channel.loop_started = true;
                    match channel.sample.loop_type {
                        PingPongLoop => {
                            channel.sample_position = (channel.sample.loop_end-1) as f32;
                            channel.du = -channel.du;
                        }
                        ForwardLoop => {
                            channel.sample_position = channel.sample.loop_start as f32;
                        }
                        NoLoop => {
                            channel.on = false;
                        }
                    }
                    if channel.loop_started && channel.sample_position < channel.sample.loop_start as f32 {
                        // match channel.sample.loop_type {
                        //     PingPongLoop => {
                            channel.du = -channel.du;
                        // }
                        //     _ => {}
                        // }
                        channel.sample_position = channel.sample.loop_start as f32;
                    }
                    if channel.sample_position as u32 >= channel.sample.length {
                        channel.on = false;
                        break;
                    }
                }
            }
        }
    }
}

// struct DoubleBuffer {
//     buf: [Mutex<[f32; 64]>; 2],
//     producer_position: std::sync::atomic::AtomicUsize,
//     consumer_position: std::sync::atomic::AtomicUsize,
// }
//
// impl DoubleBuffer {
//     fn produce(&mut self, f: fn(&[f32;64]))  {
//         let producer_position = self.producer_position as usize;
//         // f(buf[self.buf[current_producer].lock()]);
//         // *self.producer.lock() = 1 - current_producer;
//     }
//
//     fn consume(&mut self, f: fn(&[f32;64])) {
//         let producer_position = self.producer_position as usize;
//         let consumer_buf = self.buf[1 - producer_position].lock().unwrap();
//         f(consumer_buf.borrow());
//         *producer_position.get_mut() = 1 - producer_position
//     }
// }

fn run(song_data : SongData) -> Result<(), pa::Error> {
    const CHANNELS: i32 = 2;
    const NUM_SECONDS: i32 = 100;
    const SAMPLE_RATE: f64 = 48_000.0;
    const FRAMES_PER_BUFFER: u32 = 4096;

    println!(
        "PortAudio Test: output sawtooth wave. SR = {}, BufSize = {}",
        SAMPLE_RATE, FRAMES_PER_BUFFER
    );


    let mut song = Song {
        song_position: 0,
        row: 0,
        tick: 0,
        rate: 48000.0,
        speed: song_data.tempo as u32,
        bpm: song_data.bpm as u32,
        song_data: &song_data,
        channels: [ChannelData{
            instrument: &song_data.instruments[0],
            sample: &song_data.instruments[0].samples[0],
            note: 0,
            frequency: 0.0,
            du: 0.0,
            volume: 0,
            sample_position: 0.0,
            loop_started: false,
            on: false
        }; 32],
        deferred_ops: vec![],
        internal_buffer: vec![]
    };

    let mut temp_buf = [0.0f32; audio_buf_size];
    let mut buf_ref = Arc::new(AtomicPtr::new(&mut temp_buf as *mut [f32; audio_buf_size]));
    let mut generator = song.get_next_tick_callback(buf_ref.clone());

    thread::scope(|scope| {
        let mut q = &mut ConsumerProducerQueue::new();
        let mut q = Arc::new(AtomicPtr::new(q as *mut ConsumerProducerQueue));
        {
            let mut q = q.clone();
            scope.spawn(move |_| unsafe {
                let mut idx = 0;
                let mut q = q.load(Ordering::Acquire);


                // let mut generator = || {
                //     loop {
                //         let mut buf = &mut *buf_ref.load(Ordering::Acquire);
                //         for i in 0..audio_buf_size {
                //             buf[i] = idx as f32;
                //             idx += 1;
                //         }
                //         yield;
                //     }
                //     return ();
                // };

                (*q).produce(|buf: &mut [f32; audio_buf_size]| {
                    buf_ref.store(buf as *mut [f32; audio_buf_size], Ordering::Release);
                    if let GeneratorState::Complete(_) = Pin::new(&mut generator).resume(()) { panic!("unexpected value from resume") }
                });
            });
        }


        // {
        //     let mut q = q.load(Ordering::Acquire);
        //
        //     for idx in 0..100 {
        //         unsafe { (*q).consume(|buf: &[f32; 64]| println!("consume: {}", buf[0])); }
        //     }
        // }


        let mut count = Arc::new(Mutex::new(0));

        let pa_result: Result<pa::PortAudio, pa::Error> = pa::PortAudio::new();
        let pa = match pa_result {
            Ok(p) => p,
            Err(e) => return Err(e),
        };

        let mut settings =
            pa.default_output_stream_settings(CHANNELS, SAMPLE_RATE, (audio_buf_size/2) as u32)?;
        // we won't output out of range samples so don't bother clipping them.
//    settings.flags = pa::stream_flags::CLIP_OFF;
//
        // This routine will be called by the PortAudio engine when audio is needed. It may called at
        // interrupt level on some machines so don't do anything that could mess up the system like
        // dynamic resource allocation or IO.
        let guard = {
            let count = count.clone();

            let callback = move |pa::OutputStreamCallbackArgs { buffer, frames, .. }| {
                //     println!("{}", frames);
                let mut idx: usize = 0;
                let ofs: usize = *count.lock().unwrap() * audio_buf_size as usize;

                let mut q = q.load(Ordering::Acquire);

                unsafe { (*q).consume(|buf: &[f32; audio_buf_size]| { buffer.clone_from_slice(buf); }) }

                //    for _ in 0..frames {
                //        buffer[idx] = buf[idx];
                //        buffer[idx + 1] = buf[idx + 1];
                //        idx += 2;
                // //       println!("{}", temp);
                //    }
                *count.lock().unwrap() += 1;
                pa::Continue
            };
            let mut stream = pa.open_non_blocking_stream(settings, callback)?;

            stream.start()?;

            println!("Play for {} seconds.", NUM_SECONDS);
            pa.sleep(NUM_SECONDS * 1_000);

            stream.stop()?;
            stream.close()?;
        };
        Ok(())
    });

    println!("Test finished.");



//    println!("samples: {}", *count.lock().unwrap());
    Ok(())
}


