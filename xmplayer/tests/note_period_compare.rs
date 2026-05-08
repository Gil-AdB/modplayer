// Synthetic test: compare our period LUT (AMIGA_PERIODS) with OpenMPT's
// closed-form S3M period formula at c5_speed = 8363 (the default).
//
// OpenMPT's S3M period (Snd_fx.cpp:6456):
//   note0 = note - NOTE_MIN          (NOTE_MIN = 1, so note0 is 0-indexed)
//   period = 8363 * 32 * FreqS3MTable[note0 % 12] / (c5_speed << (note0 / 12))
//
// Our path: AMIGA_PERIODS[(note - 1) * 16 + ((finetune >> 3) + 16)].
// At finetune = 0, idx = note * 16.
//
// Goal: figure out the offset between OpenMPT's note convention and ours, and
// quantify the residual cents error introduced by the finite LUT resolution.

use xmplayer::tables::AMIGA_PERIODS;

const FREQ_S3M_TABLE: [u16; 12] = [1712, 1616, 1524, 1440, 1356, 1280, 1208, 1140, 1076, 1016, 960, 907];

fn openmpt_s3m_period(note0: u32, c5_speed: u32) -> u32 {
    // Closed form, exact integer math from OpenMPT's Snd_fx.cpp:6456.
    let f = FREQ_S3M_TABLE[(note0 % 12) as usize] as u64;
    let num = 8363u64 * (f << 5);
    let den = (c5_speed as u64) << (note0 / 12);
    (num / den) as u32
}

fn our_period(note: u8) -> u32 {
    let idx = (note as usize - 1) * 16 + 16; // finetune = 0
    AMIGA_PERIODS[idx] as u32
}

fn cents(p_ours: u32, p_omt: u32) -> f64 {
    // period is inverse to frequency, so cents diff is -1200 * log2(p_ours/p_omt).
    -1200.0 * (p_ours as f64 / p_omt as f64).log2()
}

#[test]
#[ignore]
fn print_note_period_table() {
    // For each of our notes, find which OpenMPT 0-indexed note (at c5=8363)
    // produces the closest period. The offset should be constant — that's the
    // note-convention shift we need to apply when porting to the formula.
    println!("our_note  our_period  omt_note(c5=8363)  omt_period  cents_diff");
    for our_note in 1u8..=96 {
        let ours = our_period(our_note);
        let mut best_omt = 0u32;
        let mut best_diff = u32::MAX;
        let mut best_period = 0u32;
        for omt in 0u32..=120 {
            let p = openmpt_s3m_period(omt, 8363);
            let d = (p as i64 - ours as i64).unsigned_abs() as u32;
            if d < best_diff {
                best_diff = d;
                best_omt = omt;
                best_period = p;
            }
        }
        let c = cents(ours, best_period);
        println!(
            "{:>8}  {:>10}  {:>17}  {:>10}  {:>+9.3}",
            our_note, ours, best_omt, best_period, c
        );
    }
}

#[test]
#[ignore]
fn print_finetune_resolution_at_c5() {
    // At a fixed note, walk finetune across the LUT range to see the LUT step
    // size in cents. This shows the worst-case quantization error from
    // c2spd → (relnote, finetune) → LUT.
    let note: u8 = 49; // arbitrary mid-range
    println!("note={} (our convention), finetune sweep:", note);
    println!("finetune  idx  period  cents_from_ft0");
    let p_ft0 = our_period(note);
    for ft in -16i8..=15 {
        let idx = (note as i32 - 1) * 16 + ((ft >> 3) as i32 + 16);
        let p = AMIGA_PERIODS[idx as usize] as u32;
        let c = -1200.0 * (p as f64 / p_ft0 as f64).log2();
        println!("{:>8}  {:>3}  {:>6}  {:>+8.3}", ft, idx, p, c);
    }
}

#[test]
#[ignore]
fn print_c5_speed_sweep_at_one_note() {
    // For a fixed S3M note (e.g., octave 4 row 0 in S3M file = 0x40 → engine
    // note 49), sweep c5_speed and compare:
    //   - what our pipeline produces (via c2spd_to_finetune_relnote → LUT)
    //   - what OpenMPT's continuous formula produces
    // The gap quantifies the error our path eats vs the formula.
    fn c2spd_to_finetune_relnote(c2spd: u32) -> (i8, i8) {
        // Lifted from module_reader::mod_::c2spd_to_finetune_relnote.
        let d_freq = (c2spd as f64 / 8363.0).log2() * (12.0 * 128.0);
        let linear_freq = (d_freq + 0.5) as i32;
        let finetune = (((linear_freq + 128) & 255) - 128) as i8;
        let mut relative_note = ((linear_freq - finetune as i32) >> 7) as i8;
        if relative_note < -48 { relative_note = -48; }
        if relative_note > 71 { relative_note = 71; }
        (finetune, relative_note)
    }
    let s3m_note: u8 = 49; // engine-side note value for S3M oct=4 nibble=0
    println!("s3m_note={}, c5_speed sweep (period, cents from openmpt):", s3m_note);
    println!("c5_speed  rel_note  finetune  our_period  omt_period  cents_diff");
    for c5 in &[8000u32, 8200, 8363, 8500, 8800, 9000, 9500, 10000, 11000, 12000, 16000] {
        let (ft, rel) = c2spd_to_finetune_relnote(*c5);
        let real_note = (s3m_note as i16 + rel as i16).clamp(1, 120) as u8;
        let idx = (real_note as i32 - 1) * 16 + ((ft >> 3) as i32 + 16);
        let our_p = AMIGA_PERIODS[idx.clamp(0, 1935) as usize] as u32;

        // OpenMPT: pass raw S3M note (after NOTE_MIN subtraction). The note
        // mapping from a tracker's 0x40 byte to OpenMPT's internal 0-indexed
        // note is: oct*12 + n + (5*12) - they treat the 4th written octave as
        // octave 5 internally. But for our comparison we want to know the
        // absolute pitch the formula produces — so we sweep `omt_note` to find
        // the best match, the same way the first test does.
        let mut best_omt = 0u32;
        let mut best_p = 0u32;
        let mut best_d = u32::MAX;
        for omt in 0..=120u32 {
            let p = openmpt_s3m_period(omt, *c5);
            let d = (p as i64 - our_p as i64).unsigned_abs() as u32;
            if d < best_d {
                best_d = d;
                best_omt = omt;
                best_p = p;
            }
        }
        let _ = best_omt;
        let c = cents(our_p, best_p);
        println!(
            "{:>8}  {:>+8}  {:>+8}  {:>10}  {:>10}  {:>+9.3}",
            c5, rel, ft, our_p, best_p, c
        );
    }
}
