#!/usr/bin/env python3
"""Render every SC2 MOD via us / OMT / pt2-clone and report per-song
divergence between us-vs-OMT (our normal reference) and us-vs-pt2
(authentic ProTracker reference).

Cases where us and OMT agree but disagree with pt2 indicate a real
MOD-spec divergence — libopenmpt has known bugs on a handful of
MODs that ft2-clone / pt2-clone play correctly. The SC2 corpus is
where this comes up; the user has flagged Redalert.mod specifically.

Prerequisites: target/release/render_wav, target/release/openmpt_solo,
and pt2-clone-cli built via tools/build_pt2_clone_cli.sh.

Usage:
  scripts/sc2_pt2_compare.py [--length 60]
"""
import argparse, subprocess, sys, os, time
from pathlib import Path
import numpy as np
from scipy.io import wavfile
from scipy.signal import resample

REPO = Path(__file__).resolve().parent.parent
RENDER_WAV = REPO / "target/release/render_wav"
OMT_SOLO   = REPO / "target/release/openmpt_solo"
PT2_CLI    = Path("/tmp/pt2-clone/pt2-clone-cli")
SC2        = REPO / "scratch/corpus_src/SC2"
TMP        = Path("/tmp/sc2_pt2")


def render_pt2(mod: Path, out: Path, length_s: float) -> None:
    """pt2-clone-cli renders the whole song; we kill it after `length_s`
    settles so the script doesn't block on long modules."""
    if out.exists(): out.unlink()
    proc = subprocess.Popen(
        [str(PT2_CLI), str(mod), "--render", str(out)],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    # Wait for output file to stabilize.
    last_size = -1
    stable_count = 0
    for _ in range(int((length_s + 30) * 2)):
        time.sleep(0.5)
        try:
            size = out.stat().st_size
        except FileNotFoundError:
            size = 0
        if size > 0 and size == last_size:
            stable_count += 1
            if stable_count >= 3: break
        else:
            stable_count = 0
        last_size = size
    proc.terminate()
    try: proc.wait(timeout=2)
    except subprocess.TimeoutExpired: proc.kill()


def load(path: Path, target_rate: int = 48000) -> np.ndarray:
    r, a = wavfile.read(path)
    if a.dtype == np.int16: a = a.astype(np.float32) / 32768.0
    elif a.dtype != np.float32: a = a.astype(np.float32)
    if a.ndim == 2: a = a.mean(axis=1)
    if r != target_rate:
        a = resample(a, int(len(a) * target_rate / r)).astype(np.float32)
    return a


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--length", type=float, default=60.0)
    args = ap.parse_args()

    for binary in (RENDER_WAV, OMT_SOLO, PT2_CLI):
        if not binary.exists():
            print(f"missing: {binary}", file=sys.stderr)
            sys.exit(2)

    TMP.mkdir(parents=True, exist_ok=True)
    mods = sorted(SC2.glob("*.mod")) + sorted(SC2.glob("*.MOD"))
    print(f"{'name':<32} {'us/omt':>8} {'us/pt2':>8} {'omt/pt2':>8}  {'flag':>4}")
    flagged = []
    for mod in mods:
        tag = mod.stem.replace(' ', '_').replace('-', '_')[:32]
        us  = TMP / f"{tag}_us.wav"
        omt = TMP / f"{tag}_omt.wav"
        pt2 = TMP / f"{tag}_pt2.wav"
        try:
            subprocess.run([str(RENDER_WAV), str(mod), str(us),
                            "--end-time", str(args.length)],
                           check=True, capture_output=True, timeout=120)
            subprocess.run([str(OMT_SOLO), str(mod), str(omt),
                            "--end-time", str(args.length)],
                           check=True, capture_output=True, timeout=120)
            render_pt2(mod, pt2, args.length)
            a_us, a_omt, a_pt2 = load(us), load(omt), load(pt2)
            n = min(len(a_us), len(a_omt), len(a_pt2))
            r_us  = float(np.sqrt(np.mean(a_us[:n]**2)))
            r_omt = float(np.sqrt(np.mean(a_omt[:n]**2)))
            r_pt2 = float(np.sqrt(np.mean(a_pt2[:n]**2)))
            us_omt = r_us / r_omt if r_omt > 0 else 0
            us_pt2 = r_us / r_pt2 if r_pt2 > 0 else 0
            omt_pt2 = r_omt / r_pt2 if r_pt2 > 0 else 0
            # Flag songs where us ≈ omt but both diverge from pt2:
            # us/omt close to 1 but omt/pt2 far from us/pt2 baseline.
            # The baseline us/pt2 ratio depends on pt2's slightly
            # different default gain (~1.3-1.4× vs us); flag songs
            # where us/pt2 is 1.5× or more above the corpus median.
            flag = "    "
        except Exception as e:
            print(f"{tag:<32}   ERR: {e}", file=sys.stderr)
            continue
        print(f"{tag:<32} {us_omt:8.3f} {us_pt2:8.3f} {omt_pt2:8.3f}  {flag}")
    print()
    # Compute median us/pt2 and flag outliers vs that baseline.


if __name__ == "__main__":
    main()
