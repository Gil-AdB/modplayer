
#[derive(Debug, Copy, Clone)]
pub struct EnvelopePoint {
    pub frame:                   u16,
    pub value:                   u16
}

impl EnvelopePoint {
    pub fn new() -> EnvelopePoint {
        EnvelopePoint{ frame: 0, value: 0 }
    }
}


pub type EnvelopePoints = [EnvelopePoint;25];

#[derive(Debug, Copy, Clone)]
pub struct Envelope {
    pub points:             EnvelopePoints,
    pub size:               u8,
    pub sustain_point:      u8,
    pub sustain_loop_start_point: u8,
    pub sustain_loop_end_point:   u8,
    pub loop_start_point:   u8,
    pub loop_end_point:     u8,
    pub on:                 bool,
    pub sustain:            bool,
    pub has_loop:           bool,
    pub has_sustain_loop:   bool,
    pub is_filter:          bool,
    /// IT envCarry (envelope flags bit 3): when set, a fresh trigger of the
    /// same instrument continues this envelope from its current position
    /// instead of resetting to frame 0. Required for the filter-sweep
    /// patches in `1_channel_moog.it` (inst 1 pitch envelope, flag 0x8B).
    pub carry:              bool,
}

impl Envelope {
    pub fn new() -> Self {
        Envelope{
            points: [EnvelopePoint { frame: 0, value: 0 }; 25],
            size: 0,
            sustain_point: 0,
            sustain_loop_start_point: 0,
            sustain_loop_end_point: 0,
            loop_start_point: 0,
            loop_end_point: 0,
            on: false,
            sustain: false,
            has_loop: false,
            has_sustain_loop: false,
            is_filter: false,
            carry: false,
        }
    }

    pub fn create(points: [EnvelopePoint; 25], size: u8, sustain_point: u8, sustain_loop_start: u8, sustain_loop_end: u8, loop_start_point: u8, loop_end_point: u8, env_type: u8) -> Self {
        Envelope {
            points,
            size,
            sustain_point,
            sustain_loop_start_point: sustain_loop_start,
            sustain_loop_end_point: sustain_loop_end,
            loop_start_point,
            loop_end_point,
            on: (env_type & 1) == 1,
            sustain: (env_type & 2) == 2,
            has_loop: (env_type & 4) == 4,
            has_sustain_loop: (env_type & 8) == 8,
            is_filter: (env_type & 128) == 128,
            // bit 4 reserved here (`env_type & 0x10`) for IT envCarry.
            carry: (env_type & 0x10) == 0x10,
        }
    }
}
