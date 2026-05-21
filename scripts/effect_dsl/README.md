# effect_dsl — combination-effect XM fixture generator

## Rationale

OMT's effect corpus exercises one effect at a time. The bugs that hide in
modplayer (and trackers in general) live at the **combinations**:
porta-to-note **and** a fine vol-slide on the same row; retrigger with no
instrument byte **and** a vol-column slide; key-off **during** a sustain
envelope. Hand-authoring those fixtures in MilkyTracker is slow and the
combinatorial space is large, so we want a tight Python DSL that emits
valid XM 1.04 files readable by both `ft2play-cli` (our canonical FT2
reference per `CLAUDE.md`) and our own `render_wav`.

## DSL: Python literal, not YAML

Each fixture is a plain Python `dict` defined in a small `.py` file. Why
Python over YAML:

- **No extra dependency.** PyYAML isn't installed in the repo's tooling
  baseline; `scipy` is, but it's already used by `scripts/`.
- **Hex literals.** Effect params want `0x3, 0x10`, not `48` or strings.
- **Inline helpers.** Shared instrument bundles (`SINE_INST`) can be
  reused with dict-spread; harder in YAML.
- **Cell tuples > nested dicts.** `("C-4", 1, 0x40, 0x3, 0x10)` is
  shorter than five YAML keys per cell × 16 rows × 4 channels.

A fixture looks like:

```python
{
    "name": "porta_plus_volslide",
    "channels": 2,
    "tempo": 6, "bpm": 125,
    "instruments": [
        {"wave": "sine", "length": 256, "loop": "fwd", "volume": 64},
    ],
    "patterns": [[
        # row N: per channel cell = (note, inst, vol, eff, param) or None
        [("C-4", 1, 0,    0,   0),    ("E-4", 1, 0,    0,   0)],
        [("G-4", 1, 0,    0x3, 0x10), ("B-4", 1, 0,    0x3, 0x10)],   # porta
        [(0,    0, 0,    0xA, 0x04), (0,    0, 0,    0xA, 0x04)],    # + A04
        # ...
    ]],
}
```

`note_value()` accepts `"C-4"`, `"F#3"`, raw ints, and `"off"` (key-off).
Effect bytes are raw XM (see XM 1.04 spec at
<https://wiki.openmpt.org/Manual:_Module_formats#XM_files>).

## Coverage

The v0 starter set under `fixtures/` covers six combo classes:

| fixture | exercises |
|---|---|
| `porta_plus_volslide.xm` | 3xx (porta-to-note) + Axy (vol slide) same row |
| `vibrato_plus_volslide.xm` | 6xy combined vs split 4xy + Axy |
| `retrig_noinst_slide.xm` | note row with no instrument byte + vol-col fine slide (the bug class fixed today) |
| `keyoff_envelope_carry.xm` | both note 97 and K00 (eff 0x14) forms of key-off, no envelope |
| `multi_channel_phase.xm` | same note on 2 hard-panned channels, phase-stability check |
| `retrig_speed_change.xm` | R0x + E9x retrig with Fxx tempo changes mid-row |

All six load and render in both `ft2play-cli` and `render_wav`; see
`build_fixtures.py` for the spec source.

## v0 limitations (deliberately accepted)

- 1-4 channels (auto-padded to even number; FT2 requires even chan count).
- 1-4 instruments, **single sample each**, no multi-sample keymaps.
- **No envelopes** (volume / panning / vibrato). The `keyoff_envelope_carry`
  fixture therefore tests the *no-envelope* key-off path; once envelopes
  land in v1, it should be extended to test the with-envelope path too.
- **No ping-pong loops.** Forward loop only.
- Patterns: 1-2 per fixture, ≤256 rows each (uncompressed pattern
  storage — the loader accepts this and avoids a class of MSB-flag bugs
  in the generator).
- No instrument-relative-note keymap; `relative_note` is per-instrument
  and shared across all keys.

## Regenerating

```bash
python3 /Users/gil-ad/work/modplayer/scripts/effect_dsl/build_fixtures.py
```

To verify both renderers still accept them:

```bash
for f in scripts/effect_dsl/fixtures/*.xm; do
    /tmp/ft2play/ft2play-cli "$f" --render "/tmp/$(basename $f .xm).ft2.wav"
    target/release/render_wav "$f" "/tmp/$(basename $f .xm).ours.wav" --end-time 5
done
```

Both must complete without "Couldn't load song!" and produce non-empty
WAVs.

## Follow-ups (not in v0)

- Hook the fixtures into `scripts/corpus_regression.py` so the
  `B`/`C`-flag harness reports per-fixture us-vs-ft2 ratios.
- Add envelopes (XM 1.04 layout: 12 points × {frame, value} × 2 bytes
  each = 48 bytes per envelope, already reserved in the instrument
  header by the generator — just needs the type/point fields populated).
- Add multi-sample instruments with a keymap so we can author
  sample-swap-on-note fixtures.
- A small `record-expected.py` that captures the ft2play render's
  per-window RMS into a `.expected.json` for each fixture, turning the
  fixture set into a real regression suite.
