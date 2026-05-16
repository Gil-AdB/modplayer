#!/usr/bin/env python3
"""Run the OpenMPT MOD test-case modules through our renderer and compare
against libopenmpt's reference render of the same modules.

The test-case modules on the OpenMPT wiki use varied conventions: some
put our-output on L and ProTracker-output on R (so a correct player has
L ≈ R), others are "listen for the 'success' voice" probes, others mix
test and reference across all 4 channels. Rather than try to model each
test's intent, we use libopenmpt as the gold-standard render and check
that our output matches it within tolerance.

For each test module:
    us_wav  = render_wav <module>
    omt_wav = openmpt_solo <module>
    per-channel: rms_us / rms_omt close to 1.0
    per-channel: (us - omt) RMS small relative to omt RMS

Usage: scripts/run_openmpt_mod_tests.py [--length SEC] [--verbose]
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
TEST_DIR = REPO / "tests" / "openmpt_cases" / "mod"

# Per-channel ratio tolerance (us_rms / omt_rms must fall in this band).
RATIO_LOW = 0.85
RATIO_HIGH = 1.15

# Per-channel diff tolerance: rms(us - omt) / rms(omt) below this is a pass.
# This catches cases where average loudness matches but the waveform doesn't.
# Set generously: float-position drift between renderers accumulates over
# tens of seconds, so we tolerate up to 60% diff and rely on ratio for the
# loudness check. The phase-cancellation tests are caught by ratio anyway.
DIFF_THRESHOLD = 0.60


def render(binary: Path, mod_path: Path, length_sec: float, tag: str) -> tuple[int, np.ndarray]:
    out = Path("/tmp") / f"_omt_test_{tag}_{mod_path.stem}.wav"
    subprocess.run(
        [str(binary), str(mod_path), str(out), "--end-time", str(length_sec)],
        check=True, capture_output=True,
    )
    rate, data = wavfile.read(out)
    if data.dtype == np.int16:
        data = data.astype(np.float32) / 32768.0
    elif data.dtype != np.float32:
        data = data.astype(np.float32)
    if data.ndim == 1:
        data = np.stack([data, data], axis=1)
    return rate, data


def analyze(us: np.ndarray, omt: np.ndarray, verbose: bool) -> tuple[bool, str]:
    n = min(len(us), len(omt))
    us = us[:n]; omt = omt[:n]

    fails = []
    detail_parts = []
    for ch, name in enumerate(("L", "R")):
        u = us[:, ch]; o = omt[:, ch]
        u_rms = float(np.sqrt(np.mean(u ** 2)))
        o_rms = float(np.sqrt(np.mean(o ** 2)))

        # Both silent: trivially equal.
        if max(u_rms, o_rms) < 1e-5:
            detail_parts.append(f"{name}:silent")
            continue
        # One silent, the other not: hard fail.
        if min(u_rms, o_rms) < 1e-5:
            fails.append(f"{name}_one_silent")
            detail_parts.append(f"{name}:us={u_rms:.4f}/omt={o_rms:.4f}")
            continue

        ratio = u_rms / o_rms
        diff = float(np.sqrt(np.mean((u - o) ** 2)))
        diff_rel = diff / o_rms

        if ratio < RATIO_LOW or ratio > RATIO_HIGH:
            fails.append(f"{name}_ratio={ratio:.2f}")
        if diff_rel > DIFF_THRESHOLD:
            fails.append(f"{name}_diff={diff_rel:.2f}")

        detail_parts.append(f"{name}:r={ratio:.2f} d={diff_rel:.2f}")

    passed = not fails
    detail = "  ".join(detail_parts)
    if fails and verbose:
        detail += f"  [{', '.join(fails)}]"
    return passed, detail


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--length", type=float, default=15.0,
                    help="render length in seconds (default 15)")
    ap.add_argument("--verbose", "-v", action="store_true")
    ap.add_argument("--only", action="append", default=[],
                    help="run only the named test(s) (without .mod)")
    args = ap.parse_args()

    if not RENDER_WAV.exists() or not OMT_SOLO.exists():
        print(f"binaries not built; run cargo build --release", file=sys.stderr)
        sys.exit(2)

    mods = sorted(TEST_DIR.glob("*.mod"))
    if args.only:
        wanted = set(args.only)
        mods = [m for m in mods if m.stem in wanted]

    pass_count = 0
    fail_count = 0
    failed_names = []

    for mod in mods:
        try:
            r1, us = render(RENDER_WAV, mod, args.length, "us")
            r2, omt = render(OMT_SOLO, mod, args.length, "omt")
            if r1 != r2:
                # Resample-mismatch is rare; bail loudly.
                passed = False
                detail = f"rate mismatch us={r1} omt={r2}"
            else:
                passed, detail = analyze(us, omt, args.verbose)
        except subprocess.CalledProcessError as e:
            passed = False
            detail = f"render failed: {e.stderr.decode(errors='replace')[:200]}"
        except Exception as e:
            passed = False
            detail = f"error: {e}"

        marker = "PASS" if passed else "FAIL"
        print(f"  {marker}  {mod.stem:<28}  {detail}")
        if passed:
            pass_count += 1
        else:
            fail_count += 1
            failed_names.append(mod.stem)

    total = pass_count + fail_count
    print()
    print(f"  {pass_count}/{total} passed")
    if failed_names:
        print(f"  failed: {', '.join(failed_names)}")
    sys.exit(0 if fail_count == 0 else 1)


if __name__ == "__main__":
    main()
