# modplayer — Claude instructions

## Default stance on audio-divergence reports

**Read this every session.** This is a project-wide instruction, not a
suggestion.

When the user reports an audio bug ("X sounds bad", "wrong notes",
"flanging", "weird timbre", "channel Y is off"), the bug is real until
proven otherwise. The user has trained ears, a long history with
tracker music, and can hear things that don't show up in
overall-RMS comparisons.

**Do not, under any circumstances, dismiss a reported divergence as
"interpolation difference", "cubic-vs-sinc artifact", "filter
cosmetics", or any other "this is just how it is" verdict without
all three of:**

1. A **canonical-tracker reference render** for that format
   (pt2-clone for MOD, ft2play/ft2-clone for XM, st3play for S3M,
   it2play for IT). If the binary isn't built yet, build it before
   opining.
2. A **localized comparison** — per-channel, per-effect, or
   per-instrument — showing exactly where and how we diverge from the
   canonical, not just from OMT.
3. A **specific named bug class** with a code path. "Interpolation
   differences" without a falsifiable test that distinguishes
   interpolation-only from a real bug is a non-answer.

If those three things don't all line up, the correct response is
**"I don't have enough evidence yet, here's what I'll measure next"**,
not a hand-wavy category label.

## Historical incidents

Two recent failure modes that map onto this rule:

