#!/usr/bin/env bash
# macOS ships /bin/bash 3.2 which has no associative arrays — use a temp
# file of seen hashes for portability instead.
# Scan one or more source directories for tracker modules and copy unique
# ones (by content hash) into ./corpus/<format>/<hash>.<ext>. Write a TSV
# manifest mapping hash → original path / size / format.
#
# Usage:
#   tools/build_corpus.sh ~/Downloads [more dirs...]
#
# Re-running is idempotent: existing entries in corpus/manifest.tsv are
# preserved and only new (by content-hash) modules get added.

set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <source-dir> [<source-dir>...]" >&2
  exit 2
fi

REPO="$(cd "$(dirname "$0")/.." && pwd)"
CORPUS="$REPO/corpus"
MANIFEST="$CORPUS/manifest.tsv"

mkdir -p "$CORPUS"
for fmt in s3m xm it mod stm; do mkdir -p "$CORPUS/$fmt"; done

# Read existing hashes from the manifest. macOS bash 3.2 has no `-A`, so
# we keep them in a sorted text file and grep for membership.
SEEN_FILE=$(mktemp)
trap 'rm -f "$SEEN_FILE"' EXIT
if [[ -f "$MANIFEST" ]]; then
  awk -F'\t' 'NR>1 {print $1}' "$MANIFEST" > "$SEEN_FILE"
else
  echo -e "hash\tformat\tsize\toriginal_path\tname" > "$MANIFEST"
fi

added=0
scanned=0
for SRC in "$@"; do
  if [[ ! -d "$SRC" ]]; then
    echo "skip (not a dir): $SRC" >&2
    continue
  fi
  # find -E avoids the GNU/BSD regex split for the common module extensions.
  while IFS= read -r -d '' f; do
    scanned=$((scanned+1))
    ext="${f##*.}"
    ext_lc=$(echo "$ext" | tr '[:upper:]' '[:lower:]')
    case "$ext_lc" in
      s3m|xm|it|mod|stm) ;;
      *) continue ;;
    esac
    # Content hash: shasum is universally available on macOS + Linux.
    hash=$(shasum -a 256 "$f" | awk '{print substr($1,1,16)}')
    if grep -qx "$hash" "$SEEN_FILE"; then continue; fi
    size=$(stat -f '%z' "$f" 2>/dev/null || stat -c '%s' "$f")
    name=$(basename "$f")
    dest="$CORPUS/$ext_lc/$hash.$ext_lc"
    cp "$f" "$dest"
    # Tab-separated; original_path is intentionally last (may contain spaces).
    printf '%s\t%s\t%s\t%s\t%s\n' "$hash" "$ext_lc" "$size" "$f" "$name" >> "$MANIFEST"
    echo "$hash" >> "$SEEN_FILE"
    added=$((added+1))
  done < <(find "$SRC" -type f \( -iname '*.s3m' -o -iname '*.xm' -o -iname '*.it' -o -iname '*.mod' -o -iname '*.stm' \) -print0)
done

echo "scanned $scanned files, added $added new modules to corpus/"
echo "manifest: $MANIFEST"
echo "by format:"
for fmt in s3m xm it mod stm; do
  count=$(ls -1 "$CORPUS/$fmt" 2>/dev/null | wc -l | tr -d ' ')
  echo "  $fmt: $count"
done
