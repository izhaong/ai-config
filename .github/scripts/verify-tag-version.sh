#!/usr/bin/env bash
# Verify git tag vX.Y.Z matches workspace version fields.
set -euo pipefail

TAG="${1:-}"
if [[ -z "$TAG" ]]; then
  echo "usage: verify-tag-version.sh vX.Y.Z" >&2
  exit 1
fi

if [[ ! "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "Tag must match vX.Y.Z or vX.Y.Z-prerelease (got: $TAG)" >&2
  exit 1
fi

VERSION="${TAG#v}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

read_version() {
  local file="$1"
  local pattern="$2"
  grep -m1 "$pattern" "$file" | sed -E 's/.*"([^"]+)".*/\1/'
}

CARGO_VERSION="$(read_version "$ROOT/Cargo.toml" '^version = ')"
PKG_VERSION="$(read_version "$ROOT/apps/agents-manager-gui/package.json" '"version"')"
TAURI_ROOT="$(read_version "$ROOT/apps/agents-manager-gui/tauri.conf.json" '"version"')"
TAURI_SRC="$(read_version "$ROOT/apps/agents-manager-gui/src-tauri/tauri.conf.json" '"version"')"

fail=0
check() {
  local label="$1"
  local got="$2"
  if [[ "$got" != "$VERSION" ]]; then
    echo "Version mismatch: tag $TAG expects $VERSION but $label has $got" >&2
    fail=1
  fi
}

check "Cargo.toml" "$CARGO_VERSION"
check "package.json" "$PKG_VERSION"
check "apps/agents-manager-gui/tauri.conf.json (tauri-action)" "$TAURI_ROOT"
check "apps/agents-manager-gui/src-tauri/tauri.conf.json" "$TAURI_SRC"

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

echo "Tag $TAG matches workspace version $VERSION"
