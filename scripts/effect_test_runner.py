#!/usr/bin/env python3
"""
Per-effect regression-test runner.

For each fixture in `scratch/effect_tests/*.{xm,mod,s3m,it}`:
  * For each channel N in [0, num_channels):
    * Render our engine with OUR_DUMP_CH=N -> our trace
    * Render the canonical engine with FT2_DUMP_CH=N (or equivalent) -> ref trace
  * Diff the two traces by (ord, row, tick, ch) tuple, comparing only
    user-facing fields:
        period   (OUR `period`        <-> FT2 `finalPeriod`)   tol = +-1
        vraw     (OUR `vraw`          <-> FT2 `realVol`)       tol = 0
  * Report first divergence per (fixture, channel).

Design notes:
  * OUR trace is streamed line-by-line (sparse — only voice-on ticks).
  * FT2 trace is loaded into a per-(ord,row,tick,ch) FIFO; pattern
    loops (E6x) make the stream non-monotonic in the key, so we
    can't just walk both in lockstep. A 30-second render typically
    produces a few hundred KB; even a 5-min song stays under ~5 MB.
  * Our trace only emits lines while `voice.on` is true; FT2 emits
    every tick. We compare only on the intersection of (ord,row,tick).
  * Other fields (envelopes, fadeout, internal envelope amplitudes,
    `outVol`, `finalVol`, `outPan`/`finalPan`) are intentionally
    skipped: they're either scaled differently between engines or
    expose mixer-internal state that legitimately diverges.

Currently only XM is wired up (ft2play is the canonical for that
format). MOD/S3M/IT need pt2-clone/st3play/it2play canonical traces;
those binaries don't expose per-tick channel state yet, so this
runner skips them with an informative message.
"""

from __future__ import annotations

import argparse
import os
import re
import struct
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, Iterator, Optional, Tuple

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_FIXTURES = ROOT / "scratch" / "effect_tests"
RENDER_WAV = ROOT / "target" / "release" / "render_wav"

# Canonical binaries by extension.
CANONICAL = {
    ".xm":  Path("/tmp/ft2play/ft2play-cli"),
    ".mod": Path("/tmp/pt2-clone/pt2-clone-cli"),
    ".s3m": Path("/tmp/st3play/st3play-cli"),
    ".it":  Path("/tmp/it2play/it2play-cli"),
}
# Env var that gates the per-tick stderr trace in each canonical CLI.
DUMP_ENV = {
    ".xm":  "FT2_DUMP_CH",
    ".mod": "PT2_DUMP_CH",
    ".s3m": "ST3_DUMP_CH",
    ".it":  "IT2_DUMP_CH",
}
# Whether the canonical binary actually supports the per-tick dump.
# Only ft2play has the instrumentation patch applied today.
HAS_TRACE = {
    ".xm":  True,
    ".mod": True,
    ".s3m": True,
    ".it":  True,
}

OUR_RE = re.compile(
    r"\[OUR\] ord=(\d+) row=(\d+) tick=(\d+) ch=(\d+) note=(-?\d+) "
    r"period=(-?\d+) vib_shift=(-?\d+) period_shift=(-?\d+) freq=(-?\d+).*?vraw=(-?\d+)"
)
FT2_RE = re.compile(
    r"\[FT2\] ord=(\d+) row=(\d+) tick=(\d+) ch=(\d+).*?"
    r"finalPeriod=(\d+).*?realVol=(\d+)"
)
# pt2-clone trace format (tools/pt2_clone_instrumentation.patch).
# MOD has no envelopes / auto-vib, so the per-tick state is minimal:
# the analogue of FT2's `finalPeriod` is `period`, and `volume` is
# the channel's running vol (0..64). MOD-side period maps to OUR's
# `period` field directly (both are raw Amiga periods).
PT2_RE = re.compile(
    r"\[PT2\] ord=(\d+) row=(\d+) tick=(\d+) ch=(\d+).*?"
    r"period=(-?\d+) volume=(-?\d+)"
)
# st3play trace format (tools/st3play_instrumentation.patch).
# S3M's `aspd` is the playing period (Amiga-like units), `avol` the
# channel volume (0..64). Order/row/tick captured BEFORE st3play's
# post-tick row-advance so the axis matches OUR's tick=0=readNewNote.
ST3_RE = re.compile(
    r"\[ST3\] ord=(\d+) row=(\d+) tick=(\d+) ch=(\d+).*?"
    r"period=(-?\d+) volume=(-?\d+)"
)
# it2play trace format (tools/it2play_instrumentation.patch).
# IT's `period` field here is actually slaveChn_t::Frequency — a Hz
# value, NOT an Amiga-style period like S3M/MOD/XM. OUR's `period`
# field is a period-table index. They don't compare directly without
# format-specific reconciliation; the diff will surface as a units
# mismatch on every IT tick until that calibration lands. The runner
# pipeline (regex / FIFO / ord-offset) is otherwise wired identically.
# IT2's order is 0-indexed (Song.CurrentOrder), matching OUR's
# song_position — no ord_offset needed.
IT2_RE = re.compile(
    r"\[IT2\] ord=(\d+) row=(\d+) tick=(\d+) ch=(\d+).*?"
    r"period=(-?\d+) volume=(-?\d+)"
)
# Per-canonical trace regex used by `iter_ref_trace` dispatch.
REF_RE = {
    ".xm":  FT2_RE,
    ".mod": PT2_RE,
    ".s3m": ST3_RE,
    ".it":  IT2_RE,
}


