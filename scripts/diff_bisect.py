#!/usr/bin/env python3
"""Differential per-channel bisect — fair per-channel comparison that
isolates each channel's actual contribution to the full mix, independent
of any per-engine auto-preamp or normalization.

Idea: a channel's "contribution" to the full mix is
    contribution_N = full_mix - mix_with_channel_N_muted
For each engine we compute that contribution the same way (subtraction
in audio space), so any global gain applied to the full mix cancels.
We then compare ours_contribution_N vs OMT_contribution_N — that diff
reflects an actual per-channel rendering disagreement, not a tool
artifact.

Usage:
    scripts/diff_bisect.py <module> --channels N [--end-time SEC]
                           [--t WINDOW_START] [--win WINDOW_LEN]
"""
import argparse
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np
from scipy.io import wavfile
import warnings; warnings.filterwarnings("ignore")


def load(p):
    # Return per-channel arrays as a 2D (frames, channels) tensor so the
    # caller can compute L and R diffs separately. Surround channels
    # (S91 / IT chnpan=100) have L = -R, which cancels exactly when
    # averaged into mono — making them look like "OMT contributes 0"
    # in the channel diff. Keeping stereo lets each side contribute.
    r, d = wavfile.read(p)
    if d.dtype == np.int16:
        d = d.astype(np.float32) / 32768.0
    elif d.dtype == np.int32:
        d = d.astype(np.float32) / 2147483648.0
    d = d.astype(np.float32)
    if d.ndim == 1:
        d = d[:, None]
    return r, d


def rms(x):
    return float(np.sqrt(np.mean(x ** 2))) if x.size else 0.0


def cc(a, b):
    # Flatten to 1D for the correlation — we just want shape similarity,
    # so combining L and R into one stream works fine.
    a = a.flatten(); b = b.flatten()
    n = min(len(a), len(b))
    a, b = a[:n] - a[:n].mean(), b[:n] - b[:n].mean()
    d = float(np.sqrt(np.sum(a * a) * np.sum(b * b)))
    return float(np.sum(a * b) / d) if d > 0 else 0.0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("module")
    ap.add_argument("--channels", type=int, required=True,
                    help="Channel count (openmpt123 --info MOD)")
    ap.add_argument("--end-time", type=float, default=20.0)
    ap.add_argument("--t", type=float, default=0.0,
                    help="Window start time (seconds)")
    ap.add_argument("--win", type=float, default=None,
                    help="Window length (defaults to end-time, i.e. whole render)")
    args = ap.parse_args()

    win_start = args.t
    win_len = args.win if args.win is not None else args.end_time
    win_end = min(win_start + win_len, args.end_time)

    repo = Path(__file__).resolve().parent.parent
    rw = repo / "target/release/render_wav"
    solo = repo / "target/release/openmpt_solo"
    for tool, hint in [(rw, "cargo build --release --bin render_wav"),
                        (solo, "tools/build_openmpt_solo.sh")]:
        if not tool.exists():
            sys.exit(f"missing {tool} — run: {hint}")

    with tempfile.TemporaryDirectory() as td:
        td = Path(td)

        def render_ours(mute_list, out):
            cmd = [str(rw), str(args.module), str(out),
                   "--end-time", str(args.end_time)]
            if mute_list:
                cmd += ["--mute-channels", ",".join(map(str, mute_list))]
            subprocess.run(cmd, check=True, capture_output=True)

        def render_omt(mute_list, out):
            cmd = [str(solo), str(args.module), str(out),
                   "--end-time", str(args.end_time)]
            if mute_list:
                cmd += ["--mute", ",".join(map(str, mute_list))]
            subprocess.run(cmd, check=True, capture_output=True)

        print("Rendering full mix (both engines) ...", file=sys.stderr)
        render_ours([], td / "ours_full.wav")
        render_omt([], td / "omt_full.wav")
        _, ours_full = load(td / "ours_full.wav")
        _, omt_full = load(td / "omt_full.wav")

        # Sanity: full-mix RMS comparison
        rfull = rms(ours_full)
        full_omt = rms(omt_full)
        print(f"Full-mix RMS: ours={rfull:.4f} omt={full_omt:.4f} "
              f"ratio={rfull/full_omt:.3f}", file=sys.stderr)

        rate = 48000  # both tools render at 48k
        s = int(win_start * rate)
        e = int(win_end * rate)

        print(f"\nWindow [{win_start:.1f}..{win_end:.1f}]s — differential per-channel:")
        print(f"  ch   ours_contrib  omt_contrib    ratio    cc    verdict")
        for ch in range(args.channels):
            render_ours([ch], td / "ours_no.wav")
            render_omt([ch], td / "omt_no.wav")
            _, ours_no = load(td / "ours_no.wav")
            _, omt_no = load(td / "omt_no.wav")

            n_o = min(len(ours_full), len(ours_no))
            n_m = min(len(omt_full), len(omt_no))
            ours_contrib = ours_full[:n_o] - ours_no[:n_o]
            omt_contrib = omt_full[:n_m] - omt_no[:n_m]

            seg_o = ours_contrib[s:e]
            seg_m = omt_contrib[s:e]
            ro = rms(seg_o)
            rm = rms(seg_m)
            if ro < 1e-5 and rm < 1e-5:
                continue
            ratio = (ro / rm) if rm > 1e-7 else float("inf")
            c = cc(seg_o, seg_m)
            verdict = ""
            if rm > 1e-4 or ro > 1e-4:
                if not 0.85 <= ratio <= 1.15:
                    verdict = f"LOUDNESS ({'too loud' if ratio > 1 else 'too quiet'})"
                elif c < 0.5:
                    verdict = "WAVE DIFFERS"
                elif c < 0.85:
                    verdict = "some drift"
            print(f"  {ch:2d}    {ro:.5f}      {rm:.5f}    {ratio:>5.2f}   {c:+.2f}  {verdict}")


if __name__ == "__main__":
    main()
