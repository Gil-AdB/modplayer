#!/usr/bin/env python3
"""Characterize the pitch-divergence pattern between two renders by
collecting matched-peak cent diffs across all windows.

Distinguishes:
  * Systematic offset (mean/median diff far from 0) — frequency table
    or sample-rate miscalibration affecting the whole song.
  * Scattered drift (mean ≈ 0, wide spread) — per-channel vibrato /
    porta / floating-point timing accumulated drift; audibly fine.
  * Bimodal (e.g. half at 0c, half at ±100c) — one channel out of tune
    or wrong sample mapping.

Usage:
    scripts/cents_distribution.py <ours.wav> <ref.wav> [--window 0.5]
"""
import argparse
import numpy as np
from scipy.io import wavfile
from scipy.signal import find_peaks


def load(p):
    r, d = wavfile.read(p)
    if d.dtype == np.int16:
        d = d.astype(np.float32) / 32768.0
    if d.ndim == 2:
        d = d.mean(axis=1)
    return r, d


def peaks(rate, x, n=20, rel=0.05):
    if x.size < 32:
        return []
    w = np.hanning(len(x))
    spec = np.abs(np.fft.rfft(x * w))
    freqs = np.fft.rfftfreq(len(x), 1.0 / rate)
    if spec.max() < 1e-9:
        return []
    idx, _ = find_peaks(spec, height=spec.max() * rel, distance=8)
    idx = idx[np.argsort(-spec[idx])][:n]
    return sorted([(float(freqs[i]), float(spec[i])) for i in idx])


def cents(f0, f1):
    if f0 <= 0 or f1 <= 0:
        return 0.0
    return 1200.0 * np.log2(f1 / f0)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("ours")
    ap.add_argument("ref")
    ap.add_argument("--window", type=float, default=0.5)
    ap.add_argument("--match-window", type=float, default=80.0,
                    help="Pair peaks within this many cents")
    args = ap.parse_args()

    rate, o = load(args.ours)
    _, x = load(args.ref)
    n = min(len(o), len(x))
    win = int(args.window * rate)

    all_diffs, strong_diffs = [], []
    for s in range(0, n - win, win):
        e = s + win
        po = peaks(rate, o[s:e])
        pr = peaks(rate, x[s:e])
        if not po or not pr:
            continue
        used = set()
        max_mag = max(m for _, m in po) if po else 1e-9
        for fo, mo in po:
            best_j, best_diff = -1, args.match_window
            for j, (fr, _) in enumerate(pr):
                if j in used:
                    continue
                c = abs(cents(fo, fr))
                if c < best_diff:
                    best_diff = c
                    best_j = j
            if best_j >= 0:
                used.add(best_j)
                signed = cents(pr[best_j][0], fo)  # ours - ref
                all_diffs.append(signed)
                if mo > max_mag * 0.25:
                    strong_diffs.append(signed)

    a = np.array(all_diffs)
    s = np.array(strong_diffs)
    print(f"All matched peaks (n={len(a)}):")
    print(f"  median: {np.median(a):+.2f} cents")
    print(f"  mean:   {np.mean(a):+.2f} cents")
    print(f"  pct within ±5c: {(np.abs(a) < 5).mean()*100:.1f}%")
    print(f"  pct within ±15c: {(np.abs(a) < 15).mean()*100:.1f}%")
    print(f"  pct within ±50c: {(np.abs(a) < 50).mean()*100:.1f}%")
    print(f"\nStrong peaks only (top quarter, n={len(s)}):")
    print(f"  median: {np.median(s):+.2f} cents")
    print(f"  mean:   {np.mean(s):+.2f} cents")
    print(f"  pct within ±5c: {(np.abs(s) < 5).mean()*100:.1f}%")
    print(f"  pct within ±15c: {(np.abs(s) < 15).mean()*100:.1f}%")
    print("\nStrong-peak distribution (cents bins):")
    for lo, hi in [(-1e9, -50), (-50, -15), (-15, -5), (-5, 5),
                    (5, 15), (15, 50), (50, 1e9)]:
        pct = ((s >= lo) & (s < hi)).mean() * 100
        print(f"  [{lo:6.0f}, {hi:6.0f}): {pct:5.1f}%")


if __name__ == "__main__":
    main()
