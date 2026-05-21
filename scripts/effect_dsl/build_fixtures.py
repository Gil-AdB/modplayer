"""Generate the v0 starter fixture set.

Each fixture is a small Python dict, deliberately tight (1-2 patterns,
16-32 rows) so a divergence between us and ft2play is obvious. Run::

    python3 scripts/effect_dsl/build_fixtures.py

Output goes to scripts/effect_dsl/fixtures/<name>.xm. The accompanying
README.md documents what each fixture exercises and what the canonical
ft2play render is expected to do.
"""

from pathlib import Path

from generator import write_xm


HERE = Path(__file__).resolve().parent
OUT  = HERE / "fixtures"


# A single shared instrument bank (looped sine + looped square) used by
# most fixtures so they all sound similar enough that audible differences
# are due to the effect under test, not waveform timbre.
SINE_INST   = {"name": "sine",   "wave": "sine",   "length": 256, "loop": "fwd", "volume": 64}
SQUARE_INST = {"name": "square", "wave": "square", "length": 256, "loop": "fwd", "volume": 64}
NOISE_INST  = {"name": "noise",  "wave": "noise",  "length": 4096, "loop": "fwd", "volume": 48}


def fix_porta_plus_volslide():
    """Row N: porta-to-note 3xx with vol-slide-down Axy on the same row.

    Two channels, both with the same pattern. We start a note on row 0,
    then on row 1 push a porta target with 3xx + a volslide-down A04.
    The bug class: many trackers compute the slide *before* applying the
    porta, or apply slide on tick 0 instead of skipping it. ft2play
    handles this in a specific, well-defined order — we want to match it.
    """
    return {
        "name": "porta_plus_volslide",
        "channels": 2,
        "tempo": 6,
        "instruments": [SINE_INST],
        "patterns": [[
            # ch0:           ch1:
            [("C-4", 1, 0, 0,   0),    ("E-4", 1, 0, 0,   0)],
            [("G-4", 1, 0, 0x3, 0x10), ("B-4", 1, 0, 0x3, 0x10)],
            [None,                       None],
            [None,                       None],
            # row 4: keep the porta and add Axy in the effect column
            [("G-4", 1, 0, 0x3, 0x10), ("B-4", 1, 0, 0x3, 0x10)],
            [(0, 0, 0, 0xA, 0x04),     (0, 0, 0, 0xA, 0x04)],
            [(0, 0, 0, 0xA, 0x04),     (0, 0, 0, 0xA, 0x04)],
            [(0, 0, 0, 0xA, 0x04),     (0, 0, 0, 0xA, 0x04)],
            # cool down
            [None,                       None],
            [None,                       None],
            [None,                       None],
            [None,                       None],
            [None,                       None],
            [None,                       None],
            [None,                       None],
            [None,                       None],
        ]],
    }


def fix_vibrato_plus_volslide():
    """Row N: vibrato 4xy + vol-slide-down on the same row (6xy combo).

    Effect 6xy is the XM "vibrato + volume slide" combined effect; we
    test both the combined form *and* the equivalent column form (4xy +
    Axy) on a second channel, so a divergence pins down which path is
    broken.
    """
    return {
        "name": "vibrato_plus_volslide",
        "channels": 2,
        "tempo": 6,
        "instruments": [SINE_INST],
        "patterns": [[
            [("A-4", 1, 0, 0,   0),    ("A-4", 1, 0, 0,   0)],
            # ch0: combined 6xy.  ch1: split — vibrato in vol col, slide in eff col.
            [(0, 0, 0, 0x6, 0x84),     (0, 0, 0xB4, 0xA, 0x04)],
            [(0, 0, 0, 0x6, 0x84),     (0, 0, 0xB4, 0xA, 0x04)],
            [(0, 0, 0, 0x6, 0x84),     (0, 0, 0xB4, 0xA, 0x04)],
            [(0, 0, 0, 0x6, 0x84),     (0, 0, 0xB4, 0xA, 0x04)],
            [(0, 0, 0, 0x6, 0x84),     (0, 0, 0xB4, 0xA, 0x04)],
            [(0, 0, 0, 0x6, 0x84),     (0, 0, 0xB4, 0xA, 0x04)],
            [(0, 0, 0, 0x6, 0x84),     (0, 0, 0xB4, 0xA, 0x04)],
            [None,                       None],
            [None,                       None],
            [None,                       None],
            [None,                       None],
            [None,                       None],
            [None,                       None],
            [None,                       None],
            [None,                       None],
        ]],
    }


