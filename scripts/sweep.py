#!/usr/bin/env python3
"""Auto-comparator across the module corpus.

Renders each picked module via OpenMPT (`openmpt123 --render --force`)
and our `render_wav`, walks short FFT windows, and flags any with
significant divergence (RMS ratio outside [low,high] OR matched-peak
cents-diff > N OR missing/extra peaks). Emits a markdown report and
appends each processed hash to corpus/done.tsv so subsequent runs
pick fresh modules.

Usage:
  scripts/sweep.py [--limit N] [--seed S] [--corpus DIR]
                   [--end-time SEC] [--report PATH] [--force]
                   [--rms-low F] [--rms-high F] [--cents-thresh N]

Defaults: limit=5, end-time=180, corpus=corpus, report=corpus/report.md.
`--force` ignores done.tsv and re-considers everything.

Assumes:
  * `cargo build --release --bin render_wav` already produced the binary
    at target/release/render_wav. The script will rebuild if missing.
  * `openmpt123` on PATH (or set OPENMPT123 env var).
"""
import argparse
import os
import random
import shutil
import subprocess
import sys
from pathlib import Path

import numpy as np
from scipy.io import wavfile
from scipy.signal import find_peaks


REPO = Path(__file__).resolve().parent.parent


def parse_args():
    p = argparse.ArgumentParser()
    p.add_argument("--limit", type=int, default=5)
    p.add_argument("--seed", type=int, default=None)
    p.add_argument("--corpus", default=str(REPO / "corpus"))
    p.add_argument("--end-time", type=float, default=180.0,
                   help="Max seconds to render per module")
    p.add_argument("--report", default=None)
    p.add_argument("--force", action="store_true",
                   help="Ignore done.tsv (re-sweep everything)")
    p.add_argument("--rms-low", type=float, default=0.5,
                   help="Flag windows whose ours/ref RMS ratio is below this")
    p.add_argument("--rms-high", type=float, default=2.0,
                   help="Flag windows above this ratio")
    p.add_argument("--cents-thresh", type=float, default=10.0,
                   help="Flag matched peaks deviating more than this many cents")
    return p.parse_args()


def load_manifest(corpus: Path):
    """Returns list of (hash, ext, original_name, file_path) tuples."""
    mf = corpus / "manifest.tsv"
    out = []
    if not mf.exists():
        return out
    with mf.open() as f:
        next(f, None)  # header
        for line in f:
            parts = line.rstrip("\n").split("\t")
            if len(parts) < 5:
                continue
            h, ext, _size, _orig, name = parts[0], parts[1], parts[2], parts[3], parts[4]
            fp = corpus / ext / f"{h}.{ext}"
            if fp.exists():
                out.append((h, ext, name, fp))
    return out


def load_done(corpus: Path):
    df = corpus / "done.tsv"
    seen = set()
    if df.exists():
        with df.open() as f:
            for line in f:
                h = line.strip().split("\t", 1)[0]
                if h:
                    seen.add(h)
    return seen


def append_done(corpus: Path, h: str, status: str):
    df = corpus / "done.tsv"
    with df.open("a") as f:
        f.write(f"{h}\t{status}\n")


def find_render_wav() -> Path:
    p = REPO / "target" / "release" / "render_wav"
    if not p.exists():
        print("building render_wav...", file=sys.stderr)
        subprocess.run(
            ["cargo", "build", "--release", "--bin", "render_wav"],
            cwd=REPO, check=True,
        )
    return p


def find_openmpt123() -> str:
    return os.environ.get("OPENMPT123") or shutil.which("openmpt123") or "openmpt123"


def run_render_wav(bin: Path, module: Path, out: Path, end_time: float):
    """Render via our engine. Returns True on success."""
    try:
        subprocess.run(
            [str(bin), str(module), str(out), "--end-time", str(end_time)],
            check=True, capture_output=True, timeout=end_time * 4 + 30,
        )
        return out.exists() and out.stat().st_size > 1000
    except Exception as e:
        print(f"  render_wav failed for {module}: {e}", file=sys.stderr)
        return False


