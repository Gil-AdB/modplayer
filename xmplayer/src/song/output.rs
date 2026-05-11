// Mixing path: SIMD interpolation helpers and the per-tick output_channels
// loop that runs over voices, resamples, applies the resonant filter, and
// writes into the buffer adapter.

use crate::instrument::LoopType;
use crate::tables::PANNING_TAB;
use crate::song::{Song, BufferAdapter, FilterType};

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
use core::arch::wasm32::*;

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

#[cfg(target_arch = "aarch64")]
use core::arch::aarch64::*;

#[inline(always)]
fn sinc_dot_product(samples: &[f32], coeffs: &[f32; 8]) -> f32 {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        unsafe {
            let v0 = v128_load(samples.as_ptr() as *const v128);
            let v1 = v128_load(samples.as_ptr().add(4) as *const v128);
            let c0 = v128_load(coeffs.as_ptr() as *const v128);
            let c1 = v128_load(coeffs.as_ptr().add(4) as *const v128);

            let m0 = f32x4_mul(v0, c0);
            let m1 = f32x4_mul(v1, c1);

            let sum = f32x4_add(m0, m1);
            // Horizontal sum
            let temp = f32x4_add(sum, i32x4_shuffle::<2, 3, 0, 1>(sum, sum));
            let final_sum = f32x4_add(temp, i32x4_shuffle::<1, 0, 3, 2>(temp, temp));
            f32x4_extract_lane::<0>(final_sum)
        }
    }
    #[cfg(all(target_arch = "x86_64", target_feature = "sse"))]
    {
        unsafe {
            let v0 = _mm_loadu_ps(samples.as_ptr());
            let v1 = _mm_loadu_ps(samples.as_ptr().add(4));
            let c0 = _mm_load_ps(coeffs.as_ptr());
            let c1 = _mm_load_ps(coeffs.as_ptr().add(4));

            let m0 = _mm_mul_ps(v0, c0);
            let m1 = _mm_mul_ps(v1, c1);

            let sum = _mm_add_ps(m0, m1);
            // Horizontal sum
            let temp = _mm_add_ps(sum, _mm_movehl_ps(sum, sum));
            let final_sum = _mm_add_ss(temp, _mm_shuffle_ps(temp, temp, 1));
            _mm_cvtss_f32(final_sum)
        }
    }
    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    {
        unsafe {
            let v0 = vld1q_f32(samples.as_ptr());
            let v1 = vld1q_f32(samples.as_ptr().add(4));
            let c0 = vld1q_f32(coeffs.as_ptr());
            let c1 = vld1q_f32(coeffs.as_ptr().add(4));

            let m0 = vmulq_f32(v0, c0);
            let m1 = vmulq_f32(v1, c1);

            let sum = vaddq_f32(m0, m1);
            // Horizontal sum (Native hardware reduction on AArch64)
            vaddvq_f32(sum)
        }
    }
    #[cfg(not(any(
        all(target_arch = "wasm32", target_feature = "simd128"),
        all(target_arch = "x86_64", target_feature = "sse"),
        all(target_arch = "aarch64", target_feature = "neon")
    )))]
    {
        let mut result = 0.0;
        result += samples[0] * coeffs[0];
        result += samples[1] * coeffs[1];
        result += samples[2] * coeffs[2];
        result += samples[3] * coeffs[3];
        result += samples[4] * coeffs[4];
        result += samples[5] * coeffs[5];
        result += samples[6] * coeffs[6];
        result += samples[7] * coeffs[7];
        result
    }
}