def detect_xm_channels(path: Path) -> int:
    """Read XM header to extract channel count (offset 0x44, u16le)."""
    with open(path, "rb") as f:
        f.seek(0x44)
        return struct.unpack("<H", f.read(2))[0]


def detect_mod_channels(path: Path) -> int:
    # 'M.K.' / 'M!K!' / '4CHN' / '6CHN' / '8CHN' / 'xxCH' at offset 1080.
    with open(path, "rb") as f:
        f.seek(1080)
        sig = f.read(4)
    if sig in (b"M.K.", b"M!K!", b"FLT4"):
        return 4
    if sig.endswith(b"CHN") and sig[0:1].isdigit():
        return int(sig[0:1])
    if sig.endswith(b"CH") and sig[0:2].isdigit():
        return int(sig[0:2])
    return 4


def detect_s3m_channels(path: Path) -> int:
    # S3M header: 32 channel enables at offset 64; values < 16 enabled.
    with open(path, "rb") as f:
        f.seek(64)
        chans = f.read(32)
    return sum(1 for b in chans if b < 16)


def detect_it_channels(path: Path) -> int:
    # IT header: u16 channel count at offset 36... actually IT exposes
    # nothing that simple; the cheap heuristic is to count Cpan slots
    # with the disabled-bit clear (offset 64, 64 bytes).
    with open(path, "rb") as f:
        f.seek(64)
        pans = f.read(64)
    return sum(1 for b in pans if b < 128)


def detect_channels(path: Path) -> int:
    ext = path.suffix.lower()
    if ext == ".xm":
        return detect_xm_channels(path)
    if ext == ".mod":
        return detect_mod_channels(path)
    if ext == ".s3m":
        return detect_s3m_channels(path)
    if ext == ".it":
        return detect_it_channels(path)
    return 0


@dataclass
class OurRow:
    period: int        # effective period = note + period_shift + vib_shift
    vraw: int
    period_shift: int  # arp/glissando contribution (negative = pitch up)
    vib_shift: int     # vibrato contribution
    freq: int          # Hz; already folds period_shift+vib_shift via Note::frequency


@dataclass
class FtRow:
    period: int
    vraw: int


