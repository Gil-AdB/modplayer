#!/usr/bin/env bash
# Build it2play with a CLI `--render <output.wav>` flag bolted on.
# it2play is the canonical replayer for IT files (direct C port of
# Impulse Tracker 2.15's replayer by 8bitbubsy). Used as the canonical
# baseline for IT in scripts/corpus_regression.py — same role pt2-clone
# plays for MOD and st3play plays for S3M.
#
# Prerequisites:
#   - SDL2 installed via Homebrew (brew install sdl2)
#   - it2play source cloned somewhere; set IT2_SRC if not /tmp/it2play
#
# Output: $IT2_SRC/it2play-cli  (an SDL2-linked binary that:
#   - with `<song.it> --render <out.wav>`: renders headlessly, exits)
#
# Usage:
#   tools/build_it2play_cli.sh
#   /tmp/it2play/it2play-cli /path/to/song.it --render /tmp/out.wav

set -euo pipefail
IT2_SRC="${IT2_SRC:-/tmp/it2play}"
REPO="$(cd "$(dirname "$0")/.." && pwd)"

if [[ ! -f "$IT2_SRC/it2play/src/it2play.c" ]]; then
    echo "it2play source not found at $IT2_SRC. Set IT2_SRC or run:" >&2
    echo "  git clone --depth 1 https://github.com/8bitbubsy/it2play.git $IT2_SRC" >&2
    exit 1
fi

# Apply the --render CLI patch (idempotent: skip if grep hits).
if ! grep -q "renderOutPath" "$IT2_SRC/it2play/src/it2play.c"; then
    patch -d "$IT2_SRC" -p1 < "$REPO/tools/it2play_cli_render.patch"
fi

# Replicate the upstream macOS-arm recipe but link against Homebrew SDL2
# instead of the framework (same approach as st3play-cli).
cd "$IT2_SRC"
clang -O3 -ffast-math -DNDEBUG -DAUDIODRIVER_SDL \
    $(sdl2-config --cflags) \
    audiodrivers/sdl/*.c it2drivers/*.c loaders/mmcmp/*.c loaders/*.c *.c it2play/src/*.c \
    $(sdl2-config --libs) -framework Cocoa -lm \
    -o "$IT2_SRC/it2play-cli"

echo "built: $IT2_SRC/it2play-cli"
