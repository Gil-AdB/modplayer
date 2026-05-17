#!/usr/bin/env python3
"""Render every corpus song through us + OMT and report ratios.

Flags any song where our render's RMS diverges from OMT by more than
the documented per-format baseline band. Use as a regression check
after changes that could affect real-song playback (effect handlers,
loaders, mixer math).

Also computes per-octave-band us/omt ratios and flags songs where any
band drifts far from the per-format band median. This catches bugs
that are invisible to full-song RMS — e.g. the Redalert.mod arpeggio
period_shift leak (commit da45510): full-RMS ratio 1.04 looked fine,
but the 4 kHz band was 2.5x pt2 because every porta-up sweep started
~6 semitones high. RMS averaged out across the band coverage; the
band check would have flagged it.

Usage: scripts/corpus_regression.py [--include-sc2] [--bands-only]
"""
import argparse
import subprocess
import sys
from pathlib import Path

import numpy as np
from scipy.io import wavfile


REPO = Path(__file__).resolve().parent.parent
RENDER_WAV = REPO / "target" / "release" / "render_wav"
OMT_SOLO = REPO / "target" / "release" / "openmpt_solo"
CORPUS = REPO / "scratch" / "corpus_src"
TMP = Path("/tmp/corpus_reg")

# Per-format expected full-song RMS ratio band (from prior calibration).
BANDS = {
    ".mod":  (0.90, 1.20),
    ".xm":   (0.85, 1.15),
    ".s3m":  (0.70, 1.40),
    ".it":   (0.85, 1.15),
}

# Octave-band centers (Hz) for spectrum-domain ratio check. Each band
# spans [fc/sqrt(2), fc*sqrt(2)] so the bands tile log-frequency space
# without gaps or overlap. 8 kHz is the highest band that matters for
# tracker output (sample rates rarely exceed 32 kHz playback Nyquist).
OCTAVE_CENTERS = [125, 250, 500, 1000, 2000, 4000, 8000]

# A band ratio that deviates from the per-format band median by more
# than this factor is flagged. 1.6x = ±4 dB; tight enough to catch the
# Redalert-class bug (4 kHz band was 2.5x median), loose enough to
# tolerate normal cubic-vs-linear-interp HF differences across songs.
BAND_DEVIATION_THRESHOLD = 1.6

# Minimum band amplitude (relative to the song's full-song RMS) below
# which band ratios become numerically meaningless. A 30-second
# rendering of a song whose HF content lives past second 40 will have
# near-zero 8 kHz energy; the ratio of two near-zero numbers is noise
# and produces spurious deviations (Star Control II - Intro.MOD showed
# us/omt=20.9 at 8 kHz at length=30, but us/omt=1.17 over the full song
# — the early window was simply silent at 8 kHz).
BAND_NOISE_FLOOR_REL = 0.02


def render(binary: Path, path: Path, out: Path, length: float) -> bool:
    res = subprocess.run(
        [str(binary), str(path), str(out), "--end-time", str(length)],
        capture_output=True, timeout=120,
    )
    return res.returncode == 0


def load_wav(path: Path):
    """Load a wav as float32 mono at its native rate."""
    r, a = wavfile.read(path)
    if a.dtype == np.int16:
        a = a.astype(np.float32) / 32768.0
    elif a.dtype != np.float32:
        a = a.astype(np.float32)
    if a.ndim == 2:
        a = a.mean(axis=1)
    return r, a


def rms(a: np.ndarray) -> float:
    return float(np.sqrt(np.mean(a ** 2)))