def iter_our_trace(path: Path) -> Iterator[Tuple[Tuple[int, int, int, int], OurRow]]:
    """
    Yield (key, last_row_per_key) in stream order. Our engine can emit
    multiple [OUR] lines for the same (ord,row,tick,ch) tuple — one
    before row processing (stale state) and one after. FT2 emits a
    single line per tick reflecting post-row state. To get an
    apples-to-apples diff we collapse contiguous OUR lines with the
    same key, keeping the LAST one (= post-row).

    Stops iteration at song-loop boundary: when ord wraps back to 0
    after a strictly higher ord was seen, OR when row regresses on
    the same ord by more than a few positions (allowing E6x pattern
    loops, which jump backward by a handful of rows). Without this,
    short songs that wrap during the `--end-time` window produce
    second-pass entries that mis-align with FT2's first-pass entries
    via the diff's FIFO (e.g. retrig.xm: our voice ends on row 7,
    no dump on row 9 primary, but song-loop replay does dump).
    """
    cur_key: Optional[Tuple[int, int, int, int]] = None
    cur_row: Optional[OurRow] = None
    # Stop iteration after a clear song-loop wrap: track the highest
    # (ord, row) tuple seen and break when a new line jumps backward
    # by more than 16 rows on the same/lower ord. Pattern loops (E6x)
    # rarely jump back by that much; song wraps land on row 0 of the
    # repS order. Without this gate, fixtures whose voices end before
    # song-end (e.g. retrig.xm ch5: inst 2 sample is non-looping,
    # 60 ms long) produce stretched dump traces where our missing
    # primary-pass entries mis-align with FT2's via the FIFO.
    max_ord_row = (-1, -1)
    with open(path, "r", errors="replace") as f:
        for line in f:
            m = OUR_RE.match(line)
            if not m:
                continue
            ord_, row, tick, ch, _note, period, vib_shift, period_shift, freq, vraw = (int(x) for x in m.groups())
            cur_ord_row = (ord_, row)
            if max_ord_row > (-1, -1) and (
                ord_ < max_ord_row[0]
                or (ord_ == max_ord_row[0] and row + 16 < max_ord_row[1])
            ):
                break
            if cur_ord_row > max_ord_row:
                max_ord_row = cur_ord_row
            key = (ord_, row, tick, ch)
            if cur_key is None or key == cur_key:
                cur_key = key
                cur_row = OurRow(period=period, vraw=vraw, period_shift=period_shift, vib_shift=vib_shift, freq=freq)
                continue
            # New key — flush previous.
            assert cur_row is not None
            yield cur_key, cur_row
            cur_key = key
            cur_row = OurRow(period=period, vraw=vraw, period_shift=period_shift, vib_shift=vib_shift, freq=freq)
    if cur_key is not None and cur_row is not None:
        yield cur_key, cur_row


def iter_ref_trace(path: Path, regex: re.Pattern, transition_fixup: bool, ord_offset: int = 0) -> Iterator[Tuple[Tuple[int, int, int, int], FtRow]]:
    """Generic canonical-trace iterator. `regex` extracts (ord, row,
    tick, ch, period, vraw) groups in order. `transition_fixup`
    controls FT2-style row-transition re-keying.

    FT2's `mainPlayer` calls `getNextPos()` BEFORE the dump for the
    last tick of every row (whenever `song.timer == 1`). So FT2
    prints `(row=R+1, tick=N-1)` for what is logically row R's last
    tick — `pattPos` has already incremented. Our `[OUR]` dump uses
    `(row=R, tick=N-1)` for the same moment. Without a fixup the
    runner would mis-key every row-boundary tick and report
    spurious vraw/period divergences.

    pt2-clone uses `song->currRow` (the display-side field, updated
    via `nextPosition()`), which doesn't have FT2's increment-before-
    dump quirk — so the fixup is skipped for MOD.

    Heuristic: if a line has tick > 0 AND its row differs from the
    previous line's row, re-key the row back to the previous row.
    The very-first line is left alone.
    """
    prev_row: Optional[int] = None
    prev_ord: Optional[int] = None
    max_ord_row = (-1, -1)
    with open(path, "r", errors="replace") as f:
        for line in f:
            m = regex.match(line)
            if not m:
                continue
            ord_, row, tick, ch, period, vraw = (int(x) for x in m.groups())
            # Per-canonical order-numbering offset. st3play's np_ord is
            # 1-indexed (incremented before pattern lookup); our
            # `song_position` is 0-indexed. Subtract 1 to align axes.
            ord_ += ord_offset
            # Stop on song-loop wrap: see `iter_our_trace` for the same
            # backward-jump heuristic. Both engines wrap the song
            # naturally when `--end-time` exceeds song length; we want
            # to compare only the first pass.
            cur_ord_row = (ord_, row)
            if max_ord_row > (-1, -1) and (
                ord_ < max_ord_row[0]
                or (ord_ == max_ord_row[0] and row + 16 < max_ord_row[1])
            ):
                break
            if cur_ord_row > max_ord_row:
                max_ord_row = cur_ord_row
            fixed_row = row
            fixed_ord = ord_
            if (
                transition_fixup
                and tick > 0
                and prev_row is not None
                and (row != prev_row or ord_ != prev_ord)
            ):
                fixed_row = prev_row
                fixed_ord = prev_ord
            prev_row = row
            prev_ord = ord_
            yield (fixed_ord, fixed_row, tick, ch), FtRow(period=period, vraw=vraw)


