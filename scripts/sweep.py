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
    p.add_argument("--bisect", action="store_true",
                   help="For each flagged module, render once per channel "
                        "with that channel muted to attribute the divergence "
                        "to specific channels. Adds N_channels renders per "
                        "flagged module; skip with --no-bisect for speed.")
    p.add_argument("--bisect-max-channels", type=int, default=32,
                   help="Skip bisect on modules with more channels than this "
                        "(too many renders).")
    p.add_argument("--solo-bisect", action="store_true",
                   help="Per-channel solo on BOTH our engine and libopenmpt "
                        "(via the openmpt_solo tool). For each channel: "
                        "compare ours_solo vs ref_solo directly, computing "
                        "per-channel rms and rms(ours - ref) divergence — "
                        "the smoking-gun diagnostic that mute-out can't "
                        "give. Implies --bisect (channel attribution).")
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


def find_openmpt_solo() -> Path:
    """libopenmpt-linked solo renderer for per-channel attribution. Built
    by tools/build_openmpt_solo.sh — we don't auto-build because that
    needs cc + libopenmpt headers; better to fail clearly if missing.
    """
    p = REPO / "target" / "release" / "openmpt_solo"
    if not p.exists():
        raise FileNotFoundError(
            f"openmpt_solo not built. Run tools/build_openmpt_solo.sh"
        )
    return p


def run_openmpt_solo(bin: Path, module: Path, out: Path, end_time: float,
                     solo_channel: int):
    """Render a single channel via the libopenmpt extension interface.
    `solo_channel` muting all others. Output filename is <out>."""
    try:
        subprocess.run(
            [str(bin), str(module), str(out),
             "--solo", str(solo_channel),
             "--end-time", str(end_time)],
            check=True, capture_output=True, timeout=end_time * 4 + 30,
        )
        return out.exists() and out.stat().st_size > 1000
    except Exception as e:
        print(f"  openmpt_solo failed for {module} ch={solo_channel}: {e}",
              file=sys.stderr)
        return False


def run_render_wav(bin: Path, module: Path, out: Path, end_time: float,
                   mute_channels=None):
    """Render via our engine. Returns True on success."""
    cmd = [str(bin), str(module), str(out), "--end-time", str(end_time)]
    if mute_channels:
        cmd.extend(["--mute-channels", ",".join(str(c) for c in mute_channels)])
    try:
        subprocess.run(
            cmd, check=True, capture_output=True, timeout=end_time * 4 + 30,
        )
        return out.exists() and out.stat().st_size > 1000
    except Exception as e:
        print(f"  render_wav failed for {module}: {e}", file=sys.stderr)
        return False