def band_rms(a: np.ndarray, rate: int, centers=OCTAVE_CENTERS, nfft=4096) -> dict:
    """Per-octave-band RMS via averaged power spectrum. Returns
    {center_hz: rms} so a band absent from the signal still gets a
    well-defined zero rather than KeyError downstream."""
    hop = nfft // 2
    win = np.hanning(nfft)
    nb = max((len(a) - nfft) // hop, 1)
    psd = np.zeros(nfft // 2 + 1)
    for i in range(nb):
        seg = a[i * hop : i * hop + nfft] * win
        psd += np.abs(np.fft.rfft(seg)) ** 2
    psd /= nb
    freqs = np.fft.rfftfreq(nfft, 1.0 / rate)
    out = {}
    for fc in centers:
        lo, hi = fc / np.sqrt(2), fc * np.sqrt(2)
        mask = (freqs >= lo) & (freqs < hi)
        # mean PSD in band -> equivalent band RMS. sqrt because PSD is
        # power; we want amplitude units for ratio comparisons.
        out[fc] = float(np.sqrt(psd[mask].mean())) if mask.any() else 0.0
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--include-sc2", action="store_true",
                    help="include Star Control II MOD batch (~60 files, slow)")
    ap.add_argument("--length", type=float, default=30.0)
    ap.add_argument("--bands-only", action="store_true",
                    help="suppress per-song RMS line; only print band-flagged songs")
    args = ap.parse_args()

    TMP.mkdir(parents=True, exist_ok=True)

    candidates = []
    for ext in (".mod", ".xm", ".s3m", ".it"):
        candidates.extend(sorted(CORPUS.glob(f"*{ext}")))
    if args.include_sc2:
        sc2 = CORPUS / "SC2"
        if sc2.exists():
            candidates.extend(sorted(sc2.glob("*.mod")) + sorted(sc2.glob("*.MOD")))

    # Two passes: (1) render and collect ratios + per-band ratios;
    # (2) compute per-format band medians and flag deviations.
    rows_by_format = {".mod": [], ".xm": [], ".s3m": [], ".it": []}
    failures = []

    for src in candidates:
        ext = src.suffix.lower()
        if ext not in BANDS:
            continue
        tag = src.stem.replace(" ", "_")[:50]
        us = TMP / f"{tag}_us.wav"
        omt = TMP / f"{tag}_omt.wav"
        try:
            if not render(RENDER_WAV, src, us, args.length):
                raise RuntimeError("us render failed")
            if not render(OMT_SOLO, src, omt, args.length):
                raise RuntimeError("omt render failed")
            ur, ua = load_wav(us)
            or_, oa = load_wav(omt)
            u_full = rms(ua)
            o_full = rms(oa)
            ratio = u_full / o_full if o_full > 1e-6 else float("inf")
            u_bands = band_rms(ua, ur)
            o_bands = band_rms(oa, or_)
            # Noise floor: if either render's band amplitude is well
            # below the song's full-song RMS, the ratio is dominated by
            # whichever side picked up a tiny nonzero from windowing and
            # the comparison is not informative. NaN it out so the
            # deviation pass skips it.
            floor = BAND_NOISE_FLOOR_REL * max(u_full, o_full)
            band_ratios = {}
            for fc in OCTAVE_CENTERS:
                if (u_bands[fc] < floor or o_bands[fc] < floor
                        or o_bands[fc] <= 1e-8):
                    band_ratios[fc] = float("nan")
                else:
                    band_ratios[fc] = u_bands[fc] / o_bands[fc]
        except Exception as e:
            failures.append((src.name, str(e)[:80]))
            continue
        rows_by_format[ext].append({
            "name": src.name, "ratio": ratio, "u": u_full, "o": o_full,
            "band_ratios": band_ratios,
        })

    # Per-format per-band median, used as the reference for deviation.
    band_medians = {}
    for ext, rows in rows_by_format.items():
        if not rows:
            continue
        per_band = {}
        for fc in OCTAVE_CENTERS:
            vals = [r["band_ratios"][fc] for r in rows
                    if not np.isnan(r["band_ratios"][fc])]
            per_band[fc] = float(np.median(vals)) if vals else 1.0
        band_medians[ext] = per_band

    # Pass 2: print and flag.
    for ext in (".mod", ".xm", ".s3m", ".it"):
        rows = rows_by_format[ext]
        if not rows:
            continue
        lo, hi = BANDS[ext]
        meds = band_medians[ext]
        for r in rows:
            rms_flag = " " if lo <= r["ratio"] <= hi else "!"
            # Band-deviation flag: any band whose ratio is far from the
            # per-format band median. The factor we use is symmetric in
            # log space (max(x, 1/x)).
            worst_fc, worst_dev = None, 1.0
            for fc in OCTAVE_CENTERS:
                br = r["band_ratios"][fc]
                if np.isnan(br) or br <= 0 or meds[fc] <= 0:
                    continue
                dev = max(br / meds[fc], meds[fc] / br)
                if dev > worst_dev:
                    worst_dev, worst_fc = dev, fc
            band_flag = "B" if worst_dev > BAND_DEVIATION_THRESHOLD else " "
            if args.bands_only and band_flag == " ":
                continue
            tail = ""
            if band_flag == "B":
                tail = f"  band={worst_fc}Hz dev={worst_dev:.2f}x (us/omt={r['band_ratios'][worst_fc]:.2f}, fmt-med={meds[worst_fc]:.2f})"
            print(f"  {rms_flag}{band_flag} {ext[1:]:<3} {r['ratio']:6.3f}  "
                  f"us={r['u']:.4f} omt={r['o']:.4f}  {r['name']}{tail}")

    print("\n=== Per-format summary ===")
    for ext, rows in rows_by_format.items():
        if not rows:
            continue
        ratios = [r["ratio"] for r in rows if r["ratio"] != float("inf")]
        if not ratios:
            continue
        median = sorted(ratios)[len(ratios) // 2]
        lo, hi = BANDS[ext]
        in_band = sum(1 for r in ratios if lo <= r <= hi)
        n_band_flagged = 0
        meds = band_medians[ext]
        for r in rows:
            for fc in OCTAVE_CENTERS:
                br = r["band_ratios"][fc]
                if np.isnan(br) or br <= 0 or meds[fc] <= 0:
                    continue
                if max(br / meds[fc], meds[fc] / br) > BAND_DEVIATION_THRESHOLD:
                    n_band_flagged += 1
                    break
        print(f"  {ext[1:]:<5} {in_band}/{len(rows)} RMS in band [{lo}, {hi}]  "
              f"median={median:.3f}  band-flagged={n_band_flagged}")
        # Per-band corpus median for diagnostic context.
        print(f"         per-band medians:  " +
              "  ".join(f"{fc}Hz={meds[fc]:.2f}" for fc in OCTAVE_CENTERS))

    if failures:
        print("\n=== Render failures ===")
        for name, err in failures:
            print(f"  {name}: {err}")


if __name__ == "__main__":
    main()
