#!/usr/bin/env bash
# Build st3play with a CLI `--render <output.wav>` flag bolted on.
# st3play is the canonical replayer for S3M files (direct C port of
# Scream Tracker 3.21's replayer by 8bitbubsy). Used as the canonical
# baseline for S3M in scripts/corpus_regression.py — same role pt2-clone
# plays for MOD.
#
# Prerequisites:
#   - SDL2 installed via Homebrew (brew install sdl2)
#   - st3play source cloned somewhere; set ST3_SRC if not /tmp/st3play
#
# Output: $ST3_SRC/st3play-cli  (an SDL2-linked binary that:
#   - with `<song.s3m> --render <out.wav>`: renders headlessly, exits)
#
# Usage:
#   tools/build_st3play_cli.sh
#   /tmp/st3play/st3play-cli /path/to/song.s3m --render /tmp/out.wav

set -euo pipefail
ST3_SRC="${ST3_SRC:-/tmp/st3play}"
REPO="$(cd "$(dirname "$0")/.." && pwd)"

if [[ ! -f "$ST3_SRC/st3play/src/st3play.c" ]]; then
    echo "st3play source not found at $ST3_SRC. Set ST3_SRC or run:" >&2
    echo "  git clone --depth 1 https://github.com/8bitbubsy/st3play.git $ST3_SRC" >&2
    exit 1
fi

# Apply the --render CLI patch (idempotent: skip if grep hits).
if ! grep -q "renderOutPath" "$ST3_SRC/st3play/src/st3play.c"; then
    patch -d "$ST3_SRC" -p1 < "$REPO/tools/st3play_cli_render.patch"
fi

# st3play's make scripts are platform-specific; replicate the macOS-arm
# recipe here but link against Homebrew SDL2 instead of the framework.
cd "$ST3_SRC"
# digread.c uses NULL without including stddef.h directly; force-include
# stddef so the macOS clang/Homebrew SDL2 build path matches what the
# upstream script implicitly got via /Library/Frameworks/SDL2 headers.
clang -O3 -DNDEBUG -DAUDIODRIVER_SDL -include stddef.h \
    $(sdl2-config --cflags) \
    audiodrivers/sdl/*.c *.c mixer/*.c opl2/*.c st3play/src/*.c \
    $(sdl2-config --libs) -framework Cocoa -lm \
    -o "$ST3_SRC/st3play-cli"

echo "built: $ST3_SRC/st3play-cli"
