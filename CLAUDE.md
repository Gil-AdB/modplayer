# modplayer — Claude instructions

## Project policy: native-player parity, not OMT parity

**The canonical reference for each format is the native tracker's own
replayer**, not libopenmpt. When us, OMT, and the native disagree, we
always align with the native:

| Format | Native canonical | Why |
|--------|------------------|-----|
| MOD    | **pt2-clone**    | ProTracker 2 is the format spec; OMT has documented MOD-effect divergences |
| XM     | **ft2play** / ft2-clone | FastTracker 2 is the format spec; OMT calibrates differently |
| S3M    | **st3play**      | Scream Tracker 3 is the format spec |
| IT     | **it2play**      | Impulse Tracker 2 is the format spec; OMT applies its own MixLevels matrix |

OMT remains a *secondary* reference for cases where the native tracker
itself had a bug fixed by Schism / MPT / OMT — but it is not the gain
or behaviour target. Per-format gain calibration in
`xmplayer/src/song/backend.rs::*_MIX` should track the native tracker,
not OMT.

Examples of recalibration following this policy:
- `348d58f` XM_MIX.global_scale 0.7468 → 0.6 (was OMT-tuned, now
  ft2play-tuned; SHOOTING.XM 1.45× too loud → 1.00× of ft2play).
- Pending: MOD_MIX.global_scale, S3M_MIX.global_scale, IT_MIX.global_scale
  should be re-checked against pt2-clone / st3play / it2play
  respectively.

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

### Numerical match ≠ audible match — do not declare victory on metrics

This is a separate recurring failure mode. After finding and fixing
one or two real bugs in an audible-bug investigation, the temptation
is to keep tuning a knob (gain calibration, scale constants, ratio
thresholds) until a *metric* lines up with the reference — then
declare the bug fixed. **Don't.** The user can hear things that don't
show up in RMS, peak, per-octave-band RMS, full-mix cross-correlation,
or even spectrogram magnitude. Specifically: phase, timing, per-event
envelopes, click/pop transients, per-effect mis-handling, per-channel
phase drift — all can be present at 1.003× RMS of the reference and
still sound clearly wrong.

Real example (SHOOTING.XM, 2026-05-19): two real bugs landed
(auto-vibrato sweep, frequency_shift reset). I then "fixed" the gain
by recalibrating `XM_MIX.global_scale` so SHOOTING.XM's RMS matched
ft2play's RMS to 0.3%, declared the bug fixed, and got the response
"the module sounds wrong, no amount of average gain is going to fix
it, you are gaming the system." Lesson: **audible-bug investigations
end when the user says it sounds right, not when the numbers line up.**

Before claiming a fix on an audible bug:
- Ask **what specifically** they hear — which note, which moment,
  which channel, what symptom (pitch / timing / timbre / click /
  missing). One sentence from the user beats another hour of metric
  tuning.
- If you've fixed two bugs and the user is still hearing something,
  there's a third bug. Don't reach for a knob; ask.
- Gain/calibration constants are the easiest knobs to turn. They're
  also the most attractive to me as cheap "fixes" — which is why
  they're suspect on an *audible* (not numerical) report.

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

  1. **Wrong call 1**: "OMT uses sinc, we use cubic — interpolation
     difference, just build ft2play to confirm cosmetics." User
     pushed back: we *already* support user-selectable interpolators
     (default is `FilterType::Sinc` per `song/mod.rs:377,705`); the
     audible divergence is way bigger than sinc-vs-cubic could
     explain.

  2. **Wrong call 2**: per-channel us-vs-OMT showed ch0/1/2 at
     spectrogram-corr 0.765, those three all use **instrument 12**
     (the only one with auto-vibrato in the file). I went straight to
     "auto-vibrato implementation differs". User had to push back
     ("master plays this right — use it for comparison").

  3. **Wrong call 3**: After confirming branch is **1.45× louder
     than master across every octave band**, I claimed the audible
     "flanging" was clipping headroom violation from the gain
     regression. User pushed back again: **uniform gain scaling
     cannot produce flanging.** A flat gain × everything ≠ beat
     patterns. The per-channel waveform divergence I'd already found
     (us-ch0 vs OMT-ch0 spectrogram-corr 0.765, residual concentrated
     at the 2nd harmonic of the channel fundamental — **same notes,
     different waveform shape over time**) is the actual audible
     issue, not the gain.

  So the SHOOTING.XM picture is TWO findings, not one:

  | finding | scope | evidence |
  |---------|-------|----------|
  | Branch is **1.45× louder than master** across all bands | full-mix amplitude | per-octave 1.31–1.58, flat |
  | Per-channel waveform diverges from OMT on ch0/1/2/14/15 | individual channels | spectrogram-corr 0.765 on ch0; same envelope, different shape |

  Neither alone explains the audible flanging. The gain regression
  causes clipping at transients; the per-channel waveform divergence
  causes beating/timbre differences. Both need fixing; only one of
  them is the user's reported "flanging".

  Numbers (30 s of SHOOTING.XM):

  | renderer | RMS | peak | gain vs ft2 |
  |----------|------|------|-------------|
  | branch   | 0.202 | 1.32 | 1.27× |
  | master   | 0.136 | 0.99 | 0.84× |
  | OMT      | 0.203 | 1.49 | 1.27× |
  | ft2play  | 0.162 | 1.02 | 1.00× |

