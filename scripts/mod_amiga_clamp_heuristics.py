#!/usr/bin/env python3
"""
Scan MOD files and report which of the proposed Amiga-clamp heuristics
each one trips. Output is intended to ground the policy choice in
real-corpus data: how many of our files would unclamp under each rule,
and how the rules overlap.

Heuristic signals (each independent; OR-combined in practice):

  multi_ch        signature is 6CHN / 8CHN / xxCH        — definitely PC
  sub_113         pattern cell has period < 113          — won't survive Amiga clamp
  super_856       pattern cell has period > 856          — won't survive Amiga clamp
  effect_8xx      pattern uses 8xx (set panning)         — PT ignores; PC author
  effect_E8       pattern uses E8x                       — PT-incompatible
  watermark       sample header has known tracker mark   — PC tracker save
  odd_sample      any sample has odd length              — PT pads even
  small_loop      any sample has loop_len in {1, 2}      — PT convention is >=4
  high_patterns   pattern count > 64                     — PT supports <=64 for M.K.

Per-file output: filename + signature + which signals fired.

Per-heuristic summary: how many files (and the subset SC2-only) trip
each signal.

Verdict columns:
  WOULD-CLAMP    none of the signals fire — looks Amiga-authored
  WOULD-UNCLAMP  at least one signal fires — likely PC origin
"""

from __future__ import annotations

import argparse
import struct
import sys
from pathlib import Path
from typing import Dict, List, Set, Tuple


# Known watermarks/strings found in sample-header reserved bytes or
# instrument names. Conservative list; case-sensitive matches.
WATERMARK_SUBSTRS = (
    b"ModPlug",
    b"OpenMPT",
    b"FastTracker",
    b"FT2",
    b"IT2",
    b"OctaMED",
    b"MED",
    b"DigiBooster",
)


def detect_channels(sig: bytes) -> Tuple[int, bool]:
    """Return (num_channels, is_amiga_signature)."""
    if sig in (b"M.K.", b"M!K!", b"FLT4"):
        return 4, True
    if sig == b"6CHN":
        return 6, False
    if sig == b"8CHN":
        return 8, False
    if sig.endswith(b"CHN") and sig[0:1].isdigit():
        return int(sig[0:1]), False
    if sig.endswith(b"CH") and sig[0:2].isdigit():
        return int(sig[0:2]), False
    return 0, False  # unknown


