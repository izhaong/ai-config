#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
STATE_DIR="${SCRIPT_DIR}/.test-state"
mkdir -p "$STATE_DIR"
echo "beforeSubmitPrompt $(date +%s)" >> "$STATE_DIR/prompt-hook.log"
echo '{"continue":true}'
