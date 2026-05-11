#!/usr/bin/env python3
"""Print the top FFT peaks of one short window from each render. Use
when `compare_renders.py` flags a window and you want to see exactly
which frequencies match and which don't.

Usage:
    scripts/peek_window.py <ours.wav> <ref.wav> <t_start_s> [win_s]
"""
import sys
import numpy as np
from scipy.io import wavfile
from scipy.signal import find_peaks


def load(p):
    r, d = wavfile.read(p)
    if d.dtype == np.int16:
        d = d.astype(np.float32) / 32768.0
    elif d.dtype == np.int32:
        d = d.astype(np.float32) / 2147483648.0
    if d.ndim == 2:
        d = d.mean(axis=1)
    return r, d


def peaks(rate, x, n=15, rel=0.05):
    if x.size == 0:
        return []
    w = np.hanning(len(x))
    spec = np.abs(np.fft.rfft(x * w))
    freqs = np.fft.rfftfreq(len(x), 1.0 / rate)
    idx, _ = find_peaks(spec, height=spec.max() * rel, distance=10)
    idx = idx[np.argsort(-spec[idx])][:n]
    return [(float(freqs[i]), float(spec[i])) for i in idx]


def main():
    ours_p = sys.argv[1]
    ref_p = sys.argv[2]
    t_start = float(sys.argv[3])
    win = float(sys.argv[4]) if len(sys.argv) > 4 else 0.5
    r, o = load(ours_p)
    _, x = load(ref_p)
    s = int(t_start * r)
    e = s + int(win * r)
    po = peaks(r, o[s:e])
    pr = peaks(r, x[s:e])
    print(f"Window [{t_start:.2f}, {t_start + win:.2f}]s")
    print("OUR peaks (Hz, mag):")
    for f, m in po:
        print(f"  {f:9.2f}  {m:.4f}")
    print("REF peaks (Hz, mag):")
    for f, m in pr:
        print(f"  {f:9.2f}  {m:.4f}")


if __name__ == "__main__":
    main()