def detect_channel_count(module: Path) -> int:
    """Best-effort channel count by reading the module's header. Used to
    bound the bisect render count without parsing the full file. Returns
    -1 on unknown formats."""
    try:
        data = module.read_bytes()[:200]
    except Exception:
        return -1
    suffix = module.suffix.lower()
    if suffix == ".s3m":
        # S3M: 32-byte channel-table at 0x40; count entries < 16 (PCM only;
        # disabled / Adlib / unused don't get engine slots).
        if len(data) < 0x60: return -1
        return sum(1 for b in data[0x40:0x60] if b < 16)
    if suffix == ".xm":
        # XM: u16 LE channel count at 0x44.
        if len(data) < 0x46: return -1
        return data[0x44] | (data[0x45] << 8)
    if suffix == ".mod":
        # Hand-wave: 4 channels for the canonical .MOD; 6/8/12-ch trackers
        # use different magic words. Good enough for bisect bounding.
        return 4
    if suffix == ".it":
        # IT: u16 LE channel count at offset 0x24.
        if len(data) < 0x26: return -1
        return data[0x24] | (data[0x25] << 8)
    return -1


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
                   rms_low: float, rms_high: float, cents_thresh: float,
                   n_channels: int = 8):
    """Returns (flag_reasons: list[str], stats: dict). Empty reasons → clean.

    `n_channels` scales the spectral peak budget. With ~8 channels each
    contributing fundamental + 2-3 strong harmonics, top-N peaks should
    be at least ~3-4× channel count. Using 12 fixed missed real
    divergences on 32-channel XMs (a quiet broken channel's peaks never
    made the top-12 set when 32 active channels each had louder peaks).
    Cap at 96 — diminishing returns past that, and the magnitude
    threshold filter still drops noise."""
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

    # Spectral check: scale the candidate-peak budget by channel count,
    # bounded to [12, 96]. Rationale: each active channel contributes a
    # fundamental + 2-3 harmonics; with 32 active channels, top-12 peaks
    # is < 1 peak/channel which silently misses divergences in quieter
    # channels. We compare top-N "primary" peaks (= 1.5 × channels,
    # bounded too) against the wider candidate set so each primary peak
    # has a fair chance of finding its match.
    n_candidates = max(12, min(96, n_channels * 4))
    n_primary = max(6, min(48, int(n_channels * 1.5)))
    p_o = top_peaks(rate_o, o, n=n_candidates)
    p_r = top_peaks(rate_o, r, n=n_candidates)
    primary_o = p_o[:n_primary]
    primary_r = p_r[:n_primary]
    cents_offenders = 0
    cents_list = []
    for fo, _ in primary_o:
        if not p_r:
            continue
        # Match by frequency proximity (smallest absolute Hz delta) across
        # the WIDER ref-peak set, but only count it as a "matched peak" if
        # within 6 Hz — a missed match is a missing-peak case, not pitch.
        nearest_f, _ = min(p_r, key=lambda kv: abs(kv[0] - fo))
        if nearest_f > 0 and fo > 0 and abs(nearest_f - fo) < 6.0:
            cents = 1200.0 * np.log2(nearest_f / fo)
            cents_list.append(cents)
            if abs(cents) > cents_thresh:
                cents_offenders += 1
    stats["cents"] = cents_list
    # Threshold scales: a 32-channel module flagging on 2/48 peaks is
    # noise; we want the rate to scale.
    cents_threshold = max(2, n_primary // 3)
    if cents_offenders >= cents_threshold:
        flags.append(f"pitch-shift({cents_offenders}/{n_primary} peaks > {cents_thresh}c)")

    # Missing-peak heuristic: scaled equivalently. >= half of primary
    # peaks in the ref missing in ours.
    missing = 0
    for fr, mr in primary_r:
        if not p_o:
            missing += 1
            continue
        nearest_f, _ = min(p_o, key=lambda kv: abs(kv[0] - fr))
        if abs(nearest_f - fr) > 6.0:
            missing += 1
    missing_threshold = max(3, n_primary // 2)
    if missing >= missing_threshold:
        flags.append(f"missing-peaks({missing}/{n_primary})")

    return flags, stats


def solo_bisect_module(args, render_bin, solo_bin, h, file_path, flagged,
                       out_dir, n_channels):
    """For one module, render each channel solo with both engines, then
    compute per-channel divergence for the WORST-flagged window. Returns
    a dict { channel_idx: {ours_rms, ref_rms, diff_rms, peaks_match} }
    where diff_rms = RMS of (ours_solo - ref_solo) over the window. A
    high diff_rms is a direct attribution of the bug to that channel.

    Cost: 2 × N_channels renders (ours + ref) per bisected module."""
    # Pick the most-divergent flagged window.
    worst = None
    for ts, flags, stats in flagged:
        ratio = stats.get("ratio", 1.0) or 1.0
        sev = abs(np.log(max(ratio, 1e-9)))
        if worst is None or sev > worst[1]:
            worst = (ts, sev, stats)
    if worst is None:
        return {}
    target_ts, _sev, _stats = worst

    out = {}
    print(f"  solo-bisecting {n_channels} channels at t={target_ts:.1f}s ...",
          file=sys.stderr)
    for ch in range(n_channels):
        ours_wav = out_dir / f"{h}.solo{ch}.ours.wav"
        ref_wav = out_dir / f"{h}.solo{ch}.ref.wav"
        for w in (ours_wav, ref_wav):
            if w.exists(): w.unlink()

        # Our engine: solo via mute-all-others with --mute-channels.
        all_other = [c for c in range(n_channels) if c != ch]
        if not run_render_wav(render_bin, file_path, ours_wav, args.end_time,
                              mute_channels=all_other):
            continue
        # libopenmpt: native --solo via the ext interactive interface.
        if not run_openmpt_solo(solo_bin, file_path, ref_wav, args.end_time, ch):
            ours_wav.unlink(missing_ok=True)
            continue

        # Compare the two solo renders at the target window. We use a
        # tighter window (1.0s) here because a single channel rarely
        # has dense low-frequency content that needs the 2.0s
        # resolution; tighter window = sharper transient capture.
        try:
            ro, mo = load_wav_mono(ours_wav)
            rr, mr = load_wav_mono(ref_wav)
            if ro != rr:
                continue
            s = int(target_ts * ro)
            e = s + int(1.0 * ro)
            e = min(e, min(len(mo), len(mr)))
            if e - s < int(0.05 * ro):
                continue
            o = mo[s:e]; r = mr[s:e]
            n = min(len(o), len(r))
            o = o[:n]; r = r[:n]
            ours_rms = rms(o)
            ref_rms = rms(r)
            diff = o - r
            diff_rms = rms(diff)
            # Peak comparison: does the channel play at the same pitch?
            p_o = top_peaks(ro, o, n=4)
            p_r = top_peaks(ro, r, n=4)
            peaks_match = "—"
            if p_o and p_r:
                # Top peak frequency ratio in cents (signed).
                cents = 1200.0 * np.log2(p_o[0][0] / p_r[0][0]) if p_o[0][0] > 0 and p_r[0][0] > 0 else 0
                peaks_match = f"{p_o[0][0]:.0f}Hz vs {p_r[0][0]:.0f}Hz ({cents:+.0f}c)"
            out[ch] = {
                "ours_rms": ours_rms,
                "ref_rms": ref_rms,
                "diff_rms": diff_rms,
                "peaks_match": peaks_match,
            }
        finally:
            ours_wav.unlink(missing_ok=True)
            ref_wav.unlink(missing_ok=True)

    return out


def sweep_module(args, render_bin, omt_bin, h, ext, name, file_path):
    """Returns (status, flags_per_window, attribution_dict)."""
    out_dir = Path(args.corpus) / "renders"
    out_dir.mkdir(exist_ok=True)
    ours_wav = out_dir / f"{h}.ours.wav"
    ref_wav = out_dir / f"{h}.ref.wav"
    if ours_wav.exists():
        ours_wav.unlink()
    if ref_wav.exists():
        ref_wav.unlink()

    if not run_render_wav(render_bin, file_path, ours_wav, args.end_time):
        return "ours-render-failed", [], {}
    if not run_openmpt(omt_bin, file_path, ref_wav):
        return "openmpt-render-failed", [], {}

    # Walk 2.0s windows at 5s intervals. 2s gives 0.5 Hz FFT bin width
    # — ~8 cents resolution at 100 Hz vs the 1.0s window's ~17 cents.
    # Pre-fix the harness was reporting pitch-shift on bin-edge peaks
    # that were actually within tolerance.
    rate, mono = load_wav_mono(ours_wav)
    duration = len(mono) / rate
    flagged = []
    win = 2.0
    n_channels = max(1, detect_channel_count(file_path))
    sample_starts = list(np.arange(5.0, min(duration, args.end_time) - win, 5.0))
    for ts in sample_starts:
        flags, stats = compare_window(
            ours_wav, ref_wav, ts, ts + win,
            args.rms_low, args.rms_high, args.cents_thresh,
            n_channels=n_channels,
        )
        if flags:
            flagged.append((ts, flags, stats))

    if not flagged:
        return "clean", [], {}

    attribution = {}
    if args.bisect or args.solo_bisect:
        attribution = bisect_module(args, render_bin, h, file_path,
                                    flagged, ours_wav, ref_wav, out_dir)

    if args.solo_bisect:
        n_channels = detect_channel_count(file_path)
        if n_channels > 0 and n_channels <= args.bisect_max_channels:
            try:
                solo_bin = find_openmpt_solo()
                solo_attr = solo_bisect_module(args, render_bin, solo_bin, h,
                                                file_path, flagged, out_dir,
                                                n_channels)
                if solo_attr:
                    attribution["__solo__"] = solo_attr
            except FileNotFoundError as e:
                print(f"  solo-bisect skipped: {e}", file=sys.stderr)

    return "flagged", flagged, attribution


def bisect_module(args, render_bin, h, file_path, flagged, ours_wav, ref_wav,
                  out_dir):
    """For each flagged window, render with each channel muted in turn and
    record which channels' mute clears the flag. Returns a dict
        { window_start_seconds: [list of (channel_idx, residual_severity)] }
    where severity = abs(log(rms_ratio)) so 0 means perfectly cleared."""
    n_channels = detect_channel_count(file_path)
    if n_channels <= 0 or n_channels > args.bisect_max_channels:
        return {}

    # Pick the worst window per (sign-of-divergence) bucket so we cover
    # both "too-loud" and "too-quiet" modules with one bisect pass each.
    by_sign = {"loud": None, "quiet": None}
    for ts, _flags, stats in flagged:
        ratio = stats.get("ratio", 1.0)
        if ratio is None:
            continue
        sev = abs(np.log(max(ratio, 1e-9)))
        bucket = "loud" if ratio > 1.0 else "quiet"
        cur = by_sign[bucket]
        if cur is None or sev > cur[1]:
            by_sign[bucket] = (ts, sev, stats)

    target_windows = [v for v in by_sign.values() if v is not None]
    if not target_windows:
        return {}

    out = {}
    print(f"  bisecting {n_channels} channels...", file=sys.stderr)
    for ch in range(n_channels):
        muted_wav = out_dir / f"{h}.mute{ch}.wav"
        if muted_wav.exists():
            muted_wav.unlink()
        if not run_render_wav(render_bin, file_path, muted_wav,
                              args.end_time, mute_channels=[ch]):
            continue
        for ts, sev, stats in target_windows:
            flags_after, stats_after = compare_window(
                muted_wav, ref_wav, ts, ts + 2.0,
                args.rms_low, args.rms_high, args.cents_thresh,
                n_channels=n_channels,
            )
            ratio_after = stats_after.get("ratio", 1.0)
            sev_after = abs(np.log(max(ratio_after, 1e-9))) if ratio_after else 0.0
            # If muting this channel cleared most of the divergence (severity
            # dropped by > 50% AND no flags remain), it's a culprit.
            improvement = sev - sev_after
            cleared = (not flags_after)
            entry = out.setdefault(ts, [])
            entry.append({
                "channel": ch,
                "ratio_before": stats.get("ratio"),
                "ratio_after": ratio_after,
                "improvement": improvement,
                "cleared": cleared,
                "flags_after": flags_after,
            })
        muted_wav.unlink(missing_ok=True)
    return out


def render_report(args, results):
    """results: list of (h, ext, name, status, flagged, attribution)."""
    out = []
    out.append(f"# Sweep report\n")
    out.append(f"_seed={args.seed} limit={args.limit} end-time={args.end_time} bisect={args.bisect}_\n")
    flagged = [r for r in results if r[3] == "flagged"]
    clean = [r for r in results if r[3] == "clean"]
    failed = [r for r in results if r[3] not in ("clean", "flagged")]
    out.append(f"\n* {len(results)} modules processed\n")
    out.append(f"* {len(flagged)} flagged, {len(clean)} clean, {len(failed)} failed\n\n")

    # Format rollup — easiest way to spot systemic per-format bugs (e.g.
    # all MOD modules flagged with the same RMS ratio + pitch shift would
    # show up as a tight median ratio in the MOD row).
    rollup = {}
    for h, ext, name, status, fl, _attr in results:
        bucket = rollup.setdefault(ext, {"total": 0, "flagged": 0, "ratios": []})
        bucket["total"] += 1
        if status == "flagged":
            bucket["flagged"] += 1
            for _ts, _flags, stats in fl:
                ratio = stats.get("ratio")
                if ratio:
                    bucket["ratios"].append(ratio)
    out.append("## By format\n\n")
    out.append("| format | total | flagged | median ratio | min | max |\n")
    out.append("|---|---:|---:|---:|---:|---:|\n")
    for ext in sorted(rollup.keys()):
        b = rollup[ext]
        ratios = b["ratios"]
        if ratios:
            md = float(np.median(ratios))
            lo = min(ratios); hi = max(ratios)
            out.append(f"| {ext} | {b['total']} | {b['flagged']} | {md:.2f} | {lo:.2f} | {hi:.2f} |\n")
        else:
            out.append(f"| {ext} | {b['total']} | {b['flagged']} | — | — | — |\n")
    out.append("\n")

    if flagged:
        out.append("## Flagged\n\n")
        def sev(r):
            _, _, _, _, fl, _attr = r
            n = len(fl)
            worst = max(
                (abs(np.log(stats.get("ratio", 1.0))) for _, _, stats in fl),
                default=0.0,
            )
            return (-n, -worst)
        flagged.sort(key=sev)
        for h, ext, name, _, fl, attr in flagged:
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
            # Attribution rollup, if bisect was run.
            if attr:
                solo = attr.pop("__solo__", None) if isinstance(attr, dict) else None
                if attr:
                    out.append("\n**Channel attribution (mute-bisect):**\n\n")
                    for ts in sorted(attr.keys()):
                        cleared = sorted(
                            (e for e in attr[ts] if e["cleared"] or e["improvement"] > 0.5),
                            key=lambda e: -e["improvement"],
                        )
                        if not cleared:
                            out.append(f"* t={ts:.1f}s: no single channel mute clears the flag (multi-channel cause)\n")
                            continue
                        parts = []
                        for e in cleared[:5]:
                            tag = "cleared" if e["cleared"] else f"-{e['improvement']:.1f}log"
                            parts.append(f"ch{e['channel']} ({tag}, ratio {e['ratio_before']:.2f}→{e['ratio_after']:.2f})")
                        out.append(f"* t={ts:.1f}s: {', '.join(parts)}\n")

                if solo:
                    # Per-channel diagnostic: for each channel, ours-solo
                    # vs ref-solo. Sort by diff_rms so the most-divergent
                    # channels appear first.
                    out.append("\n**Per-channel solo (ours vs OpenMPT, both solo):**\n\n")
                    out.append("| ch | ours rms | ref rms | diff rms | top peaks (cents) |\n")
                    out.append("|---:|---:|---:|---:|---|\n")
                    rows = sorted(solo.items(), key=lambda kv: -(kv[1].get("diff_rms") or 0))
                    for ch, st in rows[:12]:
                        out.append(
                            f"| {ch} | {st['ours_rms']:.4f} | {st['ref_rms']:.4f} | {st['diff_rms']:.4f} | {st['peaks_match']} |\n"
                        )
                    if len(rows) > 12:
                        out.append(f"\n_(+{len(rows)-12} more channels)_\n")
            out.append("\n")

    if failed:
        out.append("## Failed\n\n")
        for h, ext, name, status, _, _ in failed:
            out.append(f"* `{name}` ({ext}, hash {h}) — {status}\n")
        out.append("\n")

    if clean:
        out.append(f"## Clean ({len(clean)})\n\n")
        for h, ext, name, _, _, _ in clean[:50]:
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
        status, flagged, attribution = sweep_module(args, render_bin, omt_bin, h, ext, name, fp)
        results.append((h, ext, name, status, flagged, attribution))
        append_done(corpus, h, status)
        print(f"  → {status} ({len(flagged)} flagged windows)", file=sys.stderr)

    report = render_report(args, results)
    Path(args.report).write_text(report)
    print(f"\nreport written to {args.report}", file=sys.stderr)
    print(f"seed = {args.seed} (pass --seed {args.seed} to reproduce)", file=sys.stderr)


if __name__ == "__main__":
    main()
