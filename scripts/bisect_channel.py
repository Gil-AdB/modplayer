#!/usr/bin/env python3
"""Per-channel attribution: render each channel in solo via both
engines, then report RMS/cc per channel at a target window. The
channel with the worst cc or out-of-range RMS-ratio is the bug source.

Requires:
  * target/release/render_wav        (cargo build --release --bin render_wav)
  * target/release/openmpt_solo      (tools/build_openmpt_solo.sh)

Usage:
    scripts/bisect_channel.py <module> <t_start_s> [--win 0.5]
                              [--channels 18] [--end-time 75]
"""
import argparse
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np
from scipy.io import wavfile


def load(p):
    r, d = wavfile.read(p)
    if d.dtype == np.int16:
        d = d.astype(np.float32) / 32768.0
    if d.ndim == 2:
        d = d.mean(axis=1)
    return r, d


def rms(x):
    return float(np.sqrt(np.mean(x ** 2))) if x.size else 0.0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("module")
    ap.add_argument("t_start", type=float)
    ap.add_argument("--win", type=float, default=0.5)
    ap.add_argument("--channels", type=int, required=True,
                    help="Channel count (see `openmpt123 --info MOD`)")
    ap.add_argument("--end-time", type=float, default=None,
                    help="Defaults to t_start + 5s")
    ap.add_argument("--rms-min", type=float, default=1e-5,
                    help="Skip channels with no audio in the window")
    args = ap.parse_args()
    end_time = args.end_time or (args.t_start + 5.0)

    repo = Path(__file__).resolve().parent.parent
    rw = repo / "target/release/render_wav"
    solo = repo / "target/release/openmpt_solo"
    if not rw.exists():
        sys.exit(f"missing {rw} — run: cargo build --release --bin render_wav")
    if not solo.exists():
        sys.exit(f"missing {solo} — run: tools/build_openmpt_solo.sh")

    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        all_ch = list(range(args.channels))
        rows = []
        for ch in all_ch:
            others = ",".join(str(x) for x in all_ch if x != ch)
            our_out = td / f"our_{ch}.wav"
            omt_out = td / f"omt_{ch}.wav"
            subprocess.run(
                [str(rw), str(args.module), str(our_out),
                 "--end-time", str(end_time), "--mute-channels", others],
                check=True, capture_output=True,
            )
            subprocess.run(
                [str(solo), str(args.module), str(omt_out),
                 "--solo", str(ch), "--end-time", str(end_time)],
                check=True, capture_output=True,
            )
            r, o = load(our_out)
            _, x = load(omt_out)
            n = min(len(o), len(x))
            s = int(args.t_start * r)
            e = s + int(args.win * r)
            if e > n:
                e = n
            oo, xx = o[s:e], x[s:e]
            ro, rx = rms(oo), rms(xx)
            if max(ro, rx) < args.rms_min:
                continue
            cc = (float(np.dot(oo, xx) / (np.linalg.norm(oo) * np.linalg.norm(xx)))
                  if ro > 1e-6 and rx > 1e-6 else 0.0)
            ratio = ro / rx if rx > 1e-6 else float("inf")
            rows.append((ch, ro, rx, ratio, cc))

    print(f"window [{args.t_start:.2f}, {args.t_start + args.win:.2f}]s "
          f"({args.channels} channels):")
    print(f"{'ch':>3}  {'rms_ours':>10}  {'rms_omt':>10}  {'ratio':>6}  {'cc':>6}")
    for ch, ro, rx, ratio, cc in rows:
        marker = ""
        if cc < 0.5:
            marker = "  <-- cc drift / different waveform"
        elif ratio < 0.5 or ratio > 2.0:
            marker = "  <-- LOUDNESS BUG"
        elif cc < 0.85:
            marker = "  <-- some drift"
        print(f"{ch:>3}  {ro:>10.4f}  {rx:>10.4f}  {ratio:>6.2f}  {cc:>6.2f}{marker}")


if __name__ == "__main__":
    main()