# Backwards-compatibility alias for the XM-specific call sites.
def iter_ft2_trace(path: Path) -> Iterator[Tuple[Tuple[int, int, int, int], FtRow]]:
    return iter_ref_trace(path, FT2_RE, transition_fixup=True)


@dataclass
class Divergence:
    ord_: int
    row: int
    tick: int
    field: str
    our: int
    ref: int


def diff_traces(our_path: Path, ref_path: Path, ref_ext: str) -> Tuple[int, int, Optional[Divergence]]:
    """
    Intersect both traces on (ord,row,tick,ch). Returns
    (compared_count, intersection_count, first_divergence_or_none).

    Both engines emit trace lines in **execution order**, which means
    pattern-loop effects (E6x) can revisit the same (ord,row,tick)
    multiple times. So neither stream is monotonic in the key. To
    handle that correctly we build a FIFO per key from the FT2 side
    (the canonical), then walk OUR rows in order and pop the head of
    the FIFO for each match. The i-th OUR visit to a key matches the
    i-th FT2 visit.

    Memory cost: FT2 trace is loaded fully (typically <2MB for a 5-min
    song; we never grew larger in practice). OUR is still streamed.
    """
    # Build canonical index. Keep insertion-order per key.
    # FT2 needs the row-transition fixup (getNextPos before dump);
    # pt2-clone's `currRow` is already the visible-current row at
    # dump time, so MOD skips the fixup. st3play uses 1-indexed
    # np_ord; subtract 1 to align with our 0-indexed song_position.
    regex = REF_RE[ref_ext]
    transition_fixup = (ref_ext == ".xm")
    ord_offset = -1 if ref_ext == ".s3m" else 0
    ref_queues: Dict[Tuple[int, int, int, int], list] = {}
    for k, v in iter_ref_trace(ref_path, regex, transition_fixup, ord_offset):
        ref_queues.setdefault(k, []).append(v)

    # MOD: our internal period is stored at 4× the raw Amiga period
    # for sub-step slide resolution; pt2-clone reports the raw value.
    # Scale our period down before comparing. Also: pt2-clone's dump
    # emits `ch->n_period` (the channel's base note period, BEFORE
    # arp/vibrato shifts are applied — pt2 writes the shifted period
    # directly to the Paula register without touching n_period). Our
    # dump emits the effective period `note + period_shift + vib_shift`.
    # So for MOD we subtract our shifts and divide by 4 to get the
    # same axis as pt2. XM/S3M/IT canonicals (FT2/ST3/IT2) dump the
    # post-shift finalPeriod, which matches our effective period
    # directly. Tests:
    #   ArpWraparound.mod ch0: our effective period swings by ±25
    #   period units across arp ticks; pt2's n_period stays at 113.
    #   With this subtraction both reduce to 113. Without it we'd
    #   report Δ=25 per arp tick as a spurious bug.
    our_period_div = 4 if ref_ext == ".mod" else 1
    strip_shifts = (ref_ext == ".mod")
    # IT compares on Hz, not Amiga-style period: it2play's instrumentation
    # patch dumps `slaveChn_t::Frequency` (a Hz value) into its `period=`
    # field (confusing name preserved for regex parallelism with FT2/PT2/
    # ST3). Our `[OUR]` dump exposes `freq=` directly, which already folds
    # period_shift + vib_shift via Note::frequency. So for IT we compare
    # OUR freq vs IT2 "period" (= Hz) with a small tolerance: IT2's value
    # is integer-truncated and our compute path is float, so per-tick
    # rounding mismatches of a couple Hz are expected; anything wider is
    # a real divergence.
    compare_on_freq = (ref_ext == ".it")
    freq_tol = 2  # ±2 Hz for IT integer-truncated reference

    compared = 0
    intersect = 0
    first_div: Optional[Divergence] = None

    # Skip ticks before the first OUR period > 0 — that's pre-trigger
    # idle state where pt2-clone and our engine pick different
    # uninitialized defaults (us: vraw=64 from Volume::new(), pt2: 0).
    # Diffs there are noise, not engine bugs.
    triggered = False

    for our_key, our_row in iter_our_trace(our_path):
        q = ref_queues.get(our_key)
        if not q:
            continue
        ref_row = q.pop(0)
        # For IT, both sides report Hz (ref_row.period is actually
        # slaveChn_t::Frequency, our_row.freq is Note::frequency). The
        # "triggered" gate stays the same: we only need either side to
        # have a non-zero Hz/period to consider the voice live.
        our_trigger_val = our_row.freq if compare_on_freq else our_row.period
        if our_trigger_val > 0 or ref_row.period > 0:
            triggered = True
        if not triggered:
            continue
        intersect += 1
        # period: +-1 tolerance, optionally scaled / shift-stripped.
        # IT exception: compare Hz with ±freq_tol tolerance.
        compared += 1
        if compare_on_freq:
            our_val = our_row.freq
            ref_val = ref_row.period  # = IT2 slaveChn_t::Frequency (Hz)
            if abs(our_val - ref_val) > freq_tol:
                if first_div is None:
                    first_div = Divergence(
                        ord_=our_key[0], row=our_key[1], tick=our_key[2],
                        field="freq", our=our_val, ref=ref_val,
                    )
        else:
            our_period_raw = our_row.period
            if strip_shifts:
                our_period_raw -= our_row.period_shift + our_row.vib_shift
            our_period = our_period_raw // our_period_div if our_period_div != 1 else our_period_raw
            if abs(our_period - ref_row.period) > 1:
                if first_div is None:
                    first_div = Divergence(
                        ord_=our_key[0], row=our_key[1], tick=our_key[2],
                        field="period", our=our_period, ref=ref_row.period,
                    )
        # vraw: exact match.
        compared += 1
        if our_row.vraw != ref_row.vraw:
            if first_div is None:
                first_div = Divergence(
                    ord_=our_key[0], row=our_key[1], tick=our_key[2],
                    field="vraw", our=our_row.vraw, ref=ref_row.vraw,
                )
        # Stop early on first divergence — caller only wants the
        # first hit per channel.
        if first_div is not None:
            break

    return compared, intersect, first_div


