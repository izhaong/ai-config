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
PKG_VERSION="$(read_version "$ROOT/apps/ai-config-gui/package.json" '"version"')"
TAURI_VERSION="$(read_version "$ROOT/apps/ai-config-gui/src-tauri/tauri.conf.json" '"version"')"

fail=0
for label in Cargo.toml package.json tauri.conf.json; do
  case "$label" in
    Cargo.toml) got="$CARGO_VERSION" ;;
    package.json) got="$PKG_VERSION" ;;
    tauri.conf.json) got="$TAURI_VERSION" ;;
  esac
  if [[ "$got" != "$VERSION" ]]; then
    echo "Version mismatch: tag $TAG expects $VERSION but $label has $got" >&2
    fail=1
  fi
done

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

echo "Tag $TAG matches workspace version $VERSION"
