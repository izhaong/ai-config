#!/usr/bin/env bash
# One-shot local adaptation for ~/.agents after agents-manager asset-root migration.
# - Removes nested ~/.agents/.agents projection copies (skills already present at top level)
# - Splits monolithic ~/.agents/mcp.json into ~/.agents/mcp/servers/<name>.json
# - Extends ~/.agents/.gitignore for mcp-secrets.env*
#
# Does NOT auto-migrate ~/.ai-config or ~/.agents-manager. Review doctor --json legacy_asset_roots.
set -euo pipefail

HOME_AGENTS="${HOME}/.agents"
if [[ ! -d "$HOME_AGENTS" ]]; then
  echo "skip: $HOME_AGENTS does not exist" >&2
  exit 0
fi

NESTED="${HOME_AGENTS}/.agents"
if [[ -d "$NESTED" ]]; then
  echo "removing nested projection copy: $NESTED"
  rm -rf "$NESTED"
fi

MCP_JSON="${HOME_AGENTS}/mcp.json"
SERVERS="${HOME_AGENTS}/mcp/servers"
if [[ -f "$MCP_JSON" ]]; then
  mkdir -p "$SERVERS"
  python3 - "$MCP_JSON" "$SERVERS" <<'PY'
import json, re, sys
from pathlib import Path
mcp_path = Path(sys.argv[1])
out = Path(sys.argv[2])
data = json.loads(mcp_path.read_text())
servers = data.get("mcpServers") or data.get("servers") or {}
for name, cfg in servers.items():
    safe = re.sub(r"[^A-Za-z0-9._-]+", "_", name)
    path = out / f"{safe}.json"
    path.write_text(json.dumps(cfg, indent=2, ensure_ascii=False) + "\n")
    print(f"wrote {path}")
bak = mcp_path.with_name("mcp.json.bak-agents-root-migration")
if not bak.exists():
    mcp_path.rename(bak)
    print(f"renamed {mcp_path.name} -> {bak.name}")
else:
    print(f"backup exists ({bak.name}); left {mcp_path.name} in place")
PY
fi

GI="${HOME_AGENTS}/.gitignore"
touch "$GI"
for pattern in 'mcp-secrets.env' 'mcp-secrets.env*' '*.secret.key' '.secret.key'; do
  if ! grep -qxF "$pattern" "$GI"; then
    echo "$pattern" >>"$GI"
    echo "gitignore += $pattern"
  fi
done

echo "done. Run: agents-manager doctor --json | jq .legacy_asset_roots"