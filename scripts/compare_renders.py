#!/usr/bin/env python3
"""Window-by-window FFT/RMS/cross-correlation comparison between two WAV
renders of the same module. Reports the worst-divergence windows.

Designed for diffing our `render_wav` output against an external
reference (openmpt123, ft2-clone, or another commit's render_wav).

Defaults are tuned for "ignore phase drift, flag real bugs":
  --rms-low 0.5 --rms-high 2.0 --cents-thresh 15 --cc-low 0.3

The cross-correlation threshold is intentionally low because float
sample-position drift between long-running renders is normal and
audibly inaudible. RMS and unmatched-peak counts are the more
trustworthy bug signals.

Usage:
    scripts/compare_renders.py <ours.wav> <ref.wav> [options]
"""
import argparse
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
        mono = data.mean(axis=1)
    else:
        mono = data
    return rate, mono


def rms(x):
    return float(np.sqrt(np.mean(x ** 2))) if x.size else 0.0


def top_peaks(rate, x, n=12, rel=0.05):
    if x.size == 0 or np.max(np.abs(x)) < 1e-9:
        return []
    w = np.hanning(len(x))
    spec = np.abs(np.fft.rfft(x * w))
    freqs = np.fft.rfftfreq(len(x), 1.0 / rate)
    if spec.max() < 1e-9:
        return []
    idx, _ = find_peaks(spec, height=spec.max() * rel, distance=10)
    idx = idx[np.argsort(-spec[idx])][:n]
    return [(float(freqs[i]), float(spec[i])) for i in idx]


def cents(f0, f1):
    if f0 <= 0 or f1 <= 0:
        return float("inf")
    return 1200.0 * np.log2(f1 / f0)


def compare_window(rate, ours, ref, n_peaks=12):
    rms_o = rms(ours)
    rms_r = rms(ref)
    ratio = (rms_o / rms_r) if rms_r > 1e-6 else (float("inf") if rms_o > 1e-6 else 1.0)
    po = top_peaks(rate, ours, n=n_peaks)
    pr = top_peaks(rate, ref, n=n_peaks)
    if rms_o > 1e-6 and rms_r > 1e-6:
        cc = float(np.dot(ours, ref) / (np.linalg.norm(ours) * np.linalg.norm(ref)))
    else:
        cc = 0.0 if (rms_o + rms_r) > 1e-6 else 1.0
    matched, unmatched_o = [], list(range(len(po)))
    used_r = set()
    for io, (fo, _) in enumerate(po):
        best_j, best_diff = -1, float("inf")
        for jr, (fr, _) in enumerate(pr):
            if jr in used_r:
                continue
            c = abs(cents(fo, fr))
            if c < best_diff:
                best_diff = c
                best_j = jr
        if best_j >= 0 and best_diff < 50.0:
            matched.append((io, best_j, best_diff))
            used_r.add(best_j)
            unmatched_o.remove(io)
    unmatched_r = [j for j in range(len(pr)) if j not in used_r]
    return {
        "rms_o": rms_o, "rms_r": rms_r, "ratio": ratio, "cc": cc,
        "matched": matched, "unmatched_o": unmatched_o, "unmatched_r": unmatched_r,
    }


