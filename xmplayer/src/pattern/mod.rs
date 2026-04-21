use std::fmt;
use std::string::ToString;
use crate::module_reader::SongType;

#[derive(Copy, Clone)]
pub struct Pattern {
    pub note: u8,
    pub instrument: u8,
    pub volume: u8,
    pub effect: u8,
    pub effect_param: u8,
}


impl Pattern {
    pub const NOTES: [&'static str; 12] = ["C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-"];

    pub fn new() -> Self {
        Self {
            note: 255,
            instrument: 255,
            volume: 255,
            effect: 255,
            effect_param: 0
        }
    }
    
    fn get_note(&self) -> String {
        if self.note == 97 {"OFF". to_string() } else if self.note == 0 { "   ".to_string() } else {
            format!("{}{}", Pattern::NOTES[((self.note - 1) % 12) as usize], (((self.note - 1) / 12) + '0' as u8) as char)
        }
    }

    pub(crate) fn is_porta_to_note(&self, song_type: SongType) -> bool {
        match song_type {
            SongType::IT | SongType::S3M => self.effect == 0x07, // G
            _ => self.effect == 0x3 // XM / MOD 
        }
    }

    pub(crate) fn is_note_delay(&self, song_type: SongType) -> bool {
        match song_type {
            SongType::IT => self.effect == 0x13 && self.get_x() == 0xd, // S
            SongType::S3M => self.effect == 0x13 && self.get_x() == 0xd, // S
            _ => self.effect == 0xe && self.get_x() == 0xd // XM / MOD
        }
    }

    pub(crate) fn has_vibrato(&self, song_type: SongType) -> bool {
        match song_type {
            SongType::IT => self.effect == 0x08 || self.effect == 0x06, // H or F
            SongType::S3M => self.effect == 0x08 || self.effect == 0x0B, // H or K (Vibrato+VolSlide)
            _ => self.get_volume_effect() == 0xb || self.effect == 0x4 || self.effect == 0x6
        }
    }

    pub(crate) fn has_tremolo(&self, song_type: SongType) -> bool {
        match song_type {
            SongType::IT => self.effect == 0x1D, // Tremolo in IT is 0x1D (R)? No, actually S3M/IT/XM vary.
            SongType::S3M => self.effect == 0x12, // R
            _ => self.effect == 0x7
        }
    }

    pub(crate) fn get_x(&self) -> u8 {
        self.effect_param >> 4
    }

    pub(crate) fn get_y(&self) -> u8 {
        self.effect_param & 0xf
    }

    fn get_volume_effect(&self) -> u8 {
        self.volume & 0xf0 >> 4
    }
    pub(crate) fn get_volume_param(&self) -> u8 {
        self.volume & 0xf
    }

    pub(crate) fn get_vibrato_speed(&self) -> u8 {
        if self.effect == 0x4 { self.get_x() }
        else if (self.volume & 0xf0) == 0xb0 { self.get_volume_param() }
        else { 0 }
    }

    pub(crate) fn get_vibrato_depth(&self) -> u8 {
        if self.effect == 0x4 { self.get_y() }
        else if (self.volume & 0xf0) == 0xa0 { self.get_volume_param() }
        else { 0 }
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
