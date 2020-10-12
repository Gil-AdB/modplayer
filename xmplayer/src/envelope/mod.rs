
#[derive(Debug, Copy, Clone)]
pub struct EnvelopePoint {
    pub(crate) frame:                   u16,
    pub(crate) value:                   u16
}

impl EnvelopePoint {
    pub(crate) fn new() -> EnvelopePoint {
        EnvelopePoint{ frame: 0, value: 0 }
    }
}


pub type EnvelopePoints = [EnvelopePoint;12];

#[derive(Debug, Copy, Clone)]
pub struct Envelope {
    pub(crate) points:             EnvelopePoints,
    pub(crate) size:               u8,
    pub(crate) sustain_point:      u8,
    pub(crate) loop_start_point:   u8,
    pub(crate) loop_end_point:     u8,
    pub(crate) on:                 bool,
    pub(crate) sustain:            bool,
    pub(crate) has_loop:           bool,
}

impl Envelope {
    pub(crate) fn new() -> Self {
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

    pub(crate) fn create(points: [EnvelopePoint; 12], size: u8, sustain_point: u8, loop_start_point: u8, loop_end_point: u8, env_type: u8) -> Self {
        Envelope {
            points,
            size,
            sustain_point,
            loop_start_point,
            loop_end_point,
            on: (env_type & 1) == 1,
            sustain: (env_type & 2) == 2,
            has_loop: (env_type & 4) == 4,
        }
    }

}