#[inline(always)]
fn lerp_simd(lo: [f32; 4], hi: [f32; 4], t: [f32; 4]) -> [f32; 4] {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        unsafe {
            let vlo = v128_load(lo.as_ptr() as *const v128);
            let vhi = v128_load(hi.as_ptr() as *const v128);
            let vt  = v128_load(t.as_ptr() as *const v128);

            let diff = f32x4_sub(vhi, vlo);
            let res  = f32x4_add(vlo, f32x4_mul(diff, vt));

            let mut result = [0.0f32; 4];
            v128_store(result.as_mut_ptr() as *mut v128, res);
            result
        }
    }
    #[cfg(all(target_arch = "x86_64", target_feature = "sse"))]
    {
        unsafe {
            let vlo = _mm_loadu_ps(lo.as_ptr());
            let vhi = _mm_loadu_ps(hi.as_ptr());
            let vt  = _mm_loadu_ps(t.as_ptr());

            let diff = _mm_sub_ps(vhi, vlo);
            let res  = _mm_add_ps(vlo, _mm_mul_ps(diff, vt));

            let mut result = [0.0f32; 4];
            _mm_storeu_ps(result.as_mut_ptr(), res);
            result
        }
    }
    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    {
        unsafe {
            let vlo = vld1q_f32(lo.as_ptr());
            let vhi = vld1q_f32(hi.as_ptr());
            let vt  = vld1q_f32(t.as_ptr());

            let diff = vsubq_f32(vhi, vlo);
            let res  = vaddq_f32(vlo, vmulq_f32(diff, vt));

            let mut result = [0.0f32; 4];
            vst1q_f32(result.as_mut_ptr(), res);
            result
        }
    }
    #[cfg(not(any(
        all(target_arch = "wasm32", target_feature = "simd128"),
        all(target_arch = "x86_64", target_feature = "sse"),
        all(target_arch = "aarch64", target_feature = "neon")
    )))]
    {
        [
            lo[0] + (hi[0] - lo[0]) * t[0],
            lo[1] + (hi[1] - lo[1]) * t[1],
            lo[2] + (hi[2] - lo[2]) * t[2],
            lo[3] + (hi[3] - lo[3]) * t[3],
        ]
    }
}