def render_our(fixture: Path, ch: int, end_time: float, trace_path: Path) -> None:
    env = dict(os.environ)
    env["OUR_DUMP_CH"] = str(ch)
    with tempfile.NamedTemporaryFile(suffix=".wav", delete=True) as wav:
        cmd = [str(RENDER_WAV), str(fixture), wav.name,
               "--end-time", str(end_time)]
        with open(trace_path, "w") as tf:
            r = subprocess.run(cmd, env=env, stdout=subprocess.DEVNULL,
                               stderr=tf)
        if r.returncode != 0:
            raise RuntimeError(f"render_wav failed: {r.returncode}")


def render_ref(fixture: Path, ch: int, trace_path: Path) -> None:
    ext = fixture.suffix.lower()
    bin_path = CANONICAL[ext]
    env_var = DUMP_ENV[ext]
    env = dict(os.environ)
    env[env_var] = str(ch)
    # Some canonicals (pt2-clone, st3play) don't accept an --end-time
    # flag and will keep rendering until natural song end — which for
    # looping test fixtures is never. Cap the subprocess at 15 s; the
    # dump fflushes after each tick so the partial trace is still
    # usable. Non-zero return / timeout doesn't raise here unless the
    # trace file ends up empty.
    #
    # Use a non-pre-existing wav path: pt2-clone's mod2WavRender does
    # stat() on the output and triggers an interactive "OVERWRITE
    # FILE?" askBox prompt if it exists, which in headless mode hangs
    # silently (no trace ever emitted). tempfile.mktemp returns a
    # path that does NOT exist yet, which is what we want.
    wav_path = tempfile.mktemp(suffix=".wav", prefix="effect_test_")
    try:
        # pt2-clone changes cwd internally so it needs an absolute
        # fixture path. ft2play handles relative paths fine, but
        # always using absolute keeps the code simple.
        cmd = [str(bin_path), str(fixture.resolve()), "--render", wav_path]
        with open(trace_path, "w") as tf:
            try:
                r = subprocess.run(cmd, env=env, stdout=subprocess.DEVNULL,
                                   stderr=tf, timeout=15)
                rc = r.returncode
            except subprocess.TimeoutExpired:
                rc = -1  # treated as timeout (partial trace ok)
        # Trace empty == real error; non-empty partial trace is fine.
        if trace_path.stat().st_size == 0 and rc != 0:
            raise RuntimeError(f"canonical render failed: rc={rc}")
    finally:
        try:
            os.unlink(wav_path)
        except OSError:
            pass


