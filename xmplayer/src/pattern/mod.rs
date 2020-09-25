use std::fmt;
use std::string::ToString;

#[derive(Clone)]
pub(crate) struct Pattern {
    pub(crate) note: u8,
    pub(crate) instrument: u8,
    pub(crate) volume: u8,
    pub(crate) effect: u8,
    pub(crate) effect_param: u8,
}


impl Pattern {
    const NOTES: [&'static str; 12] = ["C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-"];

    pub(crate) fn new() -> Self {
        Self {
            note: 0,
            instrument: 0,
            volume: 0,
            effect: 0,
            effect_param: 0
        }
    }
    
    fn get_note(&self) -> String {
        if self.note == 97 {"OFF". to_string() } else if self.note == 0 { "   ".to_string() } else {
            format!("{}{}", Pattern::NOTES[((self.note - 1) % 12) as usize], (((self.note - 1) / 12) + '0' as u8) as char)
        }
    }

    pub(crate) fn is_porta_to_note(&self) -> bool {
        self.effect == 0x3
    }

    pub(crate) fn is_note_delay(&self) -> bool {
        self.effect == 0xe && self.get_x() == 0xd
    }

    pub(crate) fn has_vibrato(&self) -> bool { self.get_volume_effect() == 0xb || self.effect == 0x4 || self.effect == 0x6 }

    pub(crate) fn has_tremolo(&self) -> bool { self.effect == 0x7 }

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
