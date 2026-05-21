"""XM fixture generator from a small declarative Python DSL.

Goal
----
Author tight combo-effect tests (porta+volslide, retrig-no-instrument+slide,
keyoff during sustain loop, NNA chains under fast retrigger, etc.) without
opening MilkyTracker. The DSL is plain Python dicts/strings so we can mix
hex literals, comments, and helper functions naturally.

Output
------
A valid XM 1.04 file readable by both ft2play-cli (the canonical FT2 replayer
per CLAUDE.md) and modplayer's render_wav. Patterns are stored uncompressed
(every channel emits the 5-byte unpacked form, no MSB-flag packing); sample
data is 16-bit signed delta-encoded PCM as the XM spec requires.

DSL shape
---------
A fixture is a Python dict::

    {
        "name": "porta_plus_volslide",
        "channels": 2,                # 1..4
        "tempo": 6,                   # ticks/row (default 6)
        "bpm": 125,                   # default 125
        "instruments": [
            # each entry is a dict; only "wave" is required.
            {"wave": "sine",          # "sine" | "square" | "saw" | "noise"
             "length": 2048,          # samples
             "loop": "fwd",           # "none" | "fwd"
             "volume": 64,            # 0..64 (default 64)
             "finetune": 0,           # -128..127 (default 0)
             "relative_note": 0,      # default 0
             "panning": 128},         # 0..255 (default 128)
        ],
        "patterns": [
            # each entry is a list of rows; each row is a list[channel] of
            # cell tuples (note, inst, vol, eff, param). Use None to mean
            # "empty cell" — equivalent to (0, 0, 0, 0, 0).
            [
                [("C-4", 1, 0, 0, 0), None],                 # row 0
                [None,            ("E-4", 1, 0x40, 0x3, 0x08)],
                ...
            ],
        ],
        "order": [0],                 # default [0..len(patterns)]
    }

Notes:
- `note` is either a string "C-4".."B-8", "off" (key-off, 97), or an int (XM
  note number; 1=C-0, 97=key-off).
- `vol` 0 means "no volume column"; XM volume column is 0x00 or 0x10..0xFF
  with sub-ranges encoding fine-slides, panning, etc. We pass it through
  verbatim — pick the right byte yourself (see XM spec).
- `eff` and `param` are raw XM effect bytes (0..0x22 or 'a'..'z' / arpeggio,
  see XM spec). E-effects use eff=0xE and the high nibble of param selects
  the sub-effect (e.g. E90 retrig: eff=0xE, param=0x90).

We intentionally do *not* support envelopes, multi-sample instruments, or
ping-pong loops in v0 — those go in v1 after the combo-effect cases reveal
the highest-leverage holes.
"""

from __future__ import annotations

import math
import struct
from pathlib import Path
from typing import Iterable


# -----------------------------------------------------------------------------
# Helpers
# -----------------------------------------------------------------------------

NOTE_NAMES = ["C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-"]
NOTE_TO_NUM = {name + str(octv): octv * 12 + i + 1
               for octv in range(0, 8)
               for i, name in enumerate(NOTE_NAMES)}


def note_value(n):
    """Translate a DSL note token to the XM 1-byte note number.

    XM pattern note encoding:
      0     = no note
      1..96 = C-0..B-7
      97    = key off
    """
    if n is None or n == 0:
        return 0
    if isinstance(n, int):
        return n
    if isinstance(n, str):
        if n.lower() in ("off", "key_off", "keyoff"):
            return 97
        if n in NOTE_TO_NUM:
            return NOTE_TO_NUM[n]
        # accept "C#4" too (no '-')
        if len(n) == 3 and n[2].isdigit():
            alt = n[:2] + "-" + n[2] if n[1] != "#" else n
            if alt in NOTE_TO_NUM:
                return NOTE_TO_NUM[alt]
    raise ValueError(f"bad note: {n!r}")


