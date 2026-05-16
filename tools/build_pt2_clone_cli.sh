#!/usr/bin/env bash
# Build pt2-clone with a CLI `--render <output.wav>` flag bolted on.
# pt2-clone is the reference MOD player for cases where libopenmpt
# diverges from authentic ProTracker semantics (e.g. SC2/Redalert.mod).
#
# Prerequisites:
#   - SDL2 installed via Homebrew (brew install sdl2)
#   - pt2-clone source cloned somewhere; set PT2_SRC if not /tmp/pt2-clone
#
# Output: $PT2_SRC/pt2-clone-cli  (a SDL2-linked binary that:
#   - with no args: launches the GUI as normal
#   - with `<mod> --render <out.wav>`: skips GUI, renders, exits)
#
# Usage:
#   tools/build_pt2_clone_cli.sh
#   /tmp/pt2-clone/pt2-clone-cli /path/to/song.mod --render /tmp/out.wav

set -euo pipefail
PT2_SRC="${PT2_SRC:-/tmp/pt2-clone}"
REPO="$(cd "$(dirname "$0")/.." && pwd)"

if [[ ! -f "$PT2_SRC/src/pt2_main.c" ]]; then
    echo "pt2-clone source not found at $PT2_SRC. Set PT2_SRC or run:" >&2
    echo "  git clone --depth 1 https://github.com/8bitbubsy/pt2-clone.git $PT2_SRC" >&2
    exit 1
fi

# Apply the --render CLI patch (idempotent: skip if grep hits).
if ! grep -q "\\-\\-render" "$PT2_SRC/src/pt2_main.c"; then
    patch -d "$PT2_SRC" -p1 < "$REPO/tools/pt2_clone_cli_render.patch"
fi

cd "$PT2_SRC"
clang -O2 $(sdl2-config --cflags) -DNDEBUG \
    src/gfx/*.c src/modloaders/*.c src/smploaders/*.c src/*.c \
    $(sdl2-config --libs) -framework Cocoa -lm \
    -o "$PT2_SRC/pt2-clone-cli"

echo "built: $PT2_SRC/pt2-clone-cli"
