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

# Per-format canonical-tracker reference. These predate OMT and are the
# closest thing to ground truth for files authored in their native
# tracker — pt2-clone is ProTracker 2 (MOD), st3play is a direct C port
# of Scream Tracker 3.21 (S3M). Used to surface "us and OMT both wrong"
# cases that pure us/OMT comparison can't catch (Redalert.mod was the
# motivating example). Optional: if the binary isn't present, fall back
# to us/OMT-only output.
CANONICAL = {
    ".mod":  Path("/tmp/pt2-clone/pt2-clone-cli"),
    ".s3m":  Path("/tmp/st3play/st3play-cli"),
    ".it":   Path("/tmp/it2play/it2play-cli"),
    # ".xm" → add when ft2play CLI is built.
}

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


def render_canonical(binary: Path, path: Path, out: Path, length: float) -> bool:
    """Render via a 8bitbubsy *play binary (pt2-clone-cli / st3play-cli).
    These tools render the whole song; we kill the process after the
    output file's size stabilizes so we don't block forever on songs
    with internal loops. Length is the cap, not a `--end-time` arg."""
    import time
    if out.exists(): out.unlink()
    proc = subprocess.Popen(
        [str(binary), str(path), "--render", str(out)],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        stdin=subprocess.DEVNULL,
    )
    last_size, stable = -1, 0
    deadline = time.time() + length + 30
    while time.time() < deadline:
        time.sleep(0.5)
        try:
            size = out.stat().st_size
        except FileNotFoundError:
            size = 0
        if size > 0 and size == last_size:
            stable += 1
            if stable >= 3: break
        else:
            stable = 0
        last_size = size
    proc.terminate()
    try: proc.wait(timeout=2)
    except subprocess.TimeoutExpired: proc.kill()
    return out.exists() and out.stat().st_size > 0


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
        can = TMP / f"{tag}_can.wav"
        canonical_bin = CANONICAL.get(ext)
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
            # Optional canonical reference. Skip silently if the binary
            # isn't built; we only want extra signal when available.
            c_full, c_bands = None, None
            if canonical_bin and canonical_bin.exists():
                if render_canonical(canonical_bin, src, can, args.length):
                    cr, cana = load_wav(can)
                    n_can = len(cana)
                    if cr != ur:
                        # Resample canonical to our analysis rate.
                        from scipy.signal import resample
                        cana = resample(cana, int(n_can * ur / cr)).astype(np.float32)
                    # Compare only on the overlapping prefix; canonical
                    # tools render whole songs, length is just a cap.
                    n = min(len(ua), len(cana))
                    cana = cana[:n]
                    c_full = rms(cana)
                    # Sanity: st3play renders some S3M files as effective
                    # silence (e.g. AdLib-only songs that need the OPL2
                    # driver and don't produce SBPro output). Treating
                    # those as a valid reference produces useless 22x
                    # divergence flags — overdriv.s3m was the canary.
                    if c_full < 0.005 or float(np.max(np.abs(cana))) < 0.05:
                        c_full = None
                    else:
                        c_bands = band_rms(cana, ur)
        except Exception as e:
            failures.append((src.name, str(e)[:80]))
            continue
        rows_by_format[ext].append({
            "name": src.name, "ratio": ratio, "u": u_full, "o": o_full,
            "c": c_full, "band_ratios": band_ratios,
            "u_bands": u_bands, "o_bands": o_bands, "c_bands": c_bands,
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

    # Per-format gain normalization for the canonical reference. st3play
    # and pt2-clone use different default output gains than OMT (e.g.
    # st3play renders S3M at ~2x OMT amplitude in Original mix-levels
    # mode). The corpus median of (omt_rms / canonical_rms) tells us the
    # constant offset to apply before declaring spectral divergence.
    canonical_gain = {}
    for ext, rows in rows_by_format.items():
        ratios = [r["o"] / r["c"] for r in rows
                  if r["c"] is not None and r["c"] > 1e-6]
        if ratios:
            canonical_gain[ext] = float(np.median(ratios))

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
            # Canonical-disagreement flag (C): us and OMT agree, but the
            # format-native tracker disagrees in the same direction. This
            # is the Redalert-class signal — both modern players have an
            # inherited bug that the original tracker doesn't.
            canon_flag = " "
            canon_tail = ""
            if r["c"] is not None and r["c"] > 1e-6 and ext in canonical_gain:
                g = canonical_gain[ext]
                # Normalized canonical RMS at OMT's gain.
                c_norm = r["c"] * g
                # us/canonical and omt/canonical at normalized gain.
                uc = r["u"] / c_norm
                oc = r["o"] / c_norm
                # Both diverge from canonical in the same direction, and
                # neither is wildly off from the other (i.e. us-vs-OMT
                # roughly agrees). The threshold for "agreement with
                # canonical" is per-format band loose vs the RMS band.
                same_dir = (uc > 1.15 and oc > 1.15) or (uc < 0.87 and oc < 0.87)
                us_omt_agree = 0.85 <= r["ratio"] <= 1.15
                if same_dir and us_omt_agree:
                    canon_flag = "C"
                    canon_tail = f"  us/canon={uc:.2f}  omt/canon={oc:.2f}"
            if args.bands_only and band_flag == " " and canon_flag == " ":
                continue
            tail = ""
            if band_flag == "B":
                tail = f"  band={worst_fc}Hz dev={worst_dev:.2f}x (us/omt={r['band_ratios'][worst_fc]:.2f}, fmt-med={meds[worst_fc]:.2f})"
            print(f"  {rms_flag}{band_flag}{canon_flag} {ext[1:]:<3} {r['ratio']:6.3f}  "
                  f"us={r['u']:.4f} omt={r['o']:.4f}  {r['name']}{tail}{canon_tail}")

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
        canon_info = ""
        if ext in canonical_gain:
            canon_info = f"  canon-gain={canonical_gain[ext]:.2f} (omt/canon median)"
        print(f"  {ext[1:]:<5} {in_band}/{len(rows)} RMS in band [{lo}, {hi}]  "
              f"median={median:.3f}  band-flagged={n_band_flagged}{canon_info}")
        # Per-band corpus median for diagnostic context.
        print(f"         per-band medians:  " +
              "  ".join(f"{fc}Hz={meds[fc]:.2f}" for fc in OCTAVE_CENTERS))

    if failures:
        print("\n=== Render failures ===")
        for name, err in failures:
            print(f"  {name}: {err}")


if __name__ == "__main__":
    main()