def main():
    p = argparse.ArgumentParser()
    p.add_argument("ours")
    p.add_argument("ref")
    p.add_argument("--window", type=float, default=0.5,
                   help="Window size in seconds")
    p.add_argument("--top", type=int, default=20,
                   help="How many worst windows to print (0 = just headline)")
    p.add_argument("--rms-low", type=float, default=0.5)
    p.add_argument("--rms-high", type=float, default=2.0)
    p.add_argument("--cents-thresh", type=float, default=15.0)
    p.add_argument("--cc-low", type=float, default=0.3,
                   help="Cross-correlation below this counts as a divergence")
    p.add_argument("--audible-rms", type=float, default=0.003,
                   help="Skip windows where neither render is audible")
    p.add_argument("--sustained-deficit-low", type=float, default=0.92,
                   help="Per-window RMS ratio threshold for the sustained-deficit check. "
                        "Tighter than --rms-low because a single missing voice in a busy "
                        "mix only drops full-mix RMS by 5-10%%.")
    p.add_argument("--sustained-deficit-high", type=float, default=1.08,
                   help="Upper threshold for sustained-deficit check.")
    p.add_argument("--sustained-deficit-window-count", type=int, default=10,
                   help="Number of consecutive windows outside the tight band before "
                        "flagging as a sustained deficit (likely missing voice).")
    args = p.parse_args()

    rate_o, mono_o = load(args.ours)
    rate_r, mono_r = load(args.ref)
    assert rate_o == rate_r, f"rate mismatch {rate_o} vs {rate_r}"
    rate = rate_o
    n = min(len(mono_o), len(mono_r))
    mono_o = mono_o[:n]
    mono_r = mono_r[:n]

    win_samples = int(args.window * rate)
    starts = list(range(0, n - win_samples, win_samples))

    divergences = []
    for s in starts:
        e = s + win_samples
        st = compare_window(rate, mono_o[s:e], mono_r[s:e])
        if max(st["rms_o"], st["rms_r"]) < args.audible_rms:
            continue
        reasons = []
        ratio = st["ratio"]
        if not np.isfinite(ratio) or ratio < args.rms_low or ratio > args.rms_high:
            reasons.append(f"rms-ratio={ratio:.2f}")
        worst_cents = max((c for _, _, c in st["matched"]), default=0.0)
        if worst_cents > args.cents_thresh:
            reasons.append(f"cents={worst_cents:.1f}")
        if st["cc"] < args.cc_low:
            reasons.append(f"cc={st['cc']:.2f}")
        n_unmatched = max(len(st["unmatched_o"]), len(st["unmatched_r"]))
        if n_unmatched >= 4:
            reasons.append(f"unmatched={n_unmatched}")
        if not reasons:
            continue
        score = 0.0
        if not np.isfinite(ratio):
            score += 1000.0
        else:
            score += abs(np.log(max(ratio, 1e-6)))
        score += worst_cents / 10.0
        score += (1.0 - st["cc"]) * 5.0
        score += n_unmatched * 0.5
        divergences.append((score, s / rate, reasons, st))

    divergences.sort(key=lambda x: -x[0])
    print(f"Compared {len(starts)} windows of {args.window}s; "
          f"{len(divergences)} flagged.")

    # Sustained-deficit check: a missing voice in a busy mix is a small per-window
    # RMS deficit (5-10%) that's well within --rms-low/high. Scan for stretches of
    # consecutive windows where ratio is persistently outside the tight band.
    ratios = []
    for s in starts:
        e = s + win_samples
        st = compare_window(rate, mono_o[s:e], mono_r[s:e])
        if max(st["rms_o"], st["rms_r"]) < args.audible_rms:
            ratios.append(None)
            continue
        ratios.append(st["ratio"] if np.isfinite(st["ratio"]) else None)

    deficits = []  # (t_start, t_end, n_windows, mean_ratio)
    run_start, run_count, run_sum = None, 0, 0.0
    for i, r_ in enumerate(ratios):
        outside = (r_ is not None and
                   (r_ < args.sustained_deficit_low or r_ > args.sustained_deficit_high))
        if outside:
            if run_start is None:
                run_start = i
                run_count, run_sum = 0, 0.0
            run_count += 1
            run_sum += r_
        else:
            if run_start is not None and run_count >= args.sustained_deficit_window_count:
                deficits.append((run_start * args.window,
                                 (run_start + run_count) * args.window,
                                 run_count, run_sum / run_count))
            run_start, run_count, run_sum = None, 0, 0.0
    if run_start is not None and run_count >= args.sustained_deficit_window_count:
        deficits.append((run_start * args.window,
                         (run_start + run_count) * args.window,
                         run_count, run_sum / run_count))
    if deficits:
        print(f"\nSustained-RMS-deficit regions (likely missing voice):")
        for t0, t1, n, m in deficits:
            print(f"  [{t0:6.1f}s, {t1:6.1f}s]  {n:3d} windows  mean ratio={m:.3f}")
    print()

    for score, t, reasons, st in divergences[: args.top]:
        print(f"[t={t:7.2f}s score={score:5.1f}] {', '.join(reasons)}  "
              f"rms_ours={st['rms_o']:.4f} rms_ref={st['rms_r']:.4f} "
              f"cc={st['cc']:.2f}")


if __name__ == "__main__":
    main()