The pattern across all four wrong calls: I reach for a single-cause
explanation (filter, interpolation, vibrato, gain) because it's the
*cheapest* hypothesis to articulate, and dismiss prior evidence
when the new hypothesis "explains everything". **Multiple findings
can coexist.** A gain regression and a per-channel state-machine
divergence are independent — both can be present, and the user's
audible complaint can be sensitive to one but not the other.

**Specifically: when investigating an XM audio bug, run BOTH:**

1. **Master-vs-branch sanity check** (cheap, catches gain regressions
   and other our-fault changes since master). Build `render_wav` in
   a `git worktree add /tmp/modplayer_master master`, render the
   same file from both, compute per-octave ratios.
2. **Per-channel diff against the canonical (ft2play)** (catches
   state-machine / effect-handling divergences invisible in full-mix
   numbers). Use the `--render` patches in `tools/` plus the
   instrumentation patch (see "Existing instrumentation" below) for
   per-tick channel state.

Do not collapse the two into one. They measure different things.

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
- `tools/ft2play_instrumentation.patch` — applied by
  `build_ft2play_cli.sh`. Adds an `FT2_DUMP_CH`-gated `[FT2]`
  per-tick stderr trace at the end of `pmp_main.c::mainPlayer`.
  Fields: ord, row, tick, ch, inst, samp, finalPeriod, finalVol,
  realVol, outVol, outPan, finalPan, envVPos, envVAmp, envPPos,
  envPAmp, eVibPos, eVibAmp, eVibSweep, effTyp, eff, relTonNr,
  fineTune, fadeOutAmp, status. The Order/Row/Tick prefix matches
  our `[OUR]` state_dump exactly so the traces align line-for-line
  on common ticks.

**ft2-clone has no CLI render at all** (it's a full GUI tracker;
`wavRenderer` is GUI-only). For XM bug investigation, use
ft2play-cli + FT2_DUMP_CH instead.

### XM bug investigation workflow

When the user reports an XM audio defect on channel N:

```bash
# 1. Build everything (idempotent; skips work if already done).
cargo build --release --bin render_wav --bin state_dump
tools/build_ft2play_cli.sh    # applies both --render + instrumentation patches

# 2. Render the same window from both engines.
RA="/path/to/song.xm"
target/release/render_wav "$RA" /tmp/ours.wav --end-time 60
/tmp/ft2play/ft2play-cli "$RA" --render /tmp/ft2.wav

# 3. Capture per-tick state for channel N from both.
OUR_DUMP_CH=N target/release/render_wav "$RA" /tmp/null.wav --end-time 60 2>/tmp/our.trace
FT2_DUMP_CH=N /tmp/ft2play/ft2play-cli "$RA" --render /tmp/null2.wav 2>/tmp/ft2.trace

# 4. Diff at a specific (order, row) where audio diverges.
grep "ord=3 row=[0-3] " /tmp/our.trace
grep "ord=3 row=[0-3] " /tmp/ft2.trace

# 5. Per-channel solo bisect to confirm which channel is the bug source.
python3 scripts/bisect_channel.py "$RA" 10.0 --win 1.0 --channels 18 --end-time 30
```

Field mapping (`[OUR]` ↔ `[FT2]`):
- our `period` ↔ FT2 `finalPeriod`
- our `freq` derives from period via the format-table
- our `vraw` ↔ FT2 `realVol` / `outVol` (channel slot vol)
- **Auto-vibrato state**: FT2 has `eVibPos`, `eVibAmp`, `eVibSweep`.
  Our state_dump currently does NOT expose
  `vibrato_envelope_state.{vibrato_pos,vibrato_amp,vibrato_sweep}`.
  Add those fields to `xmplayer/src/song/test_dump.rs::VoiceDump`
  before comparing auto-vibrato semantics.
- **Envelope position**: our `volume_envelope_pos` ↔ FT2 `envVPos`
  (point index), but FT2 also exposes the inter-point `envVAmp`
  (interpolated value × 256). Our existing
  `volume_envelope_state.frame` is a different field; verify
  semantics before drawing conclusions from a mismatch.

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
