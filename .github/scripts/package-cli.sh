#!/usr/bin/env bash
# Build and archive ai-config CLI for the current runner triple.
set -euo pipefail

VERSION="${1:-}"
if [[ -z "$VERSION" ]]; then
  echo "usage: package-cli.sh X.Y.Z" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

cargo build -p ai-config-cli --release

TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
OUT_DIR="$ROOT/artifacts/cli"
mkdir -p "$OUT_DIR"

BIN="$ROOT/target/release/ai-config"
if [[ "${RUNNER_OS:-}" == "Windows" ]]; then
  BIN="$ROOT/target/release/ai-config.exe"
  ARCHIVE="$OUT_DIR/ai-config-${VERSION}-${TRIPLE}.zip"
  (cd "$(dirname "$BIN")" && zip -j "$ARCHIVE" "$(basename "$BIN")")
else
  ARCHIVE="$OUT_DIR/ai-config-${VERSION}-${TRIPLE}.tar.gz"
  tar -C "$(dirname "$BIN")" -czf "$ARCHIVE" "$(basename "$BIN")"
fi

echo "archive=$ARCHIVE"
