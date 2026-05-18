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

- **SHOOTING.XM, 2026-05-18.** Per-channel analysis showed
  ch0–ch5 with cross-correlation 0.33–0.51 vs OMT, residual peaks at
  the 2nd harmonic of the channel fundamental. My first guess was
  "OMT uses sinc, we use cubic — call it interpolation". User
  correctly pushed back: project already has user-selectable
  interpolators, the difference is too large to be "just sinc vs
  cubic", and concluding without ft2play as a reference is
  premature. Investigation continues with ft2play built first.

The pattern across both: I reach for a "stylistic" explanation
(filter, interpolation, mix levels) because it's the *cheapest*
hypothesis to articulate, and only switch to looking for a real bug
when the user pushes back. Reverse the default — assume real bug,
articulate cheap hypothesis only as a foil to falsify, not as a
conclusion.

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

## When in doubt

Ask **what specifically** the user is hearing ("which channel",
"which moment in the song", "what's the symptom — pitch / timing /
timbre / missing notes"). One sentence from the user beats an hour
of unguided spectral analysis. See `feedback_trust_user_reports.md`
in user memory.
