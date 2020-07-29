#![feature(generators, generator_trait)]
#![feature(async_closure)]

use std::ops::{Generator, GeneratorState};
use std::pin::Pin;
use std::sync::{Arc, mpsc};
use std::sync::atomic::{AtomicPtr, Ordering, AtomicBool};
use std::sync::mpsc::{Receiver, Sender};

use crossbeam::thread;
use getch::Getch;

use xmplayer::producer_consumer_queue::{AUDIO_BUF_SIZE, ProducerConsumerQueue, AUDIO_BUF_FRAMES, PCQHolder};
use xmplayer::song::{Song, PlaybackCmd, PlayData};
use xmplayer::xm_reader::{read_xm, SongData, print_xm};
use std::env;
use std::sync::atomic::Ordering::Release;
use std::time::{Duration, SystemTime};
use std::io::{Read, ErrorKind, Error};
use std::thread::sleep;
use crossterm::cursor::{MoveTo, Show, Hide};
use std::io::{stdout, Write};
use xmplayer::TripleBuffer::{TripleBuffer, State};
#[cfg(feature="sdl2-feature")] use sdl2::audio::{AudioSpecDesired, AudioCallback};

use xmplayer::TripleBuffer::State::STATE_NO_CHANGE;

#[cfg(feature="portaudio-feature")] extern crate portaudio;
#[cfg(feature="portaudio-feature")] use portaudio as pa;

#[derive(Copy, Clone)]
struct RGB {
    R: u8,
    G: u8,
    B: u8,
}

struct Display{}

impl Display {
    fn color(color: RGB, str: &str) -> String {
        format!("\x1b[38;2;{};{};{}m{}", color.R, color.G, color.B, str)
    }

    fn range(pos: u32, start: u32, end: u32, width: usize) -> String {
        let mut result: String = String::from("");
        let mut indicator_pos = ((pos - start) as f32 / (end - start) as f32 * (width) as f32) as usize;
        if indicator_pos > width {
            indicator_pos = width;
        }
        for i in 0..indicator_pos {
            result += "-";
        }
        result += "=";
        for i in indicator_pos + 1..(width + 1) as usize {
            result += "-";
        }
        result
    }

    fn range_with_color(pos: u32, start: u32, end: u32, width: usize, colors: &[RGB]) -> String {
        let mut result: String = String::from("");
        if pos == 0 {
            for i in 0..width + 1 {
                result += " ";
            }
            return result;
        }

        let mut indicator_pos = ((pos - start) as f32 / (end - start) as f32 * (width) as f32) as usize;
        if indicator_pos > width {
            indicator_pos = width;
        }
        for i in 0..indicator_pos {
            result += &*Self::color(colors[i], "=");
        }
        result += &*Self::color(colors[indicator_pos], "=");
        for i in indicator_pos + 1..(width + 1) as usize {
            result += " "; //&*Self::color(colors[i], "-");
        }
        result += "\x1b[0m";
        result
    }


    fn display(play_data: &PlayData, cur_tick: usize) {
        let colors: [RGB; 12] = [
            RGB { R: 0, G: 120, B: 0 },
            RGB { R: 0, G: 140, B: 0 },
            RGB { R: 0, G: 160, B: 0 },
            RGB { R: 0, G: 180, B: 0 },
            RGB { R: 180, G: 180, B: 0 },
            RGB { R: 195, G: 195, B: 0 },
            RGB { R: 210, G: 210, B: 0 },
            RGB { R: 225, G: 225, B: 0 },
            RGB { R: 225, G: 64, B: 0 },
            RGB { R: 225, G: 64, B: 0 },
            RGB { R: 225, G: 64, B: 0 },
            RGB { R: 225, G: 64, B: 0 },
        ];
        let first_tick = play_data.tick == 0;
        if let Err(_e) = crossterm::execute!(stdout(), Hide, MoveTo(0,0)) {}
        println!("duration in frames: {:5} duration in ms: {:5} tick: {:3} pos: {:3X}/{:<3X}  row: {:3}/{:<3} bpm: {:3} speed: {:3} filter: {:5}",
                 play_data.tick_duration_in_frames, play_data.tick_duration_in_ms, play_data.tick, play_data.song_position, play_data.song_length - 1, play_data.row,
                 play_data.pattern_len - 1,
                 play_data.bpm, play_data.speed,
                 play_data.filter
        );
        if let Err(_e) = crossterm::execute!(stdout(), MoveTo(0,1)) {}

        println!("on | channel |         instrument         |frequency|   volume   |sample_position| note | period |  chan vol  |   envvol   | globalvol  |   fadeout  | panning |");

        let mut idx = 0u32;
        for channel in &play_data.channel_status {
            idx = idx + 1;
//            if idx != 1  {continue;}


            if channel.on {
                let final_vol =
                    (channel.volume / 64.0) *
                        (channel.envelope_volume / 16384.0) *
                        (channel.global_volume / 64.0) *
                        (channel.fadeout_volume / 65536.0);

                println!("{:3}| {:7} | {:26} |  {:<6} |{:11}|{:14}| {:4} | {:7}|{:11}|{:11}|{:11}|{:11}|{:8}|      ",
                         if channel.force_off { " x" } else if channel.on { "on" } else { "off" }, idx, channel.instrument.idx.to_string() + ": " + channel.instrument.name.trim(),
                         if channel.on { (channel.frequency) as u32 } else { 0 },
                         Self::range_with_color((final_vol * 12.0).ceil() as u32, 0, 12, 11, &colors),
                         Self::range(channel.sample_position as u32, 0, channel.sample.length - 1, 14),
                         channel.note, channel.period,
                         Self::range_with_color(channel.volume as u32, 0, 64, 11, &colors),
                         Self::range_with_color(channel.envelope_volume as u32, 0, 16384, 11, &colors),
                         Self::range_with_color(channel.global_volume as u32, 0, 64, 11, &colors),
                         Self::range_with_color(channel.fadeout_volume as u32, 0, 65536, 11, &colors),
                         Self::range(channel.final_panning as u32, 0, 255, 8),
                );
            } else {
                println!("{:3}| {:7} | {:26} |  {:<6} |{:12}| {:14}| {:5}| {:7}|{:12}|{:12}|{:12}|{:12}| {:8}|      ", "off", idx, "", "", "",
                         "", "", "", "", "", "", "", "");
            }
        }
        if let Err(_e) = crossterm::execute!(stdout(), Show) {}
    }
}


