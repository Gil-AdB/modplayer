---
name: detect-render-bugs
description: Diff our render against an external reference (openmpt123 / ft2-clone), localize the bad windows, attribute them to channels, and turn the channel pattern into a code-side hypothesis.
---

# Detect render bugs

When the user reports an audible defect ("missing notes", "wrong slide", "off-tune"), or asks to compare a module against a reference, follow this flow. Don't skip steps — early steps prevent later ones from wasting time on phantom bugs.

## The flow at a glance

1. **Render both engines** to WAVs of the same duration.
2. **Headline window-scan** — how widespread is the divergence?
3. **Pitch distribution** — is it systematic, scattered, or bimodal?
4. **Per-channel bisect at the worst window** — which channel(s)?
5. **Read the per-channel pattern** — what *kind* of bug?
6. **Pattern row → code-side hypothesis → fix → re-render**.

Don't commit to "no bug" until step 4. Headline window-scan numbers are noisy — most flagged windows turn out to be phase drift on the bulk of channels.

## Step 1: render both

```bash
cargo build --release --bin render_wav
openmpt123 --quiet --render --force /path/to/song.xm   # writes song.xm.wav next to input
./target/release/render_wav /path/to/song.xm /tmp/ours.wav --end-time 252
```

`openmpt123 --render` writes alongside the input; move/copy as needed. Match `--end-time` to the song's full duration (`openmpt123 --info song.xm` shows it).

## Step 2: headline window-scan

```bash
python3 scripts/compare_renders.py /tmp/ours.wav /path/song.xm.wav --window 0.5 --top 30
```

Reads per-window: RMS ratio, top-FFT-peak cents-diff, cross-correlation, unmatched-peak count. Prints the worst N windows by composite score.

**Interpret the headline count.** If "503 windows, 301 flagged" — that's just the cent threshold biting on noisy windows. Don't react to the number; characterize the divergence (step 3) before deciding anything's wrong.

Tighten thresholds to focus on amplitude/timing bugs (ignoring incidental pitch noise):

```bash
python3 scripts/compare_renders.py ours.wav ref.wav \
    --rms-low 0.4 --rms-high 2.5 --cents-thresh 100 --cc-low 0.5
```

## Step 3: cents distribution

```bash
python3 scripts/cents_distribution.py /tmp/ours.wav /path/song.xm.wav
```

Look at the strong-peak distribution. Three diagnostic patterns:

| Median (strong) | Pct ±5c (strong) | Diagnosis |
|---|---|---|
| ~0c | >80% | Pitch is fine. Stop chasing pitch — look for amplitude / timing bugs. |
| ±10c or more | <40% | Systematic offset. Suspect frequency table, sample rate conversion, finetune calibration. |
| ~0c, but bimodal in the histogram | mid | One channel out of tune. Move to channel bisect. |

86% of strong peaks within ±5c on a real-world song is "in tune". 60% within ±5c with 30% beyond ±15c is a real pitch bug.

## Step 4: per-channel bisect

You need libopenmpt's per-channel solo. Build once:

```bash
bash tools/build_openmpt_solo.sh   # produces target/release/openmpt_solo
```

Then pick a worst-divergence timestamp from step 2 and bisect:

```bash
python3 scripts/bisect_channel.py /path/song.xm 70.5 --channels 18 --end-time 75
```

Output shows RMS-ours, RMS-omt, ratio, cc per channel. The script flags:
- `LOUDNESS BUG` — ratio outside [0.5, 2.0]. A real bug; chase next.
- `cc drift / different waveform` — cc < 0.5 with matching RMS. Could be phase drift (audibly fine) or genuinely different content. See step 5.
- `some drift` — cc 0.5–0.85. Usually accumulated float-position drift; not a bug.

`--channels` must match the module's channel count (see `openmpt123 --info`).

## Step 5: read the per-channel pattern

This is the diagnostic table. Run the per-second cc/RMS dump from `peek_window.py` or inline Python on the one suspicious channel for 60-90 seconds of audio to see the pattern.

