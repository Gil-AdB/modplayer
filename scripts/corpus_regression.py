#!/usr/bin/env python3
"""Render every corpus song through us + OMT and report ratios.

Flags any song where our render's RMS diverges from OMT by more than
the documented per-format baseline band. Use as a regression check
after changes that could affect real-song playback (effect handlers,
loaders, mixer math).

Usage: scripts/corpus_regression.py [--include-sc2]
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

# Per-format expected ratio band (from prior calibration sessions). A
# song outside these bounds is flagged; a song *inside* but very close
# to the edge is informational only.
BANDS = {
    ".mod":  (0.90, 1.20),
    ".xm":   (0.85, 1.15),
    ".s3m":  (0.70, 1.40),
    ".it":   (0.85, 1.15),
}


def render(binary: Path, path: Path, out: Path, length: float) -> bool:
    res = subprocess.run(
        [str(binary), str(path), str(out), "--end-time", str(length)],
        capture_output=True, timeout=120,
    )
    return res.returncode == 0


def rms(path: Path) -> float:
    r, a = wavfile.read(path)
    if a.dtype == np.int16:
        a = a.astype(np.float32) / 32768.0
    elif a.dtype != np.float32:
        a = a.astype(np.float32)
    if a.ndim == 2:
        a = a.mean(axis=1)
    return float(np.sqrt(np.mean(a ** 2)))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--include-sc2", action="store_true",
                    help="include Star Control II MOD batch (~60 files, slow)")
    ap.add_argument("--length", type=float, default=30.0)
    args = ap.parse_args()

    TMP.mkdir(parents=True, exist_ok=True)

    candidates = []
    for ext in (".mod", ".xm", ".s3m", ".it"):
        candidates.extend(sorted(CORPUS.glob(f"*{ext}")))
    if args.include_sc2:
        sc2 = CORPUS / "SC2"
        if sc2.exists():
            candidates.extend(sorted(sc2.glob("*.mod")) + sorted(sc2.glob("*.MOD")))

    by_format = {".mod": [], ".xm": [], ".s3m": [], ".it": []}
    failures = []

    for src in candidates:
        ext = src.suffix.lower()
        if ext not in BANDS:
            continue
        tag = src.stem.replace(" ", "_")[:50]
        us = TMP / f"{tag}_us.wav"
        omt = TMP / f"{tag}_omt.wav"
        try:
            if not render(RENDER_WAV, src, us, args.length): raise RuntimeError("us render failed")
            if not render(OMT_SOLO, src, omt, args.length):   raise RuntimeError("omt render failed")
            u, o = rms(us), rms(omt)
            ratio = u / o if o > 1e-6 else float("inf")
        except Exception as e:
            failures.append((src.name, str(e)[:80]))
            continue
        by_format[ext].append((src.name, ratio, u, o))
        lo, hi = BANDS[ext]
        marker = " " if lo <= ratio <= hi else "!"
        print(f"  {marker} {ext[1:]:<3} {ratio:6.3f}  us={u:.4f} omt={o:.4f}  {src.name}")

    print("\n=== Per-format summary ===")
    for ext, rows in by_format.items():
        if not rows: continue
        ratios = [r for _, r, _, _ in rows if r != float("inf")]
        if not ratios: continue
        median = sorted(ratios)[len(ratios) // 2]
        lo, hi = BANDS[ext]
        in_band = sum(1 for r in ratios if lo <= r <= hi)
        print(f"  {ext[1:]:<5} {in_band}/{len(rows)} in band [{lo}, {hi}]  median={median:.3f}")
    if failures:
        print("\n=== Render failures ===")
        for name, err in failures:
            print(f"  {name}: {err}")


if __name__ == "__main__":
    main()
