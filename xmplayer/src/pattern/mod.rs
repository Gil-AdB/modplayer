use std::fmt;
use std::string::ToString;
use crate::module_reader::SongType;

/// What a pattern row's `note` byte means once the format has been decoded.
///
/// Each backend used to repeat the same chain of `if pattern.note == 97 { ... }
/// else if pattern.note == 121 { ... } else if pattern.note == 122 { ... }`
/// alongside an `is_note_valid` check. `Pattern::note_action` collapses that
/// into a single enum dispatch and centralises the per-format note-range
/// rules (1..=96 for XM/MOD, 1..=120 for IT/S3M).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NoteAction {
    /// No note on this row.
    None,
    /// Trigger a note. Value is the engine-space note (1..=120).
    Trigger(u8),
    /// Note Off — start fade-out / end sustain.
    Off,
    /// Note Cut — silence the voice immediately.
    Cut,
    /// Note Fade — IT-only; force fade-out without releasing sustain.
    Fade,
}

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
            note: 0,
            instrument: 0,
            volume: 255,
            effect: 0,
            effect_param: 0
        }
    }
    
    fn get_note(&self) -> String {
        match self.note {
            0 => "   ".to_string(),
            97 | 254 => "OFF".to_string(),
            121 | 255 => "CUT".to_string(),
            122 | 253 => "FAD".to_string(),
            n if n <= 120 => format!("{}{}", Pattern::NOTES[((n - 1) % 12) as usize], (((n - 1) / 12) + b'0') as char),
            _ => "???".to_string(),
        }
    }

    /// Decode `self.note` into a format-aware `NoteAction`.
    ///
    /// Engine-space encoding (set by the parsers in `module_reader::*`):
    ///   * `1..=120`  trigger that note (range capped at 96 for XM/MOD)
    ///   * `97`       Note Off — XM/MOD/STM only; for IT/S3M this is a real
    ///                C#-7 trigger and Off lives at 254 (matching IT raw)
    ///   * `121`      Note Cut — XM only; IT/S3M Cut is at 255
    ///   * `122`      Note Fade — IT-XM-style; IT Fade also accepted at 253
    ///   * `253-255`  IT/S3M Fade/Off/Cut (raw IT byte values)
    ///   * `0`        empty / nothing
    pub fn note_action(&self, song_type: SongType) -> NoteAction {
        let it_like = matches!(song_type, SongType::IT | SongType::S3M);
        match self.note {
            0 => NoteAction::None,
            // IT raw off/cut/fade markers passed through by the IT parser
            // — note 254 (Off) MUST stay distinct from engine 97 because
            // engine 97 is a real trigger in IT (file note 96 = C#-7).
            253 => NoteAction::Fade,
            254 => NoteAction::Off,
            255 => NoteAction::Cut,
            // XM/MOD-style remapped sentinels.
            // 97 is Off only for non-IT — IT/S3M parsers can produce
            // engine 97 from a real note byte 96.
            97 if !it_like => NoteAction::Off,
            // 121 and 122 are above the IT range (IT real notes peak at
            // engine 120 = B-9), so they're safe sentinels for either
            // format and stay where existing tests expect them.
            121 => NoteAction::Cut,
            122 => NoteAction::Fade,
            n => {
                let max = match song_type {
                    SongType::IT | SongType::S3M => 120,
                    _ => 96,
                };
                if n <= max { NoteAction::Trigger(n) } else { NoteAction::None }
            }
        }
    }

    pub(crate) fn is_porta_to_note(&self, song_type: SongType) -> bool {
        match song_type {
            SongType::IT => {
                self.effect == 0x07 || self.effect == 0x0c || (self.volume >= 193 && self.volume <= 202)
            }
            SongType::XM => {
                self.effect == 0x03 || self.effect == 0x05 || (self.volume >= 0xf0 && self.volume <= 0xfe)
            }
            SongType::S3M => {
                self.effect == 0x07 || self.effect == 0x0c // G or L
            }
            _ => self.effect == 0x3 || self.effect == 0x5 // MOD
        }
    }

    /// True if the row carries a vibrato or vibrato-combo effect. Gates
    /// vib-shift application so a persisted `vibrato_state.pos` doesn't
    /// keep biasing pitch on subsequent rows.
    pub(crate) fn has_vibrato(&self, song_type: SongType) -> bool {
        match song_type {
            SongType::XM | SongType::MOD => {
                self.effect == 0x04 || self.effect == 0x06
                    || (self.volume >= 0xa0 && self.volume <= 0xbf)
            }
            SongType::S3M => {
                // H = vibrato, K = vibrato + vol slide, U = fine vibrato.
                self.effect == 0x08 || self.effect == 0x0b || self.effect == 0x15
            }
            SongType::IT => {
                // H = vibrato, K = vibrato + vol slide. U is fine vibrato
                // (effect 0x15 in IT). Vol col 203-212 = vibrato depth.
                self.effect == 0x08 || self.effect == 0x0b || self.effect == 0x15
                    || (self.volume >= 203 && self.volume <= 212)
            }
            _ => self.effect == 0x04 || self.effect == 0x06,
        }
    }

    pub(crate) fn is_note_delay(&self, song_type: SongType) -> bool {
        match song_type {
            SongType::IT | SongType::S3M => {
                (self.effect == 0x13 || self.effect == 0x0e) && self.get_x() == 0x0d
            }
            _ => self.effect == 0x0e && self.get_x() == 0x0d // XM / MOD
        }
    }

    pub(crate) fn get_x(&self) -> u8 {
        self.effect_param >> 4
    }

    pub(crate) fn get_y(&self) -> u8 {
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
