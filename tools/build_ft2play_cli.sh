#!/usr/bin/env bash
# Build ft2play with a CLI `--render <output.wav>` flag bolted on.
# ft2play is the canonical replayer for XM files (direct C port of
# FastTracker 2's replayer by 8bitbubsy). Used as the canonical
# baseline for XM in scripts/corpus_regression.py — same role pt2-clone
# plays for MOD, st3play plays for S3M, and it2play plays for IT.
#
# Prerequisites:
#   - SDL2 installed via Homebrew (brew install sdl2)
#   - ft2play source cloned somewhere; set FT2_SRC if not /tmp/ft2play
#
# Output: $FT2_SRC/ft2play-cli  (an SDL2-linked binary that:
#   - with `<song.xm> --render <out.wav>`: renders headlessly, exits)
#
# Usage:
#   tools/build_ft2play_cli.sh
#   /tmp/ft2play/ft2play-cli /path/to/song.xm --render /tmp/out.wav

set -euo pipefail
FT2_SRC="${FT2_SRC:-/tmp/ft2play}"
REPO="$(cd "$(dirname "$0")/.." && pwd)"

if [[ ! -f "$FT2_SRC/ft2play/src/ft2play.c" ]]; then
    echo "ft2play source not found at $FT2_SRC. Set FT2_SRC or run:" >&2
    echo "  git clone --depth 1 https://github.com/8bitbubsy/ft2play.git $FT2_SRC" >&2
    exit 1
fi

# Apply the --render CLI patch (idempotent: skip if grep hits).
if ! grep -q "renderOutPath" "$FT2_SRC/ft2play/src/ft2play.c"; then
    patch -d "$FT2_SRC" -p1 < "$REPO/tools/ft2play_cli_render.patch"
fi

# Upstream's macOS recipe uses AudioQueue + Cocoa. Use SDL2 instead to
# match the rest of our canonical CLI binaries — simpler builds, no
# extra framework dependencies.
cd "$FT2_SRC"
clang -O3 -DNDEBUG -DAUDIODRIVER_SDL \
    $(sdl2-config --cflags) \
    audiodrivers/sdl/*.c *.c ft2play/src/*.c \
    $(sdl2-config --libs) -framework Cocoa -lm \
    -o "$FT2_SRC/ft2play-cli"

echo "built: $FT2_SRC/ft2play-cli"
