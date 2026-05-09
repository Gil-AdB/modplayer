#!/usr/bin/env bash
# Build the openmpt_solo standalone CLI. Picks up libopenmpt headers
# and library from Homebrew on macOS; override with OPENMPT_PREFIX
# env var if you have it elsewhere.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
PREFIX="${OPENMPT_PREFIX:-/opt/homebrew}"

cc="${CC:-cc}"
"$cc" -O2 -Wall -Wextra \
    -I"$PREFIX/include" \
    -L"$PREFIX/lib" \
    -o "$REPO/target/release/openmpt_solo" \
    "$REPO/tools/openmpt_solo.c" \
    -lopenmpt

echo "built: $REPO/target/release/openmpt_solo"