def run_openmpt(bin: str, module: Path, out: Path):
    """Render via openmpt123. `--output` is only honored in --ui/--batch
    modes; in --render mode openmpt writes `<input>.wav` alongside the
    input. Render in place, then move into the renders/ tree.
    """
    sibling = module.with_suffix(module.suffix + ".wav")
    # If a stale render is sitting next to the input, remove it first so
    # we can detect render success by mtime/existence cleanly.
    if sibling.exists():
        try: sibling.unlink()
        except OSError: pass
    try:
        subprocess.run(
            [bin, "--quiet", "--render", "--force", str(module)],
            check=True, capture_output=True, timeout=600,
        )
    except Exception as e:
        print(f"  openmpt123 failed for {module}: {e}", file=sys.stderr)
        return False
    if not sibling.exists() or sibling.stat().st_size < 1000:
        print(f"  openmpt123 produced no output: {sibling}", file=sys.stderr)
        return False
    sibling.replace(out)
    return True


def load_wav_mono(path: Path):
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


def rms(mono):
    return float(np.sqrt(np.mean(mono ** 2)))


def top_peaks(rate, mono, n=8, threshold_rel=0.1):
    if mono.size == 0 or np.max(np.abs(mono)) < 1e-9:
        return []
    w = np.hanning(len(mono))
    spec = np.abs(np.fft.rfft(mono * w))
    freqs = np.fft.rfftfreq(len(mono), 1.0 / rate)
    if spec.max() < 1e-9:
        return []
    idx, _ = find_peaks(spec, height=spec.max() * threshold_rel, distance=10)
    idx = idx[np.argsort(-spec[idx])][:n]
    return [(float(freqs[i]), float(spec[i])) for i in idx]


def compare_window(ours_path: Path, ref_path: Path, t_start: float, t_end: float,
                   rms_low: float, rms_high: float, cents_thresh: float):
    """Returns (flag_reasons: list[str], stats: dict). Empty reasons → clean."""
    rate_o, mono_o_full = load_wav_mono(ours_path)
    rate_r, mono_r_full = load_wav_mono(ref_path)
    if rate_o != rate_r:
        return ["rate-mismatch"], {}
    s = int(t_start * rate_o)
    e = int(t_end * rate_o)
    s = max(0, s)
    e = min(min(len(mono_o_full), len(mono_r_full)), e)
    if e - s < int(0.1 * rate_o):
        return [], {}  # silent / not enough data — skip
    o = mono_o_full[s:e]
    r = mono_r_full[s:e]
    ro = rms(o)
    rr = rms(r)
    flags = []
    stats = {"t_start": t_start, "rms_ours": ro, "rms_ref": rr}

    # If both quiet, skip — no signal to compare.
    if max(ro, rr) < 0.001:
        return [], stats

    if rr > 0:
        ratio = ro / rr
        stats["ratio"] = ratio
        if ratio < rms_low or ratio > rms_high:
            flags.append(f"rms-ratio={ratio:.2f}")

    # Spectral check: top-N peaks each, find pairwise nearest, cents diff.
    p_o = top_peaks(rate_o, o, n=8)
    p_r = top_peaks(rate_o, r, n=8)
    cents_offenders = 0
    for fo, _ in p_o[:6]:
        if not p_r:
            continue
        nearest_f, _ = min(p_r, key=lambda kv: abs(kv[0] - fo))
        if nearest_f > 0 and fo > 0 and abs(nearest_f - fo) < 6.0:
            cents = 1200.0 * np.log2(nearest_f / fo)
            if abs(cents) > cents_thresh:
                cents_offenders += 1
    if cents_offenders >= 2:
        flags.append(f"pitch-shift({cents_offenders}/6 peaks > {cents_thresh}c)")

    # Missing-peak heuristic: peaks in ref that don't have a match in ours
    # within 6 Hz at a nontrivial magnitude.
    missing = 0
    for fr, mr in p_r[:6]:
        if not p_o:
            missing += 1
            continue
        nearest_f, _ = min(p_o, key=lambda kv: abs(kv[0] - fr))
        if abs(nearest_f - fr) > 6.0:
            missing += 1
    if missing >= 3:
        flags.append(f"missing-peaks({missing}/6)")

    return flags, stats