#[inline(always)]
fn cubic_simd(p0: [f32; 4], p1: [f32; 4], p2: [f32; 4], p3: [f32; 4], t: [f32; 4]) -> [f32; 4] {
     #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        unsafe {
            let vp0 = v128_load(p0.as_ptr() as *const v128);
            let vp1 = v128_load(p1.as_ptr() as *const v128);
            let vp2 = v128_load(p2.as_ptr() as *const v128);
            let vp3 = v128_load(p3.as_ptr() as *const v128);
            let vt  = v128_load(t.as_ptr() as *const v128);

            let three = f32x4_splat(3.0);
            let two   = f32x4_splat(2.0);
            let five  = f32x4_splat(5.0);
            let four  = f32x4_splat(4.0);
            let half  = f32x4_splat(0.5);

            let c3 = f32x4_add(f32x4_sub(f32x4_mul(three, f32x4_sub(vp1, vp2)), vp0), vp3);
            let c2 = f32x4_sub(f32x4_add(f32x4_sub(f32x4_mul(two, vp0), f32x4_mul(five, vp1)), f32x4_mul(four, vp2)), vp3);
            let c1 = f32x4_sub(vp2, vp0);
            let c0 = vp1;

            let res = f32x4_add(f32x4_mul(half, f32x4_mul(f32x4_add(f32x4_mul(f32x4_add(f32x4_mul(c3, vt), c2), vt), c1), vt)), c0);

            let mut result = [0.0f32; 4];
            v128_store(result.as_mut_ptr() as *mut v128, res);
            result
        }
    }
    #[cfg(all(target_arch = "x86_64", target_feature = "sse"))]
    {
        unsafe {
            let vp0 = _mm_loadu_ps(p0.as_ptr());
            let vp1 = _mm_loadu_ps(p1.as_ptr());
            let vp2 = _mm_loadu_ps(p2.as_ptr());
            let vp3 = _mm_loadu_ps(p3.as_ptr());
            let vt  = _mm_loadu_ps(t.as_ptr());

            let three = _mm_set1_ps(3.0);
            let two   = _mm_set1_ps(2.0);
            let five  = _mm_set1_ps(5.0);
            let four  = _mm_set1_ps(4.0);
            let half  = _mm_set1_ps(0.5);

            let c3 = _mm_add_ps(_mm_sub_ps(_mm_mul_ps(three, _mm_sub_ps(vp1, vp2)), vp0), vp3);
            let c2 = _mm_sub_ps(_mm_add_ps(_mm_sub_ps(_mm_mul_ps(two, vp0), _mm_mul_ps(five, vp1)), _mm_mul_ps(four, vp2)), vp3);
            let c1 = _mm_sub_ps(vp2, vp0);
            let c0 = vp1;

            let res = _mm_add_ps(_mm_mul_ps(half, _mm_mul_ps(_mm_add_ps(_mm_mul_ps(_mm_add_ps(_mm_mul_ps(c3, vt), c2), vt), c1), vt)), c0);

            let mut result = [0.0f32; 4];
            _mm_storeu_ps(result.as_mut_ptr(), res);
            result
        }
    }
    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    {
        unsafe {
            let vp0 = vld1q_f32(p0.as_ptr());
            let vp1 = vld1q_f32(p1.as_ptr());
            let vp2 = vld1q_f32(p2.as_ptr());
            let vp3 = vld1q_f32(p3.as_ptr());
            let vt  = vld1q_f32(t.as_ptr());

            let three = vdupq_n_f32(3.0);
            let two   = vdupq_n_f32(2.0);
            let five  = vdupq_n_f32(5.0);
            let four  = vdupq_n_f32(4.0);
            let half  = vdupq_n_f32(0.5);

            let c3 = vaddq_f32(vsubq_f32(vmulq_f32(three, vsubq_f32(vp1, vp2)), vp0), vp3);
            let c2 = vsubq_f32(vaddq_f32(vsubq_f32(vmulq_f32(two, vp0), vmulq_f32(five, vp1)), vmulq_f32(four, vp2)), vp3);
            let c1 = vsubq_f32(vp2, vp0);
            let c0 = vp1;

            let res = vaddq_f32(vmulq_f32(half, vmulq_f32(vaddq_f32(vmulq_f32(vaddq_f32(vmulq_f32(c3, vt), c2), vt), c1), vt)), c0);

            let mut result = [0.0f32; 4];
            vst1q_f32(result.as_mut_ptr(), res);
            result
        }
    }
    #[cfg(not(any(
        all(target_arch = "wasm32", target_feature = "simd128"),
        all(target_arch = "x86_64", target_feature = "sse"),
        all(target_arch = "aarch64", target_feature = "neon")
    )))]
    {
        let mut result = [0.0f32; 4];
        for i in 0..4 {
            let c3 = -p0[i] + 3.0 * p1[i] - 3.0 * p2[i] + p3[i];
            let c2 = 2.0 * p0[i] - 5.0 * p1[i] + 4.0 * p2[i] - p3[i];
            let c1 = -p0[i] + p2[i];
            let c0 = p1[i];
            result[i] = 0.5 * (((c3 * t[i] + c2) * t[i]) + c1) * t[i] + c0;
        }
        result
    }
}

#[inline(always)]
fn lerp(pos: f32, p1: f32, p2: f32) -> f32 {
    let t = pos.fract();
    (1.0 - t) * p1 + t * p2
}