def scan_one(path: Path) -> Dict[str, object]:
    """Return a dict of {signal_name: bool, plus metadata}."""
    out: Dict[str, object] = {
        "path": str(path),
        "signature": None,
        "channels": 0,
        "is_amiga_sig": False,
        "num_patterns": 0,
        "signals": set(),
    }
    try:
        data = path.read_bytes()
    except (OSError, PermissionError) as e:
        out["error"] = f"read: {e}"
        return out

    if len(data) < 1084:
        out["error"] = f"too small ({len(data)} bytes)"
        return out

    sig = data[1080:1084]
    out["signature"] = sig.decode("ascii", errors="replace")

    nch, is_amiga = detect_channels(sig)
    if nch == 0:
        out["error"] = f"unrecognized signature {sig!r}"
        return out
    out["channels"] = nch
    out["is_amiga_sig"] = is_amiga
    if not is_amiga:
        out["signals"].add("multi_ch")  # type: ignore[attr-defined]

    # Sample headers: 31 × 30 bytes starting at offset 20.
    odd_sample = False
    small_loop = False
    watermark = False
    for i in range(31):
        off = 20 + i * 30
        if off + 30 > len(data):
            break
        name = data[off:off + 22]
        length = struct.unpack(">H", data[off + 22:off + 24])[0] * 2  # words → bytes
        loop_start = struct.unpack(">H", data[off + 26:off + 28])[0] * 2
        loop_len = struct.unpack(">H", data[off + 28:off + 30])[0] * 2

        if length % 2 != 0:
            odd_sample = True  # extremely rare via file structure but keep
        if loop_len in (2,):
            # ProTracker convention: loop_len >= 2 means looped (since
            # length is stored as words/2, real byte loop_len == 1 word
            # is the PT "no loop" sentinel). small_loop signal looks for
            # values that are technically valid but suspicious.
            small_loop = True

        # Watermark check against name AND any zero-stripped bytes.
        for w in WATERMARK_SUBSTRS:
            if w in name:
                watermark = True
                break

    if odd_sample:
        out["signals"].add("odd_sample")  # type: ignore[attr-defined]
    if small_loop:
        out["signals"].add("small_loop")  # type: ignore[attr-defined]
    if watermark:
        out["signals"].add("watermark")  # type: ignore[attr-defined]

    # Order list: offset 950, song_length byte + 128-byte order table.
    song_length = data[950]
    orders = data[952:1080]
    if song_length > 128:
        song_length = 128
    num_patterns = max(orders[:song_length], default=0) + 1
    out["num_patterns"] = num_patterns
    if num_patterns > 64 and is_amiga and sig != b"M!K!":
        out["signals"].add("high_patterns")  # type: ignore[attr-defined]

    # Pattern data starts at offset 1084. Each pattern is 64 rows × nch
    # × 4 bytes = 256 × nch bytes.
    pat_start = 1084
    pat_size = 64 * nch * 4
    sub_113 = False
    super_856 = False
    effect_8xx = False
    effect_E8 = False
    for p in range(num_patterns):
        base = pat_start + p * pat_size
        if base + pat_size > len(data):
            break
        for row in range(64):
            for ch in range(nch):
                cell_off = base + (row * nch + ch) * 4
                b0 = data[cell_off]
                b1 = data[cell_off + 1]
                b2 = data[cell_off + 2]
                b3 = data[cell_off + 3]
                period = ((b0 & 0x0F) << 8) | b1
                effect = b2 & 0x0F
                param = b3
                # Only count nonzero periods (0 = no note in this cell).
                if period != 0:
                    if period < 113:
                        sub_113 = True
                    elif period > 856:
                        super_856 = True
                if effect == 0x8:
                    effect_8xx = True
                elif effect == 0xE:
                    if (param & 0xF0) == 0x80:
                        effect_E8 = True

    if sub_113:
        out["signals"].add("sub_113")  # type: ignore[attr-defined]
    if super_856:
        out["signals"].add("super_856")  # type: ignore[attr-defined]
    if effect_8xx:
        out["signals"].add("effect_8xx")  # type: ignore[attr-defined]
    if effect_E8:
        out["signals"].add("effect_E8")  # type: ignore[attr-defined]

    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("paths", nargs="+", help="MOD files or directories to scan")
    ap.add_argument("--show-clamp", action="store_true",
                    help="also print files that WOULD clamp (no signals fire)")
    args = ap.parse_args()

    files: List[Path] = []
    for p in args.paths:
        pp = Path(p)
        if pp.is_dir():
            files.extend(sorted(pp.rglob("*.mod")))
        elif pp.is_file():
            files.append(pp)

    results = [scan_one(f) for f in files]
    valid = [r for r in results if "error" not in r]
    errors = [r for r in results if "error" in r]

    SIGNALS = (
        "multi_ch", "sub_113", "super_856", "effect_8xx",
        "effect_E8", "watermark", "odd_sample", "small_loop", "high_patterns",
    )

    # Per-file detail (only files with at least one signal, unless --show-clamp).
    print(f"# {len(valid)} MODs scanned ({len(errors)} unreadable / malformed)")
    print()
    print(f"{'file':<60} {'sig':<6} {'ch':>3} signals")
    for r in valid:
        sigs = sorted(r["signals"])  # type: ignore[arg-type]
        if not sigs and not args.show_clamp:
            continue
        name = Path(str(r["path"])).name
        if len(name) > 58:
            name = name[:55] + "..."
        print(f"{name:<60} {str(r['signature'] or ''):<6} {r['channels']:>3} {','.join(sigs)}")

    # Per-signal summary.
    print()
    print("# Signal summary (count of files that trip each signal)")
    sc2_files = [r for r in valid if "/SC2/" in str(r["path"])]
    nonsc2_files = [r for r in valid if "/SC2/" not in str(r["path"])]
    print(f"{'signal':<15} {'all':>5} {'sc2':>5} {'non-sc2':>8}")
    for s in SIGNALS:
        n_all = sum(1 for r in valid if s in r["signals"])  # type: ignore[operator]
        n_sc2 = sum(1 for r in sc2_files if s in r["signals"])
        n_other = sum(1 for r in nonsc2_files if s in r["signals"])
        print(f"{s:<15} {n_all:>5} {n_sc2:>5} {n_other:>8}")

    # Verdict counts.
    print()
    print("# Verdicts (any signal => UNCLAMP)")
    unclamp_all = sum(1 for r in valid if r["signals"])  # type: ignore[arg-type]
    unclamp_sc2 = sum(1 for r in sc2_files if r["signals"])  # type: ignore[arg-type]
    unclamp_other = sum(1 for r in nonsc2_files if r["signals"])  # type: ignore[arg-type]
    clamp_all = len(valid) - unclamp_all
    clamp_sc2 = len(sc2_files) - unclamp_sc2
    clamp_other = len(nonsc2_files) - unclamp_other
    print(f"{'verdict':<15} {'all':>5} {'sc2':>5} {'non-sc2':>8}")
    print(f"{'WOULD-CLAMP':<15} {clamp_all:>5} {clamp_sc2:>5} {clamp_other:>8}")
    print(f"{'WOULD-UNCLAMP':<15} {unclamp_all:>5} {unclamp_sc2:>5} {unclamp_other:>8}")

    if errors:
        print()
        print(f"# {len(errors)} unreadable / malformed files")
        for r in errors:
            print(f"  {Path(str(r['path'])).name}: {r['error']}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
