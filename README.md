# agents-manager

[![CI](https://img.shields.io/github/actions/workflow/status/izhaong/agents-manager/ci.yml?branch=develop&label=CI)](https://github.com/izhaong/agents-manager/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/izhaong/agents-manager?label=release)](https://github.com/izhaong/agents-manager/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.78%2B-orange.svg)](https://www.rust-lang.org/)

**Unified skills, rules, agents, and MCP management for multi-agent IDEs** — one user asset tree (`~/.agents/`), synced to **Cursor**, **Codex**, **Claude Code**, and **Hermes**.

> **中文简介**：在多 Agent IDE 之间统一管理 Skills、Rules、Agents 与 MCP；用户资产存放在 `~/.agents/`（Agent Skills 规范目录，MCP 为 `mcp/servers/*.json`），本仓库只提供 Rust CLI / Tauri GUI 与项目模板，不包含你的私有 skills 内容。

---

## Supported IDEs

| IDE             | Typical paths (after `install`)                                         |
| --------------- | ----------------------------------------------------------------------- |
| **Cursor**      | `~/.agents/skills/` (shared), `~/.cursor/agents/`, `~/.cursor/mcp.json` |
| **Codex**       | `~/.agents/skills/` (shared), `~/.codex/subagents/`                      |
| **Claude Code** | `~/.claude/skills/`, `~/.claude/subagents/`, rules via symlinks         |
| **Hermes**      | `~/.hermes/skills/`, `~/.hermes/agents/` (optional `HERMES_SKILLS_DIR`) |

---

## Architecture

```text
~/.agents/                 # user asset root (Agent Skills; not in this git repo)
├── skills/<name>/SKILL.md
├── rules/*.mdc
├── agents/
├── commands/
├── hooks/ + hooks.json
├── prompts/AGENTS.md
└── mcp/servers/<name>.json  # canonical MCP source → projected to IDE configs

~/.config/agents-manager/
└── secrets.env               # secrets only (0600); never commit

agents-manager (this repo)
├── crates/agents-manager-core     # sync, templates, platforms
├── crates/agents-manager-cli      # `agents-manager` binary
├── apps/agents-manager-gui        # Tauri desktop (optional)
├── templates/project/        # scaffold for new repos
└── manifests/                # plugin / asset manifests
```

Install keeps IDE skill paths discoverable: Cursor/Codex consume `~/.agents/skills` directly (Agent Skills). Claude gets links under `.claude/skills`. MCP deploy reads **`~/.agents/mcp/servers/*.json`** (or legacy `mcp.json`) and writes the target IDE file.

Details: [docs/product/ARCHITECTURE.md](docs/product/ARCHITECTURE.md) · [AGENTS.md](AGENTS.md)

---

## Quick start

```bash
git clone https://github.com/izhaong/agents-manager.git
cd agents-manager

cargo build -p agents-manager-cli --release
./target/release/agents-manager install
```

First run ensures `~/.agents/{skills,rules,agents,commands,hooks}` and prefers canonical MCP at `~/.agents/mcp/servers/*.json` (legacy root `mcp.json` can be split via `scripts/migrate-agents-root.sh`).

**Sync everything once:**

```bash
./target/release/agents-manager sync
```

**Deploy MCP to one platform:**

```bash
./target/release/agents-manager mcp deploy mcp.json cursor
```

**Optional Hermes skills dir:**

```bash
export HERMES_SKILLS_DIR="$HOME/.hermes/skills"
./target/release/agents-manager install
```

Put real API keys in `~/.config/agents-manager/secrets.env` (see `templates/project` examples). Never commit secrets.

---

## CLI commands

| Command                    | Purpose                                            |
| -------------------------- | -------------------------------------------------- |
| `install`                  | One-shot setup: dirs, symlinks, optional migration |
| `uninstall`                | Remove managed links; backup MCP files             |
| `sync`                     | Push skills/rules/MCP to all configured platforms  |
| `status`                   | Show last sync state                               |
| `list` / `show`            | List or inspect managed assets                     |
| `doctor`                   | Health check (`--json` for agents)                 |
| `secrets`                  | Manage `secrets.env` helpers                       |
| `skill` / `rule` / `agent` | Per-asset list / show / reveal                     |
| `mcp`                      | MCP server ops (incl. `deploy` / `retract`)        |
| `daemon`                   | Background watcher (when enabled)                  |
| `gui`                      | Launch Tauri UI                                    |
| `completion`               | Shell completions                                  |

Global flags: `--json`, `--quiet`, `--root <PATH>` (or `AGENTS_MANAGER_ROOT`).

```bash
agents-manager --help
agents-manager mcp --help
```

---

## Relationship to zh-cloud

[zh-cloud](https://github.com/izhaong/zh-cloud) is a larger monorepo; **agents-manager** can live beside it as an optional submodule or standalone clone. This repository is **independently open-sourced** — you do not need zh-cloud to use the CLI. Internal Gitea development may continue in parallel; see **Remotes** below.

---

## Development

```bash
cargo fmt
cargo clippy -p agents-manager-core -p agents-manager-cli -p agents-manager-store -p agents-manager-daemon -p agents-manager-bus -p agents-manager-watcher -- -D warnings
cargo test -p agents-manager-core -p agents-manager-cli -p agents-manager-store
```

**GUI (Tauri)** — bilingual UI (中文 / English), language switcher in the top bar; locale files under `apps/agents-manager-gui/src/i18n/locales/`. While the app is running, it watches `~/.agents/{skills,rules,agents,commands,hooks}` and `mcp/servers` (plus legacy `mcp.json`) (plus registered project asset roots) and refreshes the asset list and per-platform link status automatically (~200ms debounce).

```bash
cd apps/agents-manager-gui && npm install && npm run tauri:dev
```

CI runs on **Ubuntu + macOS**; GUI (Tauri) desktop bundles are built on **tagged releases** (`vX.Y.Z`). See [CONTRIBUTING.md](CONTRIBUTING.md#releases-github).

Contributing: [CONTRIBUTING.md](CONTRIBUTING.md) · Security: [SECURITY.md](SECURITY.md)

---

## Git remotes (Gitea + GitHub)

This project is developed on Gitea and mirrored to GitHub.

| Remote   | URL                                                                   |
| -------- | --------------------------------------------------------------------- |
| `origin` | `https://gitea.example.com/your-org/agents-manager.git` (primary internal) |
| `github` | `https://github.com/izhaong/agents-manager.git` (public mirror)            |

```bash
git remote add github https://github.com/izhaong/agents-manager.git   # once
git push -u github develop
git push github docs/github-open-source   # feature branches as needed
```

Push to **both** when releasing:

```bash
git push origin develop && git push github develop
```

---

## Troubleshooting

| Issue                     | Action                                    |
| ------------------------- | ----------------------------------------- |
| Cursor cannot see a skill | Run `agents-manager install`, restart Cursor   |
| MCP missing env vars      | Fill `secrets.env`, then `agents-manager sync` |
| Empty skill list          | Add folders under `~/.agents/skills/`  |

---

## License

MIT — see [LICENSE](LICENSE).
