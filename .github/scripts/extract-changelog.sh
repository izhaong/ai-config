#!/usr/bin/env bash
# Extract Keep a Changelog section for VERSION into OUTPUT file.
set -euo pipefail

VERSION="${1:-}"
OUTPUT="${2:-}"
if [[ -z "$VERSION" || -z "$OUTPUT" ]]; then
  echo "usage: extract-changelog.sh X.Y.Z output.md" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
CHANGELOG="$ROOT/CHANGELOG.md"
VERSION="${VERSION#v}"

if ! grep -q "^## \\[$VERSION\\]" "$CHANGELOG"; then
  echo "CHANGELOG.md has no section ## [$VERSION]" >&2
  exit 1
fi

awk -v ver="$VERSION" '
  $0 ~ "^## \\[" ver "\\]" { capture=1; next }
  capture && /^## \[/ { exit }
  capture { print }
' "$CHANGELOG" >"$OUTPUT"

if [[ ! -s "$OUTPUT" ]]; then
  echo "CHANGELOG section ## [$VERSION] is empty" >&2
  exit 1
fi

echo "Wrote release notes to $OUTPUT"
