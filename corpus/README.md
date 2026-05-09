# Module corpus harness

Auto-comparison sweep against an OpenMPT reference render. Build the
corpus once, then run sweeps to surface modules where our engine
diverges from libopenmpt.

## Build the corpus

`tools/build_corpus.sh` scans one or more directories for tracker
modules, hashes them by content, copies new ones into
`corpus/<format>/<hash>.<ext>`, and appends a row per file to
`corpus/manifest.tsv`. Re-running is idempotent — already-present
hashes are skipped.

```sh
tools/build_corpus.sh ~/Downloads
tools/build_corpus.sh ~/Downloads ~/Music/mods   # multiple roots OK
```

The OpenMPT project also publishes a small playback-test corpus under
`test/` in `https://github.com/OpenMPT/openmpt`. Cloning it and
pointing the script at the `test/` directory is a good way to seed
known-quirk regression cases.

## Sweep

`scripts/sweep.py` picks a small random sample, renders each module
with both engines, walks 1.0-second windows, and flags any that
diverge.

```sh
scripts/sweep.py --limit 5 --end-time 180
# Reproduce a previous run:
scripts/sweep.py --seed 1234 --limit 5
# Re-run already-processed modules:
scripts/sweep.py --force
```

`done.tsv` accumulates one row per processed module so subsequent
runs pick fresh modules. Pass `--force` to ignore it.

The flagging heuristic:

* **rms-ratio** — window's `ours_rms / ref_rms` outside `[--rms-low,
  --rms-high]` (default `[0.5, 2.0]`). Tolerates global gain drift
  from envelope/master differences.
* **pitch-shift** — ≥ 2 of the top 6 peaks deviate more than
  `--cents-thresh` (default `10`) from their nearest peak in the
  reference. Tolerates FFT bin quantization.
* **missing-peaks** — ≥ 3 of the top 6 reference peaks have no peak
  within 6 Hz in our render. Catches missing/extra notes.

Reports land at `corpus/report.md` by default.

## Layout

```
corpus/
  manifest.tsv          # hash → format / size / original_path / name
  done.tsv              # processed hashes (status from latest sweep)
  report.md             # latest sweep output
  renders/<hash>.{ours,ref}.wav   # per-module rendered audio
  s3m/<hash>.s3m
  xm/<hash>.xm
  it/<hash>.it
  mod/<hash>.mod
  stm/<hash>.stm
```

The manifest, done file, renders, and module copies are all
gitignored — only this README and the harness scripts are tracked.