def sweep_module(args, render_bin, omt_bin, h, ext, name, file_path):
    """Returns (status: str, flags_per_window: list, summary: str)."""
    out_dir = Path(args.corpus) / "renders"
    out_dir.mkdir(exist_ok=True)
    ours_wav = out_dir / f"{h}.ours.wav"
    ref_wav = out_dir / f"{h}.ref.wav"
    if ours_wav.exists():
        ours_wav.unlink()
    if ref_wav.exists():
        ref_wav.unlink()

    if not run_render_wav(render_bin, file_path, ours_wav, args.end_time):
        return "ours-render-failed", [], ""
    if not run_openmpt(omt_bin, file_path, ref_wav):
        return "openmpt-render-failed", [], ""

    # Walk 1.0s windows at 5s intervals across the rendered region.
    rate, mono = load_wav_mono(ours_wav)
    duration = len(mono) / rate
    flagged = []
    sample_starts = list(np.arange(5.0, min(duration, args.end_time) - 1.0, 5.0))
    for ts in sample_starts:
        flags, stats = compare_window(
            ours_wav, ref_wav, ts, ts + 1.0,
            args.rms_low, args.rms_high, args.cents_thresh,
        )
        if flags:
            flagged.append((ts, flags, stats))

    return ("clean" if not flagged else "flagged"), flagged, ""


def render_report(args, results):
    """results: list of (h, ext, name, status, flagged)."""
    out = []
    out.append(f"# Sweep report\n")
    out.append(f"_seed={args.seed} limit={args.limit} end-time={args.end_time}_\n")
    flagged = [r for r in results if r[3] == "flagged"]
    clean = [r for r in results if r[3] == "clean"]
    failed = [r for r in results if r[3] not in ("clean", "flagged")]
    out.append(f"\n* {len(results)} modules processed\n")
    out.append(f"* {len(flagged)} flagged, {len(clean)} clean, {len(failed)} failed\n\n")

    if flagged:
        out.append("## Flagged\n\n")
        # Sort by severity (number of flagged windows then worst RMS ratio).
        def sev(r):
            _, _, _, _, fl = r
            n = len(fl)
            worst = max(
                (abs(np.log(stats.get("ratio", 1.0))) for _, _, stats in fl),
                default=0.0,
            )
            return (-n, -worst)
        flagged.sort(key=sev)
        for h, ext, name, _, fl in flagged:
            out.append(f"### `{name}` ({ext}, hash {h})\n\n")
            out.append("| t (s) | RMS ours | RMS ref | ratio | flags |\n")
            out.append("|---:|---:|---:|---:|---|\n")
            for ts, flags, stats in fl[:12]:
                ro = stats.get("rms_ours", 0.0)
                rr = stats.get("rms_ref", 0.0)
                ratio = stats.get("ratio", float("nan"))
                out.append(f"| {ts:.1f} | {ro:.4f} | {rr:.4f} | {ratio:.2f} | {'; '.join(flags)} |\n")
            if len(fl) > 12:
                out.append(f"\n_(+{len(fl)-12} more windows)_\n")
            out.append("\n")

    if failed:
        out.append("## Failed\n\n")
        for h, ext, name, status, _ in failed:
            out.append(f"* `{name}` ({ext}, hash {h}) — {status}\n")
        out.append("\n")

    if clean:
        out.append(f"## Clean ({len(clean)})\n\n")
        for h, ext, name, _, _ in clean[:50]:
            out.append(f"* `{name}` ({ext}, hash {h})\n")

    return "".join(out)


def main():
    args = parse_args()
    if args.report is None:
        args.report = str(Path(args.corpus) / "report.md")
    if args.seed is None:
        args.seed = random.randrange(0, 1 << 30)
    random.seed(args.seed)

    corpus = Path(args.corpus)
    if not corpus.exists():
        print(f"corpus not found: {corpus}", file=sys.stderr)
        sys.exit(1)

    manifest = load_manifest(corpus)
    if not manifest:
        print("manifest is empty — run tools/build_corpus.sh first", file=sys.stderr)
        sys.exit(1)

    done = set() if args.force else load_done(corpus)
    available = [m for m in manifest if m[0] not in done]
    if not available:
        print("nothing left to sweep — pass --force to re-run", file=sys.stderr)
        sys.exit(0)

    random.shuffle(available)
    picks = available[:args.limit]

    render_bin = find_render_wav()
    omt_bin = find_openmpt123()

    results = []
    for h, ext, name, fp in picks:
        print(f"=== {name} ({ext}, hash {h}) ===", file=sys.stderr)
        status, flagged, _ = sweep_module(args, render_bin, omt_bin, h, ext, name, fp)
        results.append((h, ext, name, status, flagged))
        append_done(corpus, h, status)
        print(f"  → {status} ({len(flagged)} flagged windows)", file=sys.stderr)

    report = render_report(args, results)
    Path(args.report).write_text(report)
    print(f"\nreport written to {args.report}", file=sys.stderr)
    print(f"seed = {args.seed} (pass --seed {args.seed} to reproduce)", file=sys.stderr)


if __name__ == "__main__":
    main()