def fix_retrig_noinst_slide():
    """Note-without-instrument retrigger + vol-col fine-slide.

    This is the exact bug class fixed recently: a row that has a note
    cell but no instrument byte should retrigger using the previously
    set instrument, NOT cause volume column to be ignored. Combine that
    with a vol-col fine-slide-down (0x9x family) on the same row.

    Layout:
      row 0: full trigger (note + inst + vol)
      row 4: same note, no inst byte, vol-col fine-slide-down 0x98
      row 8: same again, vol-col fine-slide-up   0x9C
    """
    return {
        "name": "retrig_noinst_slide",
        "channels": 1,
        "tempo": 6,
        "instruments": [SINE_INST],
        "patterns": [[
            [("C-4", 1, 0x40, 0, 0)],
            [None],
            [None],
            [None],
            [("C-4", 0, 0x98, 0, 0)],   # note, NO instrument, vol-col fine slide down (0x98 = 0x90 | 8)
            [None],
            [None],
            [None],
            [("C-4", 0, 0xC8, 0, 0)],   # vol-col fine slide up (0xC0 | 8)
            [None],
            [None],
            [None],
            [None],
            [None],
            [None],
            [None],
        ]],
    }


def fix_keyoff_envelope_carry():
    """Key-off mid-note when no envelope is present.

    With no volume envelope, FT2's documented behavior is: K00 (eff 0x14
    with param 0) sets volume to 0 immediately. We don't have an
    envelope in v0; this fixture tests the K00 + already-fading note
    interaction so we lock in the no-envelope path before v1 adds
    envelopes.
    """
    return {
        "name": "keyoff_envelope_carry",
        "channels": 1,
        "tempo": 6,
        "instruments": [SINE_INST],
        "patterns": [[
            [("C-4", 1, 0x40, 0, 0)],
            [None],
            [None],
            [None],
            [("off", 0, 0, 0, 0)],       # row 4: pure key-off via note 97
            [None],
            [None],
            [None],
            [("E-4", 1, 0x40, 0, 0)],    # row 8: new note
            [None],
            [(0, 0, 0, 0x14, 0)],        # row 10: K00 eff form of key-off
            [None],
            [None],
            [None],
            [None],
            [None],
        ]],
    }


def fix_multi_channel_phase():
    """Same note on two channels, no detune — checks phase stability.

    A common bug: a renderer accidentally reseeds the LFO / sample
    position based on global tick count rather than per-channel state,
    producing a flanging beat that should not exist. With both channels
    panned hard apart, mono-summing should yield ~6 dB more energy than
    one channel alone — if instead there's destructive interference we
    see a 3-6 dB dip vs ft2play.
    """
    sine_left  = {**SINE_INST, "name": "sineL", "panning": 0}
    sine_right = {**SINE_INST, "name": "sineR", "panning": 255}
    return {
        "name": "multi_channel_phase",
        "channels": 2,
        "tempo": 6,
        "instruments": [sine_left, sine_right],
        "patterns": [[
            [("A-4", 1, 0x40, 0, 0), ("A-4", 2, 0x40, 0, 0)],
            [None,                    None],
            [None,                    None],
            [None,                    None],
            [None,                    None],
            [None,                    None],
            [None,                    None],
            [None,                    None],
            [None,                    None],
            [None,                    None],
            [None,                    None],
            [None,                    None],
            [None,                    None],
            [None,                    None],
            [None,                    None],
            [None,                    None],
        ]],
    }


def fix_retrig_under_speed_change():
    """Retrig effect (R0x = 0x1B, E9x) interacting with Fxx speed change.

    The bug class: when speed changes mid-row, retrig has to recompute
    which ticks within the row trigger the new note. We start at speed
    6, force a retrig with R03 (every 3 ticks → 2 retrigs/row), then
    drop to speed 3 (1 retrig/row) and bump up to speed 9.
    """
    return {
        "name": "retrig_speed_change",
        "channels": 1,
        "tempo": 6,
        "instruments": [SQUARE_INST],
        "patterns": [[
            [("A-4", 1, 0x40, 0,    0)],
            [(0,    0, 0,    0x1B, 0x03)],   # R03 — retrig every 3 ticks
            [(0,    0, 0,    0x1B, 0x03)],
            [(0,    0, 0,    0xF,  0x03)],   # Fxx — speed → 3
            [(0,    0, 0,    0x1B, 0x02)],   # R02 — every 2 ticks (1×/row now)
            [(0,    0, 0,    0x1B, 0x02)],
            [(0,    0, 0,    0xF,  0x09)],   # speed → 9
            [(0,    0, 0,    0x1B, 0x04)],   # R04 — every 4 ticks (~2×/row)
            [(0,    0, 0,    0x1B, 0x04)],
            [(0,    0, 0,    0xE,  0x90)],   # E90 — retrig once (param low nibble = 0 → immediate)
            [(0,    0, 0,    0xE,  0x93)],   # E93 — retrig at tick 3 within the row
            [None],
            [None],
            [None],
            [None],
            [None],
        ]],
    }


FIXTURES = [
    fix_porta_plus_volslide,
    fix_vibrato_plus_volslide,
    fix_retrig_noinst_slide,
    fix_keyoff_envelope_carry,
    fix_multi_channel_phase,
    fix_retrig_under_speed_change,
]


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for fn in FIXTURES:
        spec = fn()
        path = OUT / f"{spec['name']}.xm"
        write_xm(spec, path)
        print(f"wrote {path}  ({path.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
