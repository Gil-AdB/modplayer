#!/usr/bin/env python3
"""Run the OpenMPT test-case modules through our renderer and compare
against libopenmpt's reference render of the same modules.

Per the OpenMPT wiki, the test-case modules use varied conventions:
- some put our-output on L and ProTracker-output on R (correct → L≈R)
- others are "listen for the 'success' voice" probes
- others mix test and reference across all 4 channels.

Rather than model each test's intent, we use libopenmpt as the gold
standard render and check our output matches per-channel within
tolerance.

For each test module:
    us_wav  = render_wav <module>
    omt_wav = openmpt_solo <module>
    per-channel rms_us / rms_omt close to 1.0
    per-channel (us - omt) RMS small relative to omt RMS

Usage: scripts/run_openmpt_tests.py {mod|xm|s3m|it|all} [--length SEC]
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
TEST_ROOT = REPO / "tests" / "openmpt_cases"

RATIO_LOW = 0.85
RATIO_HIGH = 1.15
DIFF_THRESHOLD = 0.60


def render(binary: Path, mod_path: Path, length_sec: float, tag: str) -> tuple[int, np.ndarray]:
    out = Path("/tmp") / f"_omt_test_{tag}_{mod_path.stem}.wav"
    res = subprocess.run(
        [str(binary), str(mod_path), str(out), "--end-time", str(length_sec)],
        capture_output=True,
    )
    if res.returncode != 0:
        raise RuntimeError(res.stderr.decode(errors="replace")[:300] or "render failed")
    rate, data = wavfile.read(out)
    if data.dtype == np.int16:
        data = data.astype(np.float32) / 32768.0
    elif data.dtype != np.float32:
        data = data.astype(np.float32)
    if data.ndim == 1:
        data = np.stack([data, data], axis=1)
    return rate, data


def analyze(us: np.ndarray, omt: np.ndarray) -> tuple[bool, str]:
    n = min(len(us), len(omt))
    us = us[:n]; omt = omt[:n]

    fails = []
    detail_parts = []
    for ch, name in enumerate(("L", "R")):
        u = us[:, ch]; o = omt[:, ch]
        u_rms = float(np.sqrt(np.mean(u ** 2)))
        o_rms = float(np.sqrt(np.mean(o ** 2)))

        if max(u_rms, o_rms) < 1e-5:
            detail_parts.append(f"{name}:silent")
            continue
        if min(u_rms, o_rms) < 1e-5:
            fails.append(f"{name}_one_silent")
            detail_parts.append(f"{name}:us={u_rms:.4f}/omt={o_rms:.4f}")
            continue

        ratio = u_rms / o_rms
        diff = float(np.sqrt(np.mean((u - o) ** 2)))
        diff_rel = diff / o_rms

        if ratio < RATIO_LOW or ratio > RATIO_HIGH:
            fails.append(f"{name}_r={ratio:.2f}")
        if diff_rel > DIFF_THRESHOLD:
            fails.append(f"{name}_d={diff_rel:.2f}")

        detail_parts.append(f"{name}:r={ratio:.2f} d={diff_rel:.2f}")

    return not fails, "  ".join(detail_parts)


def run_format(fmt: str, length: float, only: list[str]) -> tuple[int, int, list[str]]:
    test_dir = TEST_ROOT / fmt
    if not test_dir.exists():
        print(f"  no tests for {fmt} ({test_dir} missing)", file=sys.stderr)
        return 0, 0, []

    ext = "." + fmt
    mods = sorted(test_dir.glob(f"*{ext}"))
    if only:
        wanted = set(only)
        mods = [m for m in mods if m.stem in wanted]

    pass_count = 0
    fail_count = 0
    failed_names = []

    print(f"\n=== {fmt.upper()} ({len(mods)} tests) ===")
    for mod in mods:
        try:
            r1, us = render(RENDER_WAV, mod, length, "us")
            r2, omt = render(OMT_SOLO, mod, length, "omt")
            if r1 != r2:
                passed, detail = False, f"rate mismatch us={r1} omt={r2}"
            else:
                passed, detail = analyze(us, omt)
        except Exception as e:
            passed, detail = False, f"error: {str(e)[:120]}"

        marker = "PASS" if passed else "FAIL"
        print(f"  {marker}  {mod.stem:<36}  {detail}")
        if passed:
            pass_count += 1
        else:
            fail_count += 1
            failed_names.append(mod.stem)

    return pass_count, fail_count, failed_names


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("format", choices=["mod", "xm", "s3m", "it", "all"])
    ap.add_argument("--length", type=float, default=15.0)
    ap.add_argument("--only", action="append", default=[])
    ap.add_argument("--summary-only", action="store_true",
                    help="show only the per-format summary, not each test")
    args = ap.parse_args()

    if not RENDER_WAV.exists() or not OMT_SOLO.exists():
        print(f"binaries not built; run cargo build --release", file=sys.stderr)
        sys.exit(2)

    fmts = ["mod", "xm", "s3m", "it"] if args.format == "all" else [args.format]

    total_pass = 0
    total_fail = 0
    per_format = []

    if args.summary_only:
        # Suppress per-test output by redirecting stdout temporarily; collect via stderr-free path.
        # Simpler: just reprint summaries at the end and silence the rest by replacing print.
        import io, contextlib
        for fmt in fmts:
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                p, f, names = run_format(fmt, args.length, args.only)
            per_format.append((fmt, p, f, names))
            total_pass += p
            total_fail += f
    else:
        for fmt in fmts:
            p, f, names = run_format(fmt, args.length, args.only)
            per_format.append((fmt, p, f, names))
            total_pass += p
            total_fail += f

    print()
    print("=== Summary ===")
    for fmt, p, f, _ in per_format:
        total = p + f
        print(f"  {fmt:<5} {p:>3}/{total:<3} passed")
    if len(fmts) > 1:
        print(f"  {'all':<5} {total_pass:>3}/{total_pass + total_fail:<3} passed")

    sys.exit(0 if total_fail == 0 else 1)


if __name__ == "__main__":
    main()
