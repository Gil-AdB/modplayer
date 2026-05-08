#!/usr/bin/env python3
"""Compare dominant frequencies between WAV renders of the same song
window. Used to verify pitch correctness against OpenMPT or other
external reference players when state-dump comparison alone isn't
enough.

Typical workflow:
  # 1) render via OpenMPT
  openmpt123 --render --force song.S3M    # writes song.S3M.wav
  # 2) render via our engine
  cargo run --release --bin render_wav -- song.S3M /tmp/ours.wav --end-time 135
  # 3) extract a short window at the disputed time
  ffmpeg -y -i song.S3M.wav -ss 129.6 -t 0.5 /tmp/win_openmpt.wav
  ffmpeg -y -i /tmp/ours.wav -ss 129.6 -t 0.5 /tmp/win_ours.wav
  # 4) compare (edit `paths` below to point at your wavs)
  python3 scripts/fft_compare.py

Output: top dominant frequency peaks per render plus pairwise
comparisons in cents. A few cents of drift is acceptable; tens of
cents or 0.0 vs +10 cent splits across notes indicates a real
finetune / c2spd / table-lookup divergence worth chasing."""
import sys
import numpy as np
from scipy.io import wavfile
from scipy.signal import find_peaks

def load(path):
    rate, data = wavfile.read(path)
    if data.dtype == np.int16:
        data = data.astype(np.float32) / 32768.0
    elif data.dtype == np.int32:
        data = data.astype(np.float32) / 2147483648.0
    if data.ndim == 2:
        # mix L+R for the FFT (we want the spectral content of the mix)
        data_mono = data.mean(axis=1)
    else:
        data_mono = data
    return rate, data_mono, data

def top_peaks(rate, x, n=12):
    # Window + FFT
    w = np.hanning(len(x))
    spec = np.abs(np.fft.rfft(x * w))
    freqs = np.fft.rfftfreq(len(x), 1.0/rate)
    # Find peaks above some threshold
    peak_idx, _ = find_peaks(spec, height=spec.max() * 0.05, distance=10)
    peak_idx = peak_idx[np.argsort(-spec[peak_idx])][:n]
    return sorted([(freqs[i], spec[i]) for i in peak_idx], key=lambda t: -t[1])

def main():
    paths = {
        'openmpt_s3m':  '/tmp/win_openmpt_s3m.wav',
        'openmpt_xm':   '/tmp/win_openmpt_xm.wav',
        'refactor_s3m': '/tmp/win_refactor_s3m.wav',
    }
    results = {}
    for name, p in paths.items():
        rate, mono, stereo = load(p)
        results[name] = top_peaks(rate, mono)
        # also per-channel top peak
        peaks_l = top_peaks(rate, stereo[:,0], n=4) if stereo.ndim==2 else None
        peaks_r = top_peaks(rate, stereo[:,1], n=4) if stereo.ndim==2 else None
        print(f"=== {name} (mono mix top peaks, Hz) ===")
        for f, m in results[name]:
            print(f"  {f:8.2f} Hz   mag {m:9.1f}")
        if peaks_l:
            print(f"  L top peaks: {[f'{f:.1f}' for f,_ in peaks_l]}")
            print(f"  R top peaks: {[f'{f:.1f}' for f,_ in peaks_r]}")
        print()

    # Pairwise: for each top peak in refactor, find nearest peak in openmpt_s3m
    ref = [f for f,_ in results['refactor_s3m']]
    for cmp_name in ('openmpt_s3m', 'openmpt_xm'):
        cmp = [f for f,_ in results[cmp_name]]
        print(f"=== refactor vs {cmp_name}: pairwise nearest ===")
        for r in ref[:8]:
            nearest = min(cmp, key=lambda c: abs(c - r))
            cents = 1200 * np.log2(nearest / r) if r > 0 and nearest > 0 else 0
            print(f"  refactor {r:8.2f} Hz  ↔  {cmp_name} {nearest:8.2f} Hz   ({cents:+5.1f} cents)")
        print()

main()