impl Song {
    pub fn output_channels(&mut self, current_buf_position: usize, buf: &mut impl BufferAdapter, ticks_to_generate: usize) {
        if self.is_fast_forwarding {
            return;
        }

        // Both axes (master byte mask + post-master scale) come from the
        // shared per-format mixer table (`backend::voice_mix`). S3M/STM
        // mask bit 7 (stereo flag) and apply √2 to match libopenmpt's
        // reference; XM/MOD/IT use the byte as-is at unity. Same source
        // of truth as the per-voice formula consumed by `process_voices`.
        let mix = crate::song::backend::voice_mix(self.song_data.song_type);
        let raw_master = self.master_volume & mix.master_byte_mask;
        let master_gain = (raw_master as f32 / 128.0) * (self.mixing_volume as f32 / 128.0);
        let final_master_gain = master_gain * mix.global_scale;

        for voice in &mut self.voices {
            if !voice.on { continue; }

            // Mixer instrumentation: stamp the global sample-frame position
            // every time we actually render a voice. Distinguishes "this
            // voice is being mixed" from "trigger fired earlier but mixer
            // has since cut us" in state_dump output (the latter is what
            // the prior 119-121s investigation got wrong — state_dump
            // reads `voice.sample_position` which is frozen at trigger
            // time and never advances outside this loop).
            voice.last_render_tick = current_buf_position as u64;

            let sample = self.song_data.get_sample(voice);
            let vol_right = PANNING_TAB[      voice.panning.final_panning as usize] as f32 / 65536.0;
            let vol_left  = PANNING_TAB[256 - voice.panning.final_panning as usize] as f32 / 65536.0;

            let mut i = 0;

            // Fast Path: 4-sample SIMD Block
            while i + 4 <= ticks_to_generate {
                let pos = voice.sample_position;
                let du = voice.du;

                if pos + (4.0 * du) >= sample.length as f32 ||
                   (sample.loop_type != LoopType::NoLoop && pos + (4.0 * du) >= sample.loop_end as f32) {
                    break;
                }

                let out_samples: [f32; 4];
                match self.filter {
                    FilterType::Linear => {
                        let mut lo = [0.0f32; 4];
                        let mut hi = [0.0f32; 4];
                        let mut t  = [0.0f32; 4];
                        for j in 0..4 {
                            let p = pos + (j as f32 * du);
                            let idx = p as usize;
                            lo[j] = sample.data[idx];
                            hi[j] = sample.data[idx+1];
                            t[j]  = p.fract();
                        }
                        out_samples = lerp_simd(lo, hi, t);
                    },
                    FilterType::Cubic => {
                        let mut p0 = [0.0f32; 4];
                        let mut p1 = [0.0f32; 4];
                        let mut p2 = [0.0f32; 4];
                        let mut p3 = [0.0f32; 4];
                        let mut t  = [0.0f32; 4];
                        for j in 0..4 {
                            let p = pos + (j as f32 * du);
                            let idx = p as usize;
                            p0[j] = sample.data[idx.saturating_sub(1)];
                            p1[j] = sample.data[idx];
                            p2[j] = sample.data[idx+1];
                            p3[j] = sample.data[idx+2];
                            t[j]  = p.fract();
                        }
                        out_samples = cubic_simd(p0, p1, p2, p3, t);
                    },
                    FilterType::Sinc => {
                        let mut temp = [0.0f32; 4];
                        for j in 0..4 {
                            let p = pos + (j as f32 * du);
                            let idx = p as usize;
                            let phase = (p.fract() * 512.0) as usize;
                            let table = &self.frequency_tables.resampling.sinc_table[phase];
                            temp[j] = sinc_dot_product(&sample.data[idx.saturating_sub(3)..], table);
                        }
                        out_samples = temp;
                    },
                    FilterType::None => {
                        let mut temp = [0.0f32; 4];
                        for j in 0..4 {
                            let p = pos + (j as f32 * du);
                            temp[j] = sample.data[p as usize];
                        }
                        out_samples = temp;
                    }
                }

                let output_vol = (voice.volume.output_volume / 2.0) * final_master_gain;
                let channel = &mut self.channels[voice.channel_idx];

                for j in 0..4 {
                    let mut final_sample = out_samples[j];

                    // Resonant Filter
                    if voice.filter_cutoff < 127 {
                        let low = voice.filter_state.history[0];
                        let band = voice.filter_state.history[1];
                        let new_band = band + voice.filter_state.a * (final_sample - low - voice.filter_state.b * band);
                        let new_low = low + voice.filter_state.a * new_band;
                        voice.filter_state.history[0] = new_low;
                        voice.filter_state.history[1] = new_band;
                        final_sample = new_low;
                    }

                    final_sample *= output_vol;

                    // Update per-channel visualizer
                    channel.last_samples[channel.last_samples_pos] = final_sample;
                    channel.last_samples_pos = (channel.last_samples_pos + 1) % 512;

                    let l = final_sample * vol_left;
                    let r = final_sample * vol_right;

                    self.master_samples[self.master_samples_pos] = (l + r) / 2.0;
                    self.master_samples_pos = (self.master_samples_pos + 1) % 8192;

                    buf.mix_samples(0, &[l], current_buf_position + i + j);
                    buf.mix_samples(1, &[r], current_buf_position + i + j);
                }

                voice.sample_position += 4.0 * du;
                i += 4;
            }

            // Scalar Fallback
            while i < ticks_to_generate {
                if voice.sample_position as u32 >= sample.length {
                    voice.on = false;
                    voice.cut_reason = Some(crate::channel_state::VoiceCutReason::SampleEnd);
                    break;
                }

                let mut out_sample: f32 = match self.filter {
                    FilterType::Linear => {
                        let pos = voice.sample_position as usize;
                        lerp(voice.sample_position, sample.data[pos], sample.data[pos+1])
                    },
                    FilterType::Cubic => {
                        let pos = voice.sample_position as usize;
                        voice.spline_data.p0 = sample.data[pos.saturating_sub(1)];
                        voice.spline_data.p1 = sample.data[pos];
                        voice.spline_data.p2 = sample.data[pos+1];
                        voice.spline_data.p3 = sample.data[pos+2];
                        voice.spline_data.interpolate(voice.sample_position.fract())
                    },
                    FilterType::Sinc => {
                        let pos = voice.sample_position as usize;
                        let phase = (voice.sample_position.fract() * 512.0) as usize;
                        let table = &self.frequency_tables.resampling.sinc_table[phase];
                        sinc_dot_product(&sample.data[pos - 3..], table)
                    },
                    FilterType::None => {
                        sample.data[voice.sample_position as usize]
                    }
                };

                // Resonant Filter
                if voice.filter_cutoff < 127 {
                    let low = voice.filter_state.history[0];
                    let band = voice.filter_state.history[1];
                    let new_band = band + voice.filter_state.a * (out_sample - low - voice.filter_state.b * band);
                    let new_low = low + voice.filter_state.a * new_band;
                    voice.filter_state.history[0] = new_low;
                    voice.filter_state.history[1] = new_band;
                    out_sample = new_low;
                }

                let final_sample = (out_sample / 2.0) * voice.volume.output_volume * final_master_gain;
                let channel = &mut self.channels[voice.channel_idx];

                channel.last_samples[channel.last_samples_pos] = final_sample;
                channel.last_samples_pos = (channel.last_samples_pos + 1) % 512;

                let l = final_sample * vol_left;
                let r = final_sample * vol_right;

                self.master_samples[self.master_samples_pos] = (l + r) / 2.0;
                self.master_samples_pos = (self.master_samples_pos + 1) % 8192;

                buf.mix_sample(0, l, current_buf_position + i);
                buf.mix_sample(1, r, current_buf_position + i);

                voice.sample_position += voice.du;

                if voice.sample_position as u32 >= sample.length ||
                    (sample.loop_type != LoopType::NoLoop && voice.sample_position >= sample.loop_end as f32) {
                    voice.loop_started = true;
                    match sample.loop_type {
                        LoopType::NoLoop => {
                            voice.on = false;
                            voice.cut_reason = Some(crate::channel_state::VoiceCutReason::SampleEnd);
                            voice.volume.set_volume(0);
                            break;
                        }
                        LoopType::ForwardLoop => {
                            voice.sample_position = (voice.sample_position - sample.loop_end as f32) + sample.loop_start as f32;
                        }
                        // Ping-pong loops are unfolded into ForwardLoop at load
                        // time (see Sample::setup_loops_and_padding); the engine
                        // never sees PingPongLoop here.
                        LoopType::PingPongLoop => unreachable!("ping-pong should have been unfolded to forward loop at load"),
                    }
                }
                i += 1;
            }
        }
    }
}