def gen_wave(kind: str, length: int) -> list[int]:
    """Generate a 16-bit signed PCM waveform, peak ~30000.

    The waveforms are intentionally clean and identical run-to-run so
    fixtures stay reproducible.
    """
    if kind == "sine":
        return [int(30000 * math.sin(2 * math.pi * 8 * i / length)) for i in range(length)]
    if kind == "square":
        return [30000 if (i * 16 // length) % 2 == 0 else -30000 for i in range(length)]
    if kind == "saw":
        return [int(30000 * (2 * ((i * 8 % length) / length) - 1)) for i in range(length)]
    if kind == "noise":
        # deterministic LCG so renders are reproducible
        x = 0xACE1
        out = []
        for _ in range(length):
            x = (x * 1103515245 + 12345) & 0x7FFFFFFF
            out.append((x % 60001) - 30000)
        return out
    raise ValueError(f"unknown waveform kind {kind!r}")


def delta_encode_i16(samples: Iterable[int]) -> bytes:
    """XM spec stores samples as deltas; loader integrates (Wrapping sums)."""
    prev = 0
    out = bytearray()
    for s in samples:
        d = (s - prev) & 0xFFFF
        out += struct.pack("<h", d if d < 0x8000 else d - 0x10000)
        prev = s
    return bytes(out)


# -----------------------------------------------------------------------------
# Binary writers
# -----------------------------------------------------------------------------

def _pad(s: str, n: int) -> bytes:
    """ASCII-pad/truncate to exactly n bytes (space-padded, like XM headers)."""
    b = s.encode("ascii", errors="replace")[:n]
    return b + b" " * (n - len(b))


def _build_pattern_data(rows: list, num_channels: int) -> bytes:
    """Encode one pattern. We use the *uncompressed* form for every cell —
    each cell is exactly 5 bytes (note, inst, vol, eff, param). The XM
    loader accepts this and it removes a class of generator bugs.
    """
    out = bytearray()
    for row in rows:
        if len(row) < num_channels:
            row = list(row) + [None] * (num_channels - len(row))
        for ch in range(num_channels):
            cell = row[ch]
            if cell is None:
                note = inst = vol = eff = param = 0
            else:
                n, inst, vol, eff, param = cell
                note = note_value(n)
            # Uncompressed: no MSB. All 5 bytes present.
            out += bytes([note & 0x7F, inst & 0xFF, vol & 0xFF, eff & 0xFF, param & 0xFF])
    return bytes(out)


def _build_instrument(inst_spec: dict) -> bytes:
    """One full instrument record (extended-header + 1 sample header + data).

    We always emit one sample per instrument in v0. Header size is fixed at
    263 bytes (the XM standard "extended" instrument header), which OMT,
    ft2play, and us all accept.
    """
    name      = inst_spec.get("name", "")
    wave_kind = inst_spec.get("wave", "sine")
    length    = int(inst_spec.get("length", 2048))
    loop_kind = inst_spec.get("loop", "none")
    volume    = int(inst_spec.get("volume", 64))
    finetune  = int(inst_spec.get("finetune", 0))
    relnote   = int(inst_spec.get("relative_note", 0))
    panning   = int(inst_spec.get("panning", 128))

    # --- instrument header (263 bytes) -----------------------------------
    hdr = bytearray()
    hdr += struct.pack("<I", 263)             # header size
    hdr += _pad(name, 22)                     # name
    hdr += bytes([0])                         # type
    hdr += struct.pack("<H", 1)               # number of samples
    hdr += struct.pack("<I", 40)              # sample header size
    hdr += bytes([0] * 96)                    # keymap (all to sample 0)
    hdr += bytes([0] * 48)                    # volume envelope points (12 × 4)
    hdr += bytes([0] * 48)                    # panning envelope points
    hdr += bytes([0])                         # num vol points
    hdr += bytes([0])                         # num pan points
    hdr += bytes([0])                         # vol sustain pt
    hdr += bytes([0])                         # vol loop start
    hdr += bytes([0])                         # vol loop end
    hdr += bytes([0])                         # pan sustain pt
    hdr += bytes([0])                         # pan loop start
    hdr += bytes([0])                         # pan loop end
    hdr += bytes([0])                         # vol type (no envelope)
    hdr += bytes([0])                         # pan type
    hdr += bytes([0])                         # vibrato type
    hdr += bytes([0])                         # vibrato sweep
    hdr += bytes([0])                         # vibrato depth
    hdr += bytes([0])                         # vibrato rate
    hdr += struct.pack("<H", 0)               # vol fadeout
    hdr += struct.pack("<H", 0)               # reserved
    # We allocated 4+22+1+2+4+96+48+48+8+8+4 = 245 bytes; pad to 263.
    hdr += bytes([0] * (263 - len(hdr)))
    assert len(hdr) == 263

    # --- sample header (40 bytes) ----------------------------------------
    pcm = gen_wave(wave_kind, length)
    sample_bytes_len = length * 2
    sample_flags = 0x10                       # 16-bit
    if loop_kind == "fwd":
        sample_flags |= 0x01                  # forward loop
        loop_start_bytes = 0
        loop_len_bytes = sample_bytes_len
    elif loop_kind == "none":
        loop_start_bytes = 0
        loop_len_bytes = 0
    else:
        raise ValueError(f"unsupported loop kind: {loop_kind!r}")

    shdr = bytearray()
    shdr += struct.pack("<I", sample_bytes_len)
    shdr += struct.pack("<I", loop_start_bytes)
    shdr += struct.pack("<I", loop_len_bytes)
    shdr += bytes([volume & 0xFF])
    shdr += struct.pack("<b", max(-128, min(127, finetune)))
    shdr += bytes([sample_flags & 0xFF])
    shdr += bytes([panning & 0xFF])
    shdr += struct.pack("<b", max(-128, min(127, relnote)))
    shdr += bytes([0])                        # reserved
    shdr += _pad(name, 22)                    # sample name (echo inst name)
    assert len(shdr) == 40

    # --- sample data (delta-encoded i16) ---------------------------------
    data = delta_encode_i16(pcm)
    assert len(data) == sample_bytes_len

    return bytes(hdr) + bytes(shdr) + data


def build_xm(spec: dict) -> bytes:
    """Build a complete XM 1.04 file from a DSL spec."""
    name        = spec.get("name", "fixture")
    channels    = int(spec.get("channels", 2))
    tempo       = int(spec.get("tempo", 6))
    bpm         = int(spec.get("bpm", 125))
    instruments = spec["instruments"]
    patterns    = spec["patterns"]
    order       = spec.get("order", list(range(len(patterns))))

    # FT2 (and ft2play) require even channel count, minimum 2. Auto-pad
    # odd / 1 to the next even number so caller can author "1-channel"
    # fixtures naturally; the extra channel is silent.
    if not (1 <= channels <= 4):
        raise ValueError("channels must be in 1..4 for v0")
    if channels < 2:
        channels = 2
    elif channels & 1:
        channels += 1
    spec_channels = int(spec.get("channels", 2))
    if not (1 <= len(instruments) <= 4):
        raise ValueError("instruments must be 1..4 for v0")
    if not (1 <= len(patterns) <= 2):
        raise ValueError("patterns must be 1..2 for v0")

    # --- file header ----------------------------------------------------
    out = bytearray()
    out += b"Extended Module: "              # 17 bytes
    out += _pad(name, 20)                    # 20 bytes title
    out += bytes([0x1A])                     # signature
    out += _pad("ModPlayer DSL v0", 20)      # tracker name (20 bytes)
    out += struct.pack("<H", 0x0104)         # version 1.04
    out += struct.pack("<I", 276)            # header size (276 = 20 + 256)

    song_length = len(order)
    if song_length > 256:
        raise ValueError("order list too long")
    out += struct.pack("<H", song_length)
    out += struct.pack("<H", 0)              # restart position
    out += struct.pack("<H", channels)
    out += struct.pack("<H", len(patterns))
    out += struct.pack("<H", len(instruments))
    out += struct.pack("<H", 0)              # flags: 0 = Amiga period
    out += struct.pack("<H", tempo)
    out += struct.pack("<H", bpm)
    pattern_order = bytearray(order)
    pattern_order += bytes([0] * (256 - len(pattern_order)))
    out += pattern_order

    # --- patterns -------------------------------------------------------
    for rows in patterns:
        nrows = len(rows)
        if not (1 <= nrows <= 256):
            raise ValueError(f"rows per pattern must be 1..256, got {nrows}")
        pdata = _build_pattern_data(rows, channels)
        out += struct.pack("<I", 9)           # pattern header size
        out += bytes([0])                     # packing type
        out += struct.pack("<H", nrows)
        out += struct.pack("<H", len(pdata))
        out += pdata

    # --- instruments + samples ------------------------------------------
    for inst in instruments:
        out += _build_instrument(inst)

    return bytes(out)


# -----------------------------------------------------------------------------
# Tiny convenience wrapper
# -----------------------------------------------------------------------------

def write_xm(spec: dict, path: str | Path) -> Path:
    p = Path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_bytes(build_xm(spec))
    return p
