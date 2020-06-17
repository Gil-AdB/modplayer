#![feature(generators, generator_trait)]
#![feature(vec_drain_as_slice)]
#![feature(slice_fill)]

extern crate portaudio;

use std::borrow::{Borrow, BorrowMut};
use std::cell::{RefCell, UnsafeCell};
use std::f32::consts::PI;
use std::fmt;
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom, Write, stdout};
use std::iter::FromIterator;
use std::num::Wrapping;
use std::ops::{Deref, DerefMut, Generator, GeneratorState};
use std::os::raw::*;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use byteorder::{ByteOrder, LittleEndian, ReadBytesExt};
use portaudio as pa;

use crate::LoopType::{ForwardLoop, NoLoop, PingPongLoop};
use crossbeam::thread;
use portaudio::{Error, PortAudio};
use std::cmp::min;
use std::fmt::Debug;
use std::ptr::null;
use std::slice::SliceIndex;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::mpsc::channel;
use std::thread::sleep;
use std::time;
// use term::stdout;

use crossterm::{
    ExecutableCommand, execute,
    cursor::{MoveTo, RestorePosition, SavePosition}
};
use getch::Getch;

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

fn is_note_valid(note: u8) -> bool {
    note > 0 && note < 97
}


impl Pattern {
    const notes: [&'static str;12] = ["C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-"];

    fn get_note(&self) -> String {
        if self.note == 97 || self.note == 0 { "   ".to_string() } else {
            format!("{}{}", Pattern::notes[((self.note - 1) % 12) as usize], (((self.note - 1) / 12) + '0' as u8) as char )
        }
    }

    fn is_porta_to_note(&self) -> bool {
        self.effect == 0x3
    }

    fn is_note_delay(&self) -> bool {
        self.effect == 0xe && self.get_x() == 0xd
    }

    fn get_x(&self) -> u8 {
        self.effect_param >> 4
    }

    fn get_y(&self) -> u8 {
        self.effect_param & 0xf
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
    for c in &mut buf {
        if *c < 32 || *c > 127 {
            *c = 32;
        }
    }
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

#[derive(Clone, Debug, PartialEq, Eq)]
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
    frame:                   u16,
    value:                   u16
}

type EnvelopePoints = [EnvelopePoint;12];

fn read_envelope<R: Read>(file: &mut R) -> EnvelopePoints {
    let mut result = [EnvelopePoint { frame: 0, value: 0 }; 12];

    for mut point in &mut result {
        point.frame = read_u16(file);
        point.value = read_u16(file);
    }
    result
}

#[derive(Debug)]
struct Envelope {
    points:             EnvelopePoints,
    size:               u8,
    sustain_point:      u8,
    loop_start_point:   u8,
    loop_end_point:     u8,
    on:                 bool,
    sustain:            bool,
    has_loop:           bool,
}

impl Envelope {
    fn new() -> Envelope {
        Envelope{
            points: [EnvelopePoint { frame: 0, value: 0 }; 12],
            size: 0,
            sustain_point: 0,
            loop_start_point: 0,
            loop_end_point: 0,
            on: false,
            sustain: false,
            has_loop: false
        }
    }
}

#[derive(Debug)]
struct Instrument {
    name:                           String,
    sample_indexes:                 Vec<u8>,
    volume_envelope:                Envelope,
    panning_envelope:               Envelope,
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
            volume_envelope: Envelope::new(),
            panning_envelope: Envelope::new(),
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
                volume_envelope: Envelope{
                    points: volume_envelope,
                    size: volume_points,
                    sustain_point: volume_sustain_point,
                    loop_start_point: volume_loop_start_point,
                    loop_end_point: volume_loop_end_point,
                    on: (volume_type & 1) == 1,
                    sustain: (volume_type & 2) == 2,
                    has_loop: (volume_type & 4) == 4,
                },
                panning_envelope: Envelope{
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

const AUDIO_BUF_FRAMES: usize = 1024;
const AUDIO_BUF_SIZE: usize = AUDIO_BUF_FRAMES *2;
const AUDIO_NUM_BUFFERS: usize = 3;

struct ConsumerProducerQueue {
    full_count: Semaphore,
    empty_count: Semaphore,
    buf: [[f32; AUDIO_BUF_SIZE]; AUDIO_NUM_BUFFERS],
    front: usize,
    back: usize,
}

#[derive(Clone)]
struct CPQHolder {
    q: Arc<AtomicPtr<ConsumerProducerQueue>>,
}

impl CPQHolder {
    fn get(&mut self) -> &mut ConsumerProducerQueue {
        unsafe{&mut *self.q.load(Ordering::Acquire)}
    }
}


impl ConsumerProducerQueue {
    fn new() -> CPQHolder {
        let mut q = Box::new(ConsumerProducerQueue {
            full_count: Semaphore::new(0),
            empty_count: Semaphore::new(AUDIO_NUM_BUFFERS - 1),
            // consumer: Arc::new((Mutex::new(false), Default::default())),
            buf: [[0.0f32; AUDIO_BUF_SIZE]; AUDIO_NUM_BUFFERS],
            front: 0,
            back: 0
        });
        CPQHolder{q: Arc::new(AtomicPtr::new(Box::into_raw(q) as *mut ConsumerProducerQueue))}
    }

        fn produce<F: FnMut(&mut[f32; AUDIO_BUF_SIZE])>(&mut self, mut f: F) {
        loop {
            self.empty_count.wait();
            let my_buf = &mut self.buf[self.front];
            self.front = (self.front + 1) % AUDIO_NUM_BUFFERS;
            f(my_buf);
            self.full_count.signal()
        }
    }

    fn consume<F: FnMut(&[f32; AUDIO_BUF_SIZE])>(&mut self, mut f: F) {
        self.full_count.wait();
        let my_buf = &self.buf[self.back];
        self.back = (self.back + 1) % AUDIO_NUM_BUFFERS;
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
struct PortaToNoteState {
    target_note:                Note,
    speed:                      u8,
    target_frequency:            f32
}


impl PortaToNoteState {
    fn new() -> PortaToNoteState {
        PortaToNoteState {
            target_note: Note{
                note: 0.0,
                finetune: 0.0,
                period: 0.0
            },
            target_frequency: 0.0,
            speed: 0
        }
    }
}

enum WaveControl {
    SIN,
    RAMP,
    SQUARE,
}

#[derive(Clone,Copy,Debug)]
struct VibratoState {
    speed:  i8,
    depth:  i8,
    pos:    i8,
}

impl VibratoState {

    const SIN_TABLE: [i32; 32] =
        [0,   24,   49,  74,  97, 120, 141, 161,
         180, 197, 212, 224, 235, 244, 250, 253,
         255, 253, 250, 244, 235, 224, 212, 197,
         180, 161, 141, 120,  97,  74,  49,  24];


    fn new() -> VibratoState {
        VibratoState {
            speed: 0,
            depth: 0,
            pos: 0
        }
    }

    fn set_speed(&mut self, speed: i8) {
        if speed != 0 {
            self.speed = speed;
        }
    }


    fn get_frequency_shift(&mut self, wave_control: WaveControl) -> i32 {
        let mut delta = 0;
        match wave_control {
            WaveControl::SIN => { delta = (VibratoState::SIN_TABLE[(self.pos & 31) as usize] * self.depth as i32); }
            WaveControl::RAMP => {
                let mut temp:i32 = ((self.pos & 31) * 8) as i32;
                delta = if self.pos < 0 { 255 - temp } else { temp } as i32
            }
            WaveControl::SQUARE => { delta = 255; }
        }
        ((delta / 128) * 4 * (if self.pos < 0 { -1 } else { 1 })) as i32
    }

    fn next_tick(&mut self) {
        self.pos += self.speed;
        if self.pos > 31 { self.pos -= 64; }
    }
}

#[derive(Clone,Copy,Debug)]
struct EnvelopeState {
    frame:      u16,
    sustained:  bool,
    looped:     bool,
    idx:        usize,
    // instrument:         &'a Instrument,
}

impl EnvelopeState {
    fn new() -> EnvelopeState {
        EnvelopeState { frame: 0, sustained: false, looped: false, idx: 0 }
    }

    fn handle(&mut self, env: &Envelope, channel_sustained: bool, default: u16) -> u16 {
        if !env.on || env.size < 1 { return default;} // bail out

        if env.size == 1 { // whatever
            return env.points[0].value
        }

        // set sustained if channel is sustained, we have a sustain point and we reached the sustain point
        if !self.looped && !self.sustained && env.sustain && channel_sustained && self.frame == env.points[env.sustain_point as usize].frame {
            self.sustained = true;
        }

        // if sustain was triggered, it's sticky
        if self.sustained {
            return env.points[self.idx].value
        }

        // loop
        if env.has_loop && self.frame == env.points[env.loop_end_point as usize].frame {
            self.looped = true;
            self.idx = env.loop_start_point as usize;
            self.frame = env.points[self.idx].frame;
        }

        // reached the end
        if self.idx == (env.size - 2) as usize && self.frame == env.points[self.idx + 1].frame {
            return env.points[self.idx + 1].frame
        }

        let retval = EnvelopeState::lerp(self.frame, &env.points[self.idx], &env.points[self.idx + 1]);

        if self.frame < env.points[(env.size - 1) as usize].frame {
            self.frame += 1;
            if self.idx < (env.size - 2) as usize && self.frame == env.points[self.idx + 1].frame {
                self.idx += 1;
            }
        }

        retval
    }

        // pre: e.on && e.size > 0
//    default: panning envelope: middle (0x80?), volume envelope - max (0x40)
        fn handle1(&mut self, e: &Envelope, sustained: bool, default: u16) -> u16 {
        // fn handle(&mut self, e: &Envelope, channel_sustained: bool) -> u16 {
        if !e.on || e.size < 1 { return default;} // bail out
        if e.size == 1 {
            // if !e.sustain {return default;}
            return e.points[0].value;
        }

        if e.has_loop && self.frame >= e.points[e.loop_end_point as usize].frame as u32 as u16 {
            self.frame = e.points[e.loop_start_point as usize].frame as u32 as u16
        }

        let mut idx:usize = 0;
        loop {
            if idx >= e.size as usize - 2 { break; }
            if e.points[idx].frame as u32 <= self.frame as u32 && e.points[idx+1].frame as u32 >= self.frame as u32 {
                break;
            }
            idx += 1;
        }

        // if sustained && (e.sustain && self.idx == e.sustain_point as u32) && self.idx == e.size as u32 {
        //     return e.points[self.idx as usize].value;
        // }

        let retval = EnvelopeState::lerp(self.frame as u16, &e.points[idx as usize], &e.points[(idx + 1) as usize]);


        if !sustained || !e.sustain || self.frame != e.points[e.sustain_point as usize].frame as u32 as u16 {
            self.frame += 1;
        }

        retval
    }

    fn lerp(frame: u16, e1: &EnvelopePoint, e2: &EnvelopePoint) -> u16 {
        if frame == e1.frame {
            return e1.value;
        } else if frame == e2.frame {
            return e2.value;
        }

        let p = (frame - e1.frame) as f32/ (e2.frame - e1.frame) as f32;
        ((e2.value as f32 - e1.value as f32) * p  + e1.value as f32) as u16
    }

    // fn next_tick(& mut self) {
    //     if self.instrument.volume_points > 0 {
    //         self.volume_frame += 1;
    //         if self.volume_frame >= self.instrument.volume_envelope[self.volume_idx + 1].frame_number {
    //             self.volume_idx +=1;
    //             if self.volume_idx > self.instrument.volume_loop_end_point as u32 {
    //                 self.volume_idx = self.instrument.volume_loop_start_point as u32;
    //                 self.volume_frame = self.instrument.volume_envelope[self.volume_idx];
    //
    //             }
    //
    //         }
    //     }
    //     // if self.volume_frame > self.instrument.panning_loop_end_point {
    //     //
    //     // }
    //     // self.panning_frame  += 1;
    // }

    fn reset(& mut self, pos: u16, env: &Envelope) {
        self.frame = pos;
        self.sustained = false;

        if self.frame > env.points[(env.size - 1) as usize].frame {self.frame = env.points[(env.size - 1) as usize].frame;}
        let mut idx:usize = 0;
        loop {
            if idx >= env.size as usize - 2 { break; }
            if env.points[idx].frame <= self.frame && env.points[idx+1].frame >= self.frame {
                break;
            }
            idx += 1;
        }
        self.idx = idx;
    }
}

#[derive(Clone,Copy,Debug)]
struct Note {
    note:       f32,
    finetune:   f32,
    period:     f32
}

impl Note {
    fn set_note(&mut self, note: f32, finetune: f32) {
        self.note = note;
        self.finetune = finetune;
        self.period = 10.0 * 12.0 * 16.0 * 4.0 - (self.note * 16.0 * 4.0)  - self.finetune / 2.0
    }


    fn frequency(&self, period_shift: f32) -> f32 {
        //let period = 10.0 * 12.0 * 16.0 * 4.0 - ((self.note - period_shift) * 16.0 * 4.0)  - self.finetune / 2.0;
        let period = self.period - (period_shift * 16.0 * 4.0);
        let two = 2.0f32;
        let freq = 8363.0 * two.powf((6.0 * 12.0 * 16.0 * 4.0 - period) / (12.0 * 16.0 * 4.0));
        return freq
    }

    const NOTES: [&'static str;12] = ["C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-"];

    fn to_string(&self) -> String {
        if self.note == 97.0 || self.note == 0.0 { "   ".to_string() } else {
            format!("{}{}", Self::NOTES[((self.note as u8 - 1) % 12) as usize], (((self.note as u8 - 1) / 12) + '0' as u8) as char )
        }
    }


}


#[derive(Clone,Copy,Debug)]
struct ChannelData<'a> {
    instrument:                 &'a Instrument,
    sample:                     &'a Sample,
    // note:                       u8,
    // period:                     f32,
    note:                       Note,
    frequency:                  f32,
    du:                         f32,
    volume:                     u8,
    output_volume:              f32,
    sample_position:            f32,
    loop_started:               bool,
    ping:                       bool,
    volume_envelope_state:      EnvelopeState,
    panning_envelope_state:     EnvelopeState,
    fadeout_vol:                u16,
    sustained:                  bool,
    vibrato_state:              VibratoState,
    frequency_shift:            f32,
    period_shift:               f32,
    on:                         bool,
    last_porta_up:              f32,
    last_porta_down:            f32,
    porta_to_note:              PortaToNoteState,
}

impl ChannelData<'_> {
    fn set_note(&mut self, note: i16, fine_tune: i32) {
        self.note.set_note(note as f32, fine_tune as f32);
        self.frequency_shift = 0.0;
        self.period_shift = 0.0;
        self.frequency = self.note.frequency(self.period_shift);
    }

    fn update_frequency(&mut self, rate: f32) {
        // self.frequency = self.note.frequency(self.period_shift) + self.frequency_shift;
        self.frequency = self.note.frequency(self.period_shift) + self.frequency_shift;
        self.du = self.frequency / rate;
    }

    fn reset_envelopes(&mut self) {
        self.volume_envelope_state.reset(0, &self.instrument.volume_envelope);
        self.panning_envelope_state.reset(0, &self.instrument.panning_envelope);
        self.fadeout_vol = 65535;
    }


    fn trigger_note(&mut self, pattern: &Pattern, rate: f32) {
        let mut reset_envelope = false;
        if pattern.note >= 1 && pattern.note < 97 { // trigger note

            let tone = match self.get_tone(pattern) {
                Ok(p) => p,
                Err(e) => return,
            };

            self.on = true;
            self.sample_position = 0.0;
            self.loop_started = false;
            self.ping = true;
            self.frequency_shift = 0.0;
            self.period_shift = 0.0;

            // println!("channel: {}, note: {}, relative: {}, real: {}, vol: {}", i, pattern.note, channel.sample.relative_note, pattern.note as i8 + channel.sample.relative_note, channel.volume);

            self.set_note(tone as i16, self.sample.finetune as i32);
            self.update_frequency(rate);
            self.sustained = true;
            reset_envelope = true;
        }

        if reset_envelope {
            self.reset_envelopes();
        }
    }

    fn get_tone(&mut self, pattern: &Pattern) -> Result<i8, bool> {
        let tone = pattern.note as i8 + self.sample.relative_note;
        if tone > 12 * 10 || tone < 0 {
            return Err(false);
        }
        Ok(tone)
    }

//     fn new(song_data : SongData) -> ChannelData {
//         ChannelData {
//             instrument: &song_data.instruments[0],
//             sample: &song_data.instruments[0].samples[0],
//             note: 0,
//             frequency: 0.0,
//             du: 0.0,
//             volume: 64,
//             output_volume: 1.0,
//             sample_position: 0.0,
//             loop_started: false,
//             volume_envelope_state: EnvelopeState::new(),
//             panning_envelope_state: EnvelopeState::new(),
//             fadeout_vol: 65535,
//             sustained: false,
//             vibrato_state: VibratoState::new(),
//             frequency_shift: 0.0,
//             on: false
//         }
//     }
}

enum Op {
}

const BUFFER_SIZE: usize = 4096;
struct Song<'a> {
    song_position:      usize,
    row:                usize,
    tick:               u32,
    rate:               f32,
    speed:              u32,
    bpm:                u32,
    volume:             u32,
    song_data:          &'a SongData,
    channels:           [ChannelData<'a>;32],
    deferred_ops:       Vec<Op>,
    internal_buffer:    Vec<f32>,
}

impl<'a> Song<'a> {
    // fn get_buffer(&mut self) -> Vec<f32> {
    //     let mut result: Vec<f32> = vec![];
    //     result.reserve_exact(BUFFER_SIZE);
    //     while result.len() < BUFFER_SIZE {
    //         if !self.internal_buffer.is_empty() {
    //             let copy_size = std::cmp::min(BUFFER_SIZE - result.len(), self.internal_buffer.len());
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

    fn get_linear_frequency(note: i16, fine_tune: i32, period_offset: i32) -> f32 {
        let period = 10.0 * 12.0 * 16.0 * 4.0 - (note * 16 * 4) as f32  - (fine_tune as f32) / 2.0 + period_offset as f32;
        let two = 2.0f32;
        let frequency = 8363.0 * two.powf((6.0 * 12.0 * 16.0 * 4.0 - period) / (12.0 * 16.0 * 4.0));
        frequency as f32
    }

    fn get_next_tick_callback(&'a mut self, buffer: Arc<AtomicPtr<[f32; AUDIO_BUF_SIZE]>>) -> impl Generator<Yield=(), Return=()> + 'a {
        move || {
            let tick_duration_in_ms = 2500.0 / self.bpm as f32;
            let tick_duration_in_frames = (tick_duration_in_ms / 1000.0 * self.rate as f32) as usize;


            let mut current_buf_position = 0;
            let mut buf = &mut unsafe { *buffer.load(Ordering::Acquire) };
            loop {
                self.process_tick();

//            self.internal_buffer.resize((tick_duration_in_frames * 2) as usize, 0.0);

                let mut current_tick_position = 0usize;

                while current_tick_position < tick_duration_in_frames {
                    let ticks_to_generate = min(tick_duration_in_frames, AUDIO_BUF_FRAMES - current_buf_position);

                    crossterm::execute!(stdout(), MoveTo(1,1));
                    self.output_channels(current_buf_position, buf, ticks_to_generate);
                    current_tick_position += ticks_to_generate;
                    current_buf_position += ticks_to_generate;
                    // println!("tick: {}, buf: {}, row: {}", self.tick, current_buf_position, self.row);
                    if current_buf_position == AUDIO_BUF_FRAMES {
                        // println!("Yielding: {}", current_buf_position);
                        yield;
                        //let temp_buf = &mut unsafe { *buffer.load(Ordering::Acquire) };
                        unsafe { buf = &mut *buffer.load(Ordering::Acquire); }
                        buf.fill(0.0);

                        current_buf_position = 0;
                    } else {
                        // println!("current_buf_position: {}", current_buf_position)
                    }

                }

                self.next_tick()
            }
        }
    }

    fn next_tick(&mut self) {
        self.tick += 1;
        if self.tick >= self.speed {
            self.row = self.row + 1;
            if self.row >= self.song_data.patterns[self.song_data.pattern_order[self.song_position as usize] as usize].rows.len() {
                self.row = 0;
                self.song_position = self.song_position + 1;
            }
            self.tick = 0;
        }
    }

    fn process_tick(&mut self) {
        let instruments = &self.song_data.instruments;

        let patterns = &self.song_data.patterns[self.song_data.pattern_order[self.song_position] as usize];
        let row = &patterns.rows[self.row];
        let first_tick = self.tick == 0;

        // if first_tick {
        //     // println!("{} {} {}", self.song_position, self.row, row);
        // }

        for (i, pattern) in row.channels.iter().enumerate() {
            // if i != 12 { continue; }
            let mut channel = &mut (self.channels[i]);

            if first_tick && pattern.is_porta_to_note() && pattern.instrument != 0 {
                channel.volume = channel.sample.volume;
            }

            if !pattern.is_porta_to_note() &&
                ((pattern.is_note_delay() && self.tick == pattern.get_y() as u32) ||
                    (!pattern.is_note_delay() && first_tick)) { // new row, set instruments


                if pattern.note == 97 { // note off
                    channel.sustained = false;
                    if !channel.instrument.volume_envelope.on {
                        channel.on = false;
                        continue;
                    }
                }

                if pattern.instrument != 0 {
                    let instrument = &instruments[pattern.instrument as usize];
                    channel.instrument = instrument;
                    channel.sample = &instrument.samples[instrument.sample_indexes[pattern.note as usize] as usize];
                    channel.volume = channel.sample.volume;
                }

                channel.frequency_shift = 0.0;
                channel.period_shift = 0.0;


                let mut reset_envelope = false;
                if pattern.instrument != 0 {
                    channel.reset_envelopes();
                }

                channel.trigger_note(pattern, self.rate);
            }


            if !first_tick && ((0xb0 <= pattern.volume && pattern.volume <= 0xbf) || pattern.effect == 0x4 || pattern.effect == 0x6) {
                channel.frequency_shift = channel.vibrato_state.get_frequency_shift(WaveControl::SIN) as f32;
                channel.update_frequency(self.rate);
            }

            match pattern.volume {
                0x10..=0x50 => { Song::set_volume(channel, first_tick, pattern.volume - 0x10); }       // set volume
                0x60..=0x6f => { Song::volume_slide(channel, first_tick, (pattern.volume & 0xf) as i8); }       // Volume slide down
                0x70..=0x7f => { Song::volume_slide(channel, first_tick, -((pattern.volume & 0xf) as i8)); }    // Volume slide up
                0x80..=0x8f => { Song::fine_volume_slide(channel, first_tick, (pattern.volume & 0xf) as i8); }   // Fine volume slide down
                0x90..=0x9f => { Song::fine_volume_slide(channel, first_tick, -((pattern.volume & 0xf) as i8)); }// Fine volume slide up
                0xa0..=0xaf => { channel.vibrato_state.speed = (pattern.volume & 0xf) as i8; }// Set vibrato speed
                0xb0..=0xbf => { if first_tick { channel.vibrato_state.set_speed((pattern.volume & 0xf) as i8); } else { channel.vibrato_state.next_tick(); } } // Vibrato
                0xc0..=0xcf => {}// Set panning
                0xd0..=0xdf => {}// Panning slide left
                0xe0..=0xef => {}// Panning slide right
                0xf0..=0xff => {Song::porta_to_note(channel, pattern.volume & 0xf, first_tick, pattern, self.rate); }// Tone porta

                _ => {}
            }


            // handle effects
            match pattern.effect {
                0x0 => {  // Arpeggio
                    if pattern.effect_param != 0 {
                        Song::arpeggio(channel, self.tick, pattern.get_x(), pattern.get_y());
                        channel.update_frequency(self.rate);
                    }
                }
                0x1 => { Song::porta_up(channel, first_tick, pattern.effect_param, self.rate); } // Porta up
                0x2 => { Song::porta_down(channel, first_tick, pattern.effect_param, self.rate); } // Porta down
                0x3 => { Song::porta_to_note(channel,pattern.effect_param, first_tick, pattern, self.rate); } // Porta to note
                0x4 => { if first_tick { channel.vibrato_state.set_speed((pattern.volume & 0xf) as i8); } else { channel.vibrato_state.next_tick(); } } // vibrato
                0xC => { Song::set_volume(channel, first_tick, pattern.effect_param); } // set volume

                _ => {}
            }

            if pattern.effect == 0xe {
                match pattern.get_x() {
                    0xd => { Song::set_volume(channel, self.tick == pattern.get_y() as u32, 0); }
                    _ => {}
                }
            }


            let mut ves = channel.volume_envelope_state;
            let envelope_volume = channel.volume_envelope_state.handle(&channel.instrument.volume_envelope, channel.sustained, 64);
            let envelope_volume1 = ves.handle(&channel.instrument.volume_envelope, channel.sustained, 64);
            if envelope_volume != envelope_volume1 {
                let banana = 1;
            }
            let envelope_panning = channel.panning_envelope_state.handle(&channel.instrument.panning_envelope, channel.sustained, 128);
            let scale = 0.8;

            // FinalVol = (FadeOutVol/65536)*(EnvelopeVol/64)*(GlobalVol/64)*(Vol/64)*Scale;
            // channel.update_frequency(self.rate);
            channel.output_volume = (channel.fadeout_vol as f32 / 65536.0) * (envelope_volume as f32 / 64.0) * (self.volume as f32 / 64.0) * (channel.volume as f32 / 64.0) * scale;
        }
//            row
    }

    fn arpeggio(channel: &mut ChannelData, tick: u32, x:u8, y: u8) {
        match tick % 3 {
            0 => {channel.period_shift = 0.0;}
            1 => {channel.period_shift = x as f32;}
            2 => {channel.period_shift = y as f32;}
            _ => {}
        }
    }


    fn set_volume(channel: &mut ChannelData, first_tick: bool, volume: u8) {
        if first_tick {
            channel.volume = if volume <= 0x40 {volume} else {0x40};
        }
    }

    fn volume_slide(channel: &mut ChannelData, first_tick: bool, volume: i8) {
        if !first_tick { Song::volume_slide_inner(channel, volume);}
    }

    fn fine_volume_slide(channel: &mut ChannelData, first_tick: bool, volume: i8) {
        if first_tick { Song::volume_slide_inner(channel, volume);}
    }

    fn volume_slide_inner(channel: &mut ChannelData, volume: i8) {
        let mut new_volume = channel.volume as i32 + volume as i32;

        new_volume = if new_volume < 0 { 0 } else { volume as i32 };
        new_volume = if new_volume > 0x40 { 0x40 } else { volume as i32 };

        channel.volume = new_volume as u8;
    }

    fn porta_to_note(channel: &mut ChannelData, speed: u8, first_tick: bool, pattern: &Pattern, rate: f32) {
        // let speed = pattern.effect_param;

        if first_tick {
            if speed != 0 {
                channel.porta_to_note.speed = speed;
            }

            if is_note_valid(pattern.note) {
                channel.porta_to_note.target_note.set_note((pattern.note as i16 + channel.sample.relative_note as i16) as f32, channel.sample.finetune as f32);
                channel.porta_to_note.target_frequency = channel.porta_to_note.target_note.frequency(0.0);
            }

        } else {
            let mut up = true;
            if channel.note.period < channel.porta_to_note.target_note.period {
                channel.note.period += channel.porta_to_note.speed as f32 * 4.0;
                up = true;

            } else if channel.note.period > channel.porta_to_note.target_note.period {
                channel.note.period -= channel.porta_to_note.speed as f32 * 4.0;
                up = false;
            }

            if up {
                if channel.note.period > channel.porta_to_note.target_note.period {
                    channel.note = channel.porta_to_note.target_note;
                    channel.period_shift = 0.0;
                    channel.frequency_shift = 0.0;
                }
            } else if channel.note.period < channel.porta_to_note.target_note.period {
                    channel.note = channel.porta_to_note.target_note;
                    channel.period_shift = 0.0;
                    channel.frequency_shift = 0.0;
                }


            channel.update_frequency(rate);
        }
    }


    fn porta_up(channel: &mut ChannelData, first_tick: bool, amount: u8, rate: f32) {
        if first_tick {
            if amount != 0 {
                channel.last_porta_up = amount as f32 * 4.0;
            }
        } else {
            channel.note.period -= channel.last_porta_up;
            if channel.note.period < 1.0 {
                channel.note.period = 1.0;
            }
            channel.update_frequency(rate);
        }
    }

    fn porta_down(channel: &mut ChannelData, first_tick: bool, amount: u8, rate: f32) {
        if first_tick {
            if amount != 0 {
                channel.last_porta_down = amount as f32 * 4.0;
            }
        } else {
            channel.note.period += channel.last_porta_down;
            if channel.note.period > 31999.0 {
                channel.note.period = 31999.0;
            }
            channel.update_frequency(rate);
        }
    }

    // fn porta_inner(frequncy_shift: i8, channel: &mut ChannelData) {
    //     channel.frequency_shift += frequency_shift;
    // }



    fn output_channels(&mut self, current_buf_position: usize, buf: &mut [f32; AUDIO_BUF_SIZE], ticks_to_generate: usize) {
        let mut  idx: u32 = 0;
        let mut cc = 0;
        for channel in &mut self.channels {
            if channel.on { cc += 1; }
        }

        // let onecc = 1.0f32;// / cc as f32;
        println!("position: {:3}, row: {:3}", self.song_position, self.row);
        println!("  on  | channel |       instrument       |  frequency  | volume  | sample_position");

        for channel in &mut self.channels {

            println!("{:5} | {:7} | {:22} | {:<11} | {:7} | {:19}, {:5}, {:7} {}",
                     channel.on, idx, channel.instrument.name.trim(), channel.frequency + channel.frequency_shift, channel.volume, channel.sample_position, channel.note.to_string(), channel.note.period, channel.period_shift);
            idx = idx + 1;
             // if idx != 8 {continue;}

            if !channel.on {
                continue;
            }


            // print!("channel: {}, instrument: {}, frequency: {}, volume: {}\n", idx, channel.instrument.name, channel.frequency, channel.volume);

            for i in 0..ticks_to_generate as usize {
                buf[(current_buf_position + i) * 2] += channel.sample.data[channel.sample_position as usize] as f32 / 32768.0 * channel.output_volume;
                buf[(current_buf_position + i) * 2 + 1] += channel.sample.data[channel.sample_position as usize] as f32 / 32768.0 * channel.output_volume;

                // if (i & 63) == 0 {print!("{}\n", channel.sample_position);}
                if channel.sample.loop_type == PingPongLoop && !channel.ping {
                    channel.sample_position -= channel.du;
                } else {
                    channel.sample_position += channel.du;
                }

                if channel.sample_position as u32 >= channel.sample.length ||
                    (channel.loop_started && channel.sample_position >= channel.sample.loop_end as f32) {
                    channel.loop_started = true;
                    match channel.sample.loop_type {
                        PingPongLoop => {
                            channel.sample_position = (channel.sample.loop_end - 1) as f32 - (channel.sample_position - channel.sample.loop_end as f32);
                            channel.ping = false;
                            // channel.sample_position = (channel.sample.loop_end - 1) as f32;
                            // channel.du = -channel.du;
                        }
                        ForwardLoop => {
                            channel.sample_position = (channel.sample_position - channel.sample.loop_end as f32) + channel.sample.loop_start as f32;
                        }
                        NoLoop => {
                            channel.on = false;
                            break;
                        }
                    }
                }

                if channel.loop_started && channel.sample_position < channel.sample.loop_start as f32 {
                    match channel.sample.loop_type {
                        PingPongLoop => {
                            channel.ping = true;
                        }
                            _ => {}
                        }
                    channel.sample_position = channel.sample.loop_start as f32 + (channel.sample.loop_start as f32 - channel.sample_position) as f32;
                }
                if channel.sample_position as u32 >= channel.sample.length {
                    channel.on = false;
                    break;
                }
            }
        }
        print!("===================================================================\n");
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
    const NUM_SECONDS: i32 = 500;
    const SAMPLE_RATE: f64 = 48_000.0;
    const FRAMES_PER_BUFFER: u32 = 4096;

    //crossterm::


    let mut song = Song {
        song_position: 0,
        row: 0,
        tick: 0,
        rate: 48000.0,
        speed: song_data.tempo as u32,
        bpm: song_data.bpm as u32,
        volume: 64,
        song_data: &song_data,
        channels: [ChannelData {
            instrument: &song_data.instruments[0],
            sample: &song_data.instruments[0].samples[0],
            note: Note{
                note: 0.0,
                finetune: 0.0,
                period: 0.0
            },
            frequency: 0.0,
            du: 0.0,
            volume: 64,
            output_volume: 1.0,
            sample_position: 0.0,
            loop_started: false,
            ping: true,
            volume_envelope_state: EnvelopeState::new(),
            panning_envelope_state: EnvelopeState::new(),
            fadeout_vol: 65535,
            sustained: false,
            vibrato_state: VibratoState::new(),
            frequency_shift: 0.0,
            period_shift: 0.0,
            on: false,
            last_porta_up: 0.0,
            last_porta_down: 0.0,
            porta_to_note: PortaToNoteState::new(),
        }; 32],
        deferred_ops: vec![],
        internal_buffer: vec![],
    };

    let mut temp_buf = [0.0f32; AUDIO_BUF_SIZE];
    let mut buf_ref = Arc::new(AtomicPtr::new(&mut temp_buf as *mut [f32; AUDIO_BUF_SIZE]));
    let mut generator = song.get_next_tick_callback(buf_ref.clone());

    thread::scope(|scope| {
        let mut q = ConsumerProducerQueue::new();
//        let mut q = Arc::new(AtomicPtr::new(q as *mut ConsumerProducerQueue));
        {
            let mut q = q.clone();
            scope.spawn(move |_| unsafe {
                let mut idx = 0;
                let mut q = q.get();


                // let mut generator = || {
                //     loop {
                //         let mut buf = &mut *buf_ref.load(Ordering::Acquire);
                //         for i in 0..AUDIO_BUF_SIZE {
                //             buf[i] = idx as f32;
                //             idx += 1;
                //         }
                //         yield;
                //     }
                //     return ();
                // };

                q.produce(|buf: &mut [f32; AUDIO_BUF_SIZE]| {
                    // println!("produce {}", AUDIO_BUF_SIZE);
                    buf_ref.store(buf as *mut [f32; AUDIO_BUF_SIZE], Ordering::Release);
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
            pa.default_output_stream_settings(CHANNELS, SAMPLE_RATE, (AUDIO_BUF_SIZE /2) as u32)?;
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
                let ofs: usize = *count.lock().unwrap() * AUDIO_BUF_SIZE as usize;

                let mut q = q.get();

                unsafe { q.consume(|buf: &[f32; AUDIO_BUF_SIZE]| { buffer.clone_from_slice(buf); }) }

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


            let getter = Getch::new();
            println!("Play for {} seconds.", NUM_SECONDS);

            loop {

                if let Ok(ch) = getter.getch() {if ch == 'q' as u8 {break};}


                pa.sleep(1_000);
            }

            stream.stop()?;
            stream.close()?;
        };
        Ok(())
    });

    println!("Test finished.");



//    println!("samples: {}", *count.lock().unwrap());
    Ok(())
}