- **Redalert.mod, 2026-05-17.** Two-fold pushback from the user
  ("filter is meaningless", "it's not related to the filter, for
  fuck's sake") before I finally asked "what do you hear?" and got
  pointed at the porta-up slide. The bug was a `period_shift` leak
  from arpeggio rows (`xmplayer/src/song/backend/mod_.rs`, commit
  da45510). RMS-vs-OMT matched within 5%; a per-octave spectrum check
  showed 4 kHz at 2.5x pt2. I'd spent the prior hour proposing
  variants of the wrong fix.

- **SHOOTING.XM, 2026-05-18.** Multiple wrong turns in one
  investigation, all the same anti-pattern:

  1. First wrong call: "OMT uses sinc, we use cubic — interpolation
     difference, just build ft2play to confirm cosmetics." User
     pushed back: we *already* support user-selectable interpolators
     (default is `FilterType::Sinc` per `song/mod.rs:377,705`); the
     audible divergence is way bigger than sinc-vs-cubic could
     explain.

  2. Second wrong call: per-channel us-vs-OMT showed ch0/1/2 at
     spectrogram-corr 0.765, those three all use **instrument 12**
     (the only one with auto-vibrato in the file). I went straight to
     "auto-vibrato implementation differs". User had to push back
     again ("master plays this right — use it for comparison if
     needed").

  3. Actual finding once I checked master: branch is **1.45× louder
     than master across every octave band**, while master matches
     ft2play to 0.84× (close, within Compatible-vs-Original mix-levels
     spread). The audible "flanging" is the consequence of clipping
     headroom violation, not a state-machine bug. The regression is
     somewhere between master and feat/s3m-refactor, most likely the
     XM MixLevels commits (`207274e` / `2de767e`).

  Diagnostic numbers (30 s of SHOOTING.XM):

  | renderer | RMS | peak | gain vs ft2 |
  |----------|------|------|-------------|
  | branch   | 0.202 | 1.32 | 1.27× |
  | master   | 0.136 | 0.99 | 0.84× |
  | OMT      | 0.203 | 1.49 | 1.27× |
  | ft2play  | 0.162 | 1.02 | 1.00× |

  Branch-vs-master per-octave ratios: 1.31–1.58 (flat, not
  frequency-dependent → straight gain, not a spectral bug).

The pattern across all of these: I reach for a "stylistic" explanation
(filter, interpolation, mix levels, vibrato semantics) because it's
the *cheapest* hypothesis to articulate, and only switch to looking
for a real bug when the user pushes back. Reverse the default —
assume real bug, articulate cheap hypothesis only as a foil to
falsify, not as a conclusion.

**Specifically: before opining on category for an XM audio bug, do
the two-minute master-vs-branch sanity check first.** Build
`render_wav` in a `git worktree add /tmp/modplayer_master master`,
render the same file from both, compute RMS + per-octave ratios. If
master and branch differ by >10% on the full mix, the bug is a
regression in our recent work — start with `git log master..` for
suspects, not with FT2 vs OMT theorizing.

## Canonical references in this repo

| Format | Canonical binary | CLI built via |
|--------|-------------------|---------------|
| MOD    | pt2-clone         | `tools/build_pt2_clone_cli.sh` → `/tmp/pt2-clone/pt2-clone-cli` |
| S3M    | st3play           | `tools/build_st3play_cli.sh` → `/tmp/st3play/st3play-cli` |
| IT     | it2play           | `tools/build_it2play_cli.sh` → `/tmp/it2play/it2play-cli` |
| XM     | ft2play           | not yet built — build it before opining on any XM audio bug |

`scripts/corpus_regression.py` consumes these automatically when the
binaries exist. A `C` flag in its output means "we and OMT agree but
the canonical disagrees" — that's the highest-priority class to
chase, because it's where both modern players inherit the same bug.

## Existing instrumentation — use these before building new ones

The user is consistently right that we already have diagnostic tools.
**Search before building.**

In-process / our side:
- `target/release/state_dump <song> [--order N] [--rows S..E]
  [--channels a,b] [--all-ticks] [--output FILE]` — per-tick channel
  state dump. Reports voice on/off, instrument, sample, sample_pos, dU,
  output/voice volume, panning, envelope positions, effect+param,
  channel_volume, relative_note, finetune, last_render_tick,
  cut_reason. **This already exists; do not re-invent.**
- `target/release/render_wav <song> <out.wav> [--end-time SEC]
  [--mute-channels a,b,c]` — headless render, supports per-channel
  mute for solo isolation.
- `target/release/openmpt_solo <song> <out.wav> [--solo CH]
  [--mute LIST] [--end-time SEC]` — libopenmpt-backed render with
  per-channel solo/mute.
- `OUR_DUMP_CH=<idx>` env var on `render_wav` prints `[OUR]` per-tick
  state for that channel (see `channel_state/mod.rs:864-872`).

External-reference side:
- `tools/openmpt_instrumentation.patch` adds `[OMT]` per-tick channel
  trace to libopenmpt for diff-against-`[OUR]`. Build the
  instrumented OMT via `tools/build_openmpt_inst.sh`.
- `tools/build_pt2_clone_cli.sh` / `build_st3play_cli.sh` /
  `build_it2play_cli.sh` / `build_ft2play_cli.sh` — canonical-tracker
  CLI binaries with `--render <out>`.
- **No ft2play / ft2-clone per-tick instrumentation exists yet.**
  ft2play has `--no-intrp` (linear interp, FT2-native) and
  `--no-vramp` (no volume ramp). ft2-clone has no CLI render at all
  (it's a full GUI tracker; `wavRenderer` is GUI-only).

Existing diagnostic Python:
- `scripts/compare_renders.py` — window-by-window FFT/RMS/cc.
- `scripts/cents_distribution.py` — pitch-divergence histogram.
- `scripts/peek_window.py` — top FFT peaks of one 500ms window.
- `scripts/bisect_channel.py` — per-channel solo + comparison at a
  target window. Already flags `LOUDNESS BUG`, `cc drift`,
  `some drift`. **Use this before writing new comparison code.**
- `scripts/diff_bisect.py` — fair per-channel via subtraction
  (full minus mask-N), independent of per-engine auto-preamp.
- `scripts/sc2_pt2_compare.py` — three-way us/OMT/pt2 for MOD.
- `scripts/corpus_regression.py` — full-corpus harness with `B`
  (band-deviation) and `C` (canonical disagreement) flags.

The flow for "X sounds bad on format Y" is documented in
`.claude/skills/detect-render-bugs/SKILL.md`. Read that skill before
starting any new investigation — it lists the six steps and the
diagnostic table for interpreting per-channel cc/RMS patterns.

## When in doubt

Ask **what specifically** the user is hearing ("which channel",
"which moment in the song", "what's the symptom — pitch / timing /
timbre / missing notes"). One sentence from the user beats an hour
of unguided spectral analysis. See `feedback_trust_user_reports.md`
in user memory.