fn main() {
    if env::args().len() < 2 {return;}
    let path = env::args().nth(1).unwrap();
    //let file = File::open(path).expect("failed to open the file");

    let data = read_xm(path.as_str());

    if env::args().len() > 2 {
        print_xm(&data);
    } else {
        run(data).unwrap();
    }
}

#[cfg(feature="sdl2-feature")]
struct AudioCB {
   q: PCQHolder
}

#[cfg(feature="sdl2-feature")]
impl AudioCallback for AudioCB {
   type Channel = f32;

   fn callback(&mut self, out: &mut [f32]) {
       if out.len() != AUDIO_BUF_SIZE {panic!("unexpected frame size: {}", out.len());}
       if !self.q.get().consume(|buf: &[f32; AUDIO_BUF_SIZE]| { out.clone_from_slice(buf); }) {
           // pa::Complete
       } else {
           // pa::Continue
       }
   }
}

#[cfg(feature="portaudio-feature")] type ErrorType = pa::Error;
#[cfg(feature="sdl2-feature")] type ErrorType = Error;

fn run(song_data : SongData) -> Result<(), ErrorType> {
    const CHANNELS: i32 = 2;
    const NUM_SECONDS: i32 = 500;
    const SAMPLE_RATE: f32 = 48_000.0;

    let (mut triple_buffer_reader, mut triple_buffer_writer) = TripleBuffer::<PlayData>::new();

    let mut song = Song::new(&song_data, triple_buffer_writer, SAMPLE_RATE);
    let (tx, rx): (Sender<PlaybackCmd>, Receiver<PlaybackCmd>) = mpsc::channel();


    let mut temp_buf = [0.0f32; AUDIO_BUF_SIZE];
    let buf_ref = Arc::new(AtomicPtr::new(&mut temp_buf as *mut [f32; AUDIO_BUF_SIZE]));
    let mut generator = song.get_next_tick_callback(buf_ref.clone(), rx);

    let q = ProducerConsumerQueue::new();

    #[cfg(feature="portaudio-feature")]
    let pa_result: Result<pa::PortAudio, pa::Error> = pa::PortAudio::new();
    #[cfg(feature="portaudio-feature")]
    let pa = match pa_result {
        Ok(p) => p,
        Err(e) => return Err(e),
    };

    #[cfg(feature="sdl2-feature")]
    let desired_spec = AudioSpecDesired {
       freq: Some(SAMPLE_RATE as i32),
       channels: Some(2),
       samples: Some((AUDIO_BUF_SIZE / 2) as u16)
    };

    #[cfg(feature="sdl2-feature")] let sdl_context = sdl2::init().unwrap();
    #[cfg(feature="sdl2-feature")] let audio = sdl_context.audio().unwrap();
    #[cfg(feature="portaudio-feature")]
    let pa = pa::PortAudio::new()?;
    #[cfg(feature="portaudio-feature")]
    let settings =
        pa.default_output_stream_settings(CHANNELS, SAMPLE_RATE as f64, (AUDIO_BUF_SIZE / 2) as u32)?;

    let mut qclone = q.clone();

    // This routine will be called by the PortAudio engine when audio is needed. It may called at
    // interrupt level on some machines so don't do anything that could mess up the system like
    // dynamic resource allocation or IO.
    #[cfg(feature="portaudio-feature")]
    let callback = move |pa::OutputStreamCallbackArgs { buffer, frames, .. }| {
        if frames != AUDIO_BUF_FRAMES {panic!("unexpected frame size: {}", frames);}
        if !qclone.get().consume(|buf: &[f32; AUDIO_BUF_SIZE]| { buffer.clone_from_slice(buf); }) {
            pa::Complete
        } else {
            pa::Continue
        }
    };

    #[cfg(feature="sdl2-feature")]
    let audio_output = audio.open_playback(None, &desired_spec, |spec| {
       AudioCB{ q: qclone }
    }).unwrap();

    #[cfg(feature="portaudio-feature")]
    let mut stream = pa.open_non_blocking_stream(settings, callback)?;

    let stopped = Arc::new(AtomicBool::from(false));
    let thread_stopped = stopped.clone();
    let thread_stopped_reader = stopped.clone();
    thread::scope(|scope| {
        {
            let mut q = q.clone();
            scope.spawn(move |_| {
                let q = q.get();

                q.produce(|buf: &mut [f32; AUDIO_BUF_SIZE]| -> bool {
                    // println!("produce {}", AUDIO_BUF_SIZE);
                    buf_ref.store(buf as *mut [f32; AUDIO_BUF_SIZE], Ordering::Release);
                    if let GeneratorState::Complete(_) = Pin::new(&mut generator).resume(()) { return false; }
                    true
                });
               thread_stopped.store(true, Ordering::Release);
            });

            scope.spawn( move |_| {
                let mut song_row = 0;
                let mut song_tick = 2000;

                loop {
                    if thread_stopped_reader.load(Ordering::Acquire) == true {
                        break;
                    }
                    sleep(Duration::from_millis(30));
                    let (play_data, state) = triple_buffer_reader.read();
                    if STATE_NO_CHANGE == state { continue; }
                    if play_data.tick != song_tick || play_data.row != song_row {
                        Display::display(play_data, 0);
                        song_row = play_data.row;
                        song_tick = play_data.tick;
                    }
                }
            }
            );

        }



//    settings.flags = pa::stream_flags::CLIP_OFF;
//
        #[cfg(feature="portaudio-feature")]
         stream.start().unwrap();
//
        #[cfg(feature="sdl2-feature")]
        audio_output.resume();

        println!("Play for {} seconds.", NUM_SECONDS);
        mainloop(tx, stopped);
    }).ok();

    #[cfg(feature="portaudio-feature")]
    stream.stop().unwrap();
    #[cfg(feature="portaudio-feature")]
    stream.close().unwrap();

    #[cfg(feature="sdl2-feature")]
    audio_output.close_and_get_callback();
    println!("Test finished.");



//    println!("samples: {}", *count.lock().unwrap());
    Ok(())
}