| Pattern | Likely cause |
|---|---|
| RMS matches exactly; cc cycles `+1 → 0 → +1` every few seconds | Float `sample_position` accumulated drift between retriggers. **Audibly inaudible**, not a bug worth chasing — OMT uses fixed-point internally. |
| RMS matches exactly; cc gradually drops from 1.0 to 0 over many seconds, never recovers | Same float drift, no retriggers. Same diagnosis. |
| RMS spike (>40%) at a specific second, then back to normal | Real loudness bug. A volume effect / retrig is doing the wrong thing at that row. **Chase this.** |
| RMS persistently off (>30%) across whole channel | Gain calibration error on whatever envelope/master path this channel uses. |
| Cc ≈ 1.0 for most ticks, but **misaligned peaks** in `peek_window.py` (e.g. our 66 Hz vs ref 62 Hz on the strongest peak) | Wrong note playing — finetune, transpose, or porta-target error. **Chase this.** |
| Top peaks identical, cc near 0 | Pure phase drift (often from sub-sample interpolation). Both engines are playing the same notes; the waveforms just don't line up sample-for-sample. Not a bug. |

## Step 6: pattern row → code hypothesis

Once a single channel + timestamp is identified, find the pattern row responsible:

```bash
# Approximate row index from tempo and speed (varies per song):
# tempo (BPM) determines tick rate (2500 / BPM ms per tick).
# speed (ticks/row) determines row rate (speed * tick_ms).
# Most XMs default to speed 6, BPM 125 → 120ms/row.
# row_index ≈ t_seconds * 1000 / row_ms
```

Or use the `[OUR]` per-tick state dump (set `OUR_DUMP_CH=<idx>` and run `render_wav`) to see exactly what happened on that channel around that time. Pair with `tools/openmpt_instrumentation.patch` if you need OMT's matching `[OMT]` trace.

From row + effect column, the hypothesis usually writes itself:
- LOUDNESS BUG on a row with `Cxx` → volume column / instrument retrig path.
- LOUDNESS BUG on a row with no effect → likely envelope / fadeout / note-cut wrong.
- Wrong pitch on a row with `3xx` / `5xx` → porta / glissando.
- Wrong pitch on a row with `0xy` → arpeggio / period_shift carryover.
- Missing/extra peaks on rows with `EDx` → note delay timing.

## Common pitfalls

- **Don't chase phase drift.** RMS-matched + low-cc over long stretches is float-arithmetic drift between the two engines, not a bug. Listen — if it sounds the same, it is the same.
- **Don't trust the headline flag count.** With default thresholds 50-60% of windows often flag on real songs. Always characterize via cents-distribution before reacting.
- **Cents threshold = 15 flags too much.** A single 30c peak on a quiet harmonic flags an otherwise-clean window. Use the strong-peak (top-quarter) variant or raise to 30-50c.
- **Stereo image differences inflate flag counts on MOD.** ProTracker hard-pans LRRL; our mock test harness centers all channels. Render mono-mix for comparison if needed (`compare_renders.py` already does this — both signals get mean'd to mono).
- **Save the WAVs.** Re-rendering 4-minute songs takes 10+ seconds each. Keep them in `/tmp` and reuse across multiple comparison runs.

## What changes are worth committing

If a per-channel bisect shows a LOUDNESS BUG or **wrong note** that pairs to a specific pattern row + effect, that's a real bug. Fix the effect handler, add a synthetic test (`xmplayer/src/xm_fidelity_tests.rs` or `tests/effects_fidelity.rs`), and re-bisect to confirm.

If the bisect shows phase drift only — no fix needed. Document that the module's residual divergence is float-arithmetic drift, and move on.

## Tools recap

- `scripts/compare_renders.py` — window-by-window FFT/RMS/cc comparison.
- `scripts/cents_distribution.py` — pitch-divergence histogram, "systematic vs scattered".
- `scripts/peek_window.py` — top FFT peaks of one ~500ms window from each render.
- `scripts/bisect_channel.py` — per-channel solo render + comparison at a target window.
- `scripts/sweep.py` — corpus mode of compare_renders.py over many modules.
- `tools/openmpt_solo.c` — libopenmpt-linked per-channel solo renderer (build via `tools/build_openmpt_solo.sh`).
- `tools/openmpt_instrumentation.patch` — adds an `[OMT]` per-tick channel-state trace to libopenmpt for direct diffing against our `[OUR]` dump.
