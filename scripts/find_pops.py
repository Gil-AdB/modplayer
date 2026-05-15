#!/usr/bin/env python3
"""Find pop / click / step-discontinuity artifacts in a WAV.

A pop is a single-sample (or few-sample) jump in the output that is much
larger than the local signal slope. RMS-based tools miss it because the
energy contribution of one spike is tiny, but it's acoustically obvious.

When --ref is given, we compute (ours - ref) and look for spikes in that
difference signal — this catches pops that ARE in ours but NOT in the
reference, exactly the bug class we want.

Algorithm: signed first-difference of the (possibly-differential) signal,
scaled by local RMS of the reference signal so a "loud step" during loud
audio doesn't get flagged but the same step during quiet audio does.

Usage:
    scripts/find_pops.py <wav> [--ref <wav>] [--threshold X] [--top N]
"""
import argparse
import sys
import numpy as np


def load_wav(path):
    with open(path, 'rb') as f:
        d = f.read()
    a = np.frombuffer(d[d.find(b'data')+8:], dtype=np.float32)
    if len(a) % 2 == 0:
        l, r = a[0::2], a[1::2]
        return (l + r) * 0.5
    return a


def local_rms(sig, win):
    sq = sig.astype(np.float64) ** 2
    cs = np.cumsum(np.concatenate(([0.0], sq)))
    return np.sqrt(np.maximum(0.0, (cs[win:] - cs[:-win]) / win))


def find_spikes(diff_sig, ref_sig, rate, threshold=4.0, win_ms=20.0):
    win = int(rate * win_ms / 1000.0)
    # Step size sample-to-sample on the differential. Pops are abrupt
    # so the diff has high d/dt.
    d = np.abs(np.diff(diff_sig))
    # Local "expected step" floor: use REFERENCE rms (so our pops in
    # quiet sections are flagged even when the reference is silent).
    rms = local_rms(ref_sig, win)
    n = min(len(d), len(rms))
    d = d[:n]
    rms = np.maximum(rms[:n], 1e-5)
    score = d / rms
    hits = np.where(score > threshold)[0]
    if len(hits) == 0:
        return []
    gap = int(rate * 0.005)
    out = []
    s = hits[0]; p = s; sc = score[s]
    for h in hits[1:]:
        if h - p <= gap:
            if score[h] > sc:
                sc = score[h]; p = h
        else:
            out.append((s, p, sc))
            s = h; p = h; sc = score[h]
    out.append((s, p, sc))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("wav")
    ap.add_argument("--ref", help="reference WAV (OMT); compare diff signal")
    ap.add_argument("--threshold", type=float, default=4.0)
    ap.add_argument("--top", type=int, default=30)
    ap.add_argument("--rate", type=int, default=48000)
    args = ap.parse_args()

    sig = load_wav(args.wav)
    if args.ref:
        ref = load_wav(args.ref)
        n = min(len(sig), len(ref))
        sig = sig[:n]; ref = ref[:n]
        diff = sig - ref
        pops = find_spikes(diff, ref, args.rate, threshold=args.threshold)
        print(f"Pops in {args.wav} relative to {args.ref}:")
    else:
        pops = find_spikes(sig, sig, args.rate, threshold=args.threshold)
        print(f"Pops in {args.wav} (vs own envelope):")

    print(f"  total candidates (threshold={args.threshold}σ): {len(pops)}")
    pops.sort(key=lambda c: -c[2])
    print(f"  top {min(args.top, len(pops))}:")
    print(f"  {'time_s':>9}  {'peak_σ':>7}")
    for s, p, sc in pops[:args.top]:
        t = p / args.rate
        print(f"  {t:9.3f}  {sc:7.1f}")


if __name__ == "__main__":
    main()