def run_fixture(fixture: Path, end_time: float, verbose: bool) -> None:
    ext = fixture.suffix.lower()
    if ext not in CANONICAL:
        print(f"{fixture.name:<28} SKIP unknown extension")
        return
    if not HAS_TRACE[ext]:
        print(f"{fixture.name:<28} SKIP no canonical trace for {ext}")
        return
    if not RENDER_WAV.exists():
        print(f"FATAL: {RENDER_WAV} missing; run cargo build --release",
              file=sys.stderr)
        sys.exit(1)
    bin_path = CANONICAL[ext]
    if not bin_path.exists():
        print(f"{fixture.name:<28} SKIP canonical {bin_path} missing")
        return

    try:
        n_ch = detect_channels(fixture)
    except Exception as e:
        print(f"{fixture.name:<28} ERROR channel-detect: {e}")
        return
    if n_ch <= 0:
        print(f"{fixture.name:<28} ERROR no channels detected")
        return

    results = []
    with tempfile.TemporaryDirectory(prefix="effect_test_") as td:
        td = Path(td)
        for ch in range(n_ch):
            our_trace = td / f"our_{ch}.trace"
            ref_trace = td / f"ref_{ch}.trace"
            try:
                render_our(fixture, ch, end_time, our_trace)
                render_ref(fixture, ch, ref_trace)
            except Exception as e:
                results.append((ch, "ERROR", str(e)))
                continue
            compared, intersect, div = diff_traces(our_trace, ref_trace, ext)
            if intersect == 0:
                results.append((ch, "EMPTY", "no overlapping ticks"))
            elif div is None:
                results.append((ch, "PASS", f"{intersect} ticks"))
            else:
                # Per-canonical label: "ft2:", "pt2:", "st3:", "it2:".
                ref_label = {
                    ".xm": "ft2", ".mod": "pt2",
                    ".s3m": "st3", ".it": "it2",
                }.get(ext, "ref")
                results.append((
                    ch, "FAIL",
                    f"ord={div.ord_} row={div.row} tick={div.tick} "
                    f"{div.field}={div.our} ({ref_label}:{div.ref})",
                ))

    # One-line per fixture summary, with per-channel detail underneath
    # only when something interesting happened.
    pass_n = sum(1 for _, s, _ in results if s == "PASS")
    fail_n = sum(1 for _, s, _ in results if s == "FAIL")
    empty_n = sum(1 for _, s, _ in results if s == "EMPTY")
    err_n = sum(1 for _, s, _ in results if s == "ERROR")
    header = (f"{fixture.name:<28} {pass_n}P/{fail_n}F"
              f"{f'/{empty_n}E' if empty_n else ''}"
              f"{f'/{err_n}X' if err_n else ''} (of {n_ch})")
    print(header)
    if fail_n or err_n or verbose:
        for ch, status, detail in results:
            print(f"    ch{ch:<2} {status:<5} {detail}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--fixtures", default=str(DEFAULT_FIXTURES),
                    help="directory containing test modules")
    ap.add_argument("--end-time", type=float, default=30.0,
                    help="render duration in seconds (our side)")
    ap.add_argument("--filter", default=None,
                    help="substring filter on fixture filename")
    ap.add_argument("-v", "--verbose", action="store_true",
                    help="show per-channel detail even on PASS")
    args = ap.parse_args()

    fdir = Path(args.fixtures)
    if not fdir.is_dir():
        print(f"FATAL: fixtures dir not found: {fdir}", file=sys.stderr)
        return 1

    fixtures = sorted([
        p for p in fdir.iterdir()
        if p.suffix.lower() in CANONICAL and p.is_file()
    ])
    if args.filter:
        fixtures = [p for p in fixtures if args.filter in p.name]
    if not fixtures:
        print(f"No fixtures matched in {fdir}")
        return 1

    for fx in fixtures:
        run_fixture(fx, args.end_time, args.verbose)

    return 0


if __name__ == "__main__":
    sys.exit(main())