fn is_num (ch: u8) -> bool {
    ch >= '0' as u8 && ch <= '9' as u8
}

fn mainloop(tx: Sender<PlaybackCmd>, stopped: Arc<AtomicBool>) {

    let getter = Getch::new();
    let mut last_time = SystemTime::now();
    let mut last_char = 0;

    loop {
        if stopped.load(Ordering::Acquire) {break;}

        // let input = tokio::time::timeout(Duration::from_secs(1), getter.getch()).await;
        let input = getter.getch();
        if SystemTime::now() > last_time + Duration::from_secs(1) {
            last_char = 0;
        }

        if let Ok(ch) = input {
            if ch == 'q' as u8 {
                let _ = tx.send(PlaybackCmd::Quit);
                break;
            }
            if is_num(ch) {
                if is_num(last_char) {
                    let channel_number = (last_char - '0' as u8)  * 10 + (ch - '0' as u8);
                    if channel_number > 0  && channel_number <= 32 {
                        let _ = tx.send(PlaybackCmd::ChannelToggle(channel_number - 1));
                    }
                    last_char = 0;
                } else {
                    last_char = ch;
                }
            }
            if ch == '+' as u8 {
                let _ = tx.send(PlaybackCmd::IncSpeed);
            }
            if ch == '-' as u8 {
                let _ = tx.send(PlaybackCmd::DecSpeed);
            }
            if ch == '.' as u8 {
                let _ = tx.send(PlaybackCmd::IncBPM);
            }
            if ch == ',' as u8 {
                let _ = tx.send(PlaybackCmd::DecBPM);
            }
            if ch == ' ' as u8 {
                let _ = tx.send(PlaybackCmd::PauseToggle);
            }
            if ch == 'n' as u8 {
                let _ = tx.send(PlaybackCmd::Next);
            }
            if ch == '/' as u8 {
                let _ = tx.send(PlaybackCmd::LoopPattern);
            }
            if ch == 'p' as u8 {
                let _ = tx.send(PlaybackCmd::Prev);
            }
            if ch == 'r' as u8 {
                let _ = tx.send(PlaybackCmd::Restart);
            }
            if ch == 'a' as u8 {
                let _ = tx.send(PlaybackCmd::AmigaTable);
            }
            if ch == 'l' as u8 {
                let _ = tx.send(PlaybackCmd::LinearTable);
            }
            if ch == 'f' as u8 {
                let _ = tx.send(PlaybackCmd::FilterToggle);
            }
        }
        //pa.sleep(1_000);
        last_time = SystemTime::now();
    }
}
