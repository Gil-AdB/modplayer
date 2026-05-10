#!/usr/bin/env bash
# Build an instrumented libopenmpt + a matching openmpt_solo_inst binary.
# The instrumented openmpt_solo dumps per-tick channel state (period,
# freq, vibrato pos/depth/speed, volume, sample pos/inc) to stderr,
# letting us diff against our engine's state_dump output and isolate
# vibrato / porta / interpolation bugs that the audio-level harness
# can't pin down.
#
# Usage:
#   1. Make sure OpenMPT source is at $OPENMPT_SRC (default /tmp/openmpt).
#      Clone with `git clone https://source.openmpt.org/svn/openmpt/...`
#      or download a release tarball.
#   2. Run this script. It applies tools/openmpt_instrumentation.patch
#      and rebuilds libopenmpt.a + target/release/openmpt_solo_inst.
#   3. To dump:
#        OMT_DUMP_CH=14 ./target/release/openmpt_solo_inst song.xm out.wav \
#            --end-time 50 2> ch14_dump.txt
#      Grep for `^[OMT]` lines.
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
SRC="${OPENMPT_SRC:-/tmp/openmpt}"

if [[ ! -f "$SRC/soundlib/Sndmix.cpp" ]]; then
    echo "OpenMPT source not found at $SRC. Set OPENMPT_SRC." >&2
    exit 1
fi

cd "$SRC"
# Apply the patch if not already applied (idempotent: skip if grep hits).
if ! grep -q "FEAT/S3M-REFACTOR INSTRUMENTATION" soundlib/Sndmix.cpp; then
    patch -p1 < "$REPO/tools/openmpt_instrumentation.patch"
fi
touch soundlib/Sndmix.cpp
make CONFIG=macos NO_PORTAUDIO=1 NO_SDL2=1 NO_MPG123=1 NO_OGG=1 \
     NO_VORBIS=1 NO_VORBISFILE=1 NO_PULSEAUDIO=1 NO_PORTAUDIOCPP=1 \
     NO_FLAC=1 NO_SNDFILE=1 NO_TEST=1 NO_EXAMPLES=1 NO_OPENMPT123=1 \
     >/dev/null

cd "$REPO"
cc -O2 -Wall -Wextra \
    -I"$SRC/libopenmpt" \
    -I/opt/homebrew/include \
    -o "$REPO/target/release/openmpt_solo_inst" \
    "$REPO/tools/openmpt_solo.c" \
    "$SRC/bin/libopenmpt.a" \
    -lz -lstdc++ -lm

echo "built: $REPO/target/release/openmpt_solo_inst"
echo "usage: OMT_DUMP_CH=<n> $REPO/target/release/openmpt_solo_inst <module> <wav> --end-time <sec> 2> dump.txt"
