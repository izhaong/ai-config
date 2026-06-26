#!/usr/bin/env bash
set -euo pipefail

STATE_DIR=".cursor/hooks/.test-state"
mkdir -p "$STATE_DIR"
echo "beforeSubmitPrompt $(date +%s)" >> "$STATE_DIR/prompt-hook.log"
