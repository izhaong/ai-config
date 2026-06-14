# ai-config

[![CI](https://img.shields.io/github/actions/workflow/status/izhaong/ai-config/ci.yml?branch=develop&label=CI)](https://github.com/izhaong/ai-config/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.78%2B-orange.svg)](https://www.rust-lang.org/)

**Unified skills, rules, agents, and MCP management for multi-agent IDEs** — one user asset tree (`~/.ai-config/`), synced to **Cursor**, **Codex**, **Claude Code**, and **Hermes**.

> **中文简介**：在多 Agent IDE 之间统一管理 Skills、Rules、Agents 与 MCP；用户资产存放在 `~/.ai-config/`（含单一明文 `mcp.json`），本仓库只提供 Rust CLI / Tauri GUI 与项目模板，不包含你的私有 skills 内容。

---

## Supported IDEs

| IDE | Typical paths (after `install`) |
| --- | --- |
| **Cursor** | `~/.cursor/skills/`, `~/.cursor/mcp.json` |
| **Codex** | `~/.codex/skills/` |
| **Claude Code** | `~/.claude/skills/`, rules via project symlinks |
| **Hermes** | `~/.hermes/skills/` (optional `HERMES_SKILLS_DIR`) |

---

## Architecture

```text
~/.ai-config/                 # user asset root (not in git)
├── skills/<name>/SKILL.md
├── rules/*.mdc
├── agents/
└── mcp.json                  # single plaintext MCP catalog → copied/merged to IDE configs

~/.config/ai-config/
└── secrets.env               # secrets only (0600); never commit

ai-config (this repo)
├── crates/ai-config-core     # sync, templates, platforms
├── crates/ai-config-cli      # `ai-config` binary
├── apps/ai-config-gui        # Tauri desktop (optional)
├── templates/project/        # scaffold for new repos
└── manifests/                # plugin / asset manifests
```

Install creates symlinks from each IDE’s skill (and rule) paths into `~/.ai-config/`. MCP deploy reads **`~/.ai-config/mcp.json`** and writes the target IDE file (e.g. full copy to Cursor).

Details: [docs/product/ARCHITECTURE.md](docs/product/ARCHITECTURE.md) · [AGENTS.md](AGENTS.md)

---

## Quick start

```bash
git clone https://github.com/izhaong/ai-config.git
cd ai-config

cargo build -p ai-config-cli --release
./target/release/ai-config install
```

First run ensures `~/.ai-config/{skills,rules,agents,mcp.json}`. Legacy `~/.ai-config/mcp/servers/` is merged into `mcp.json` when present.

**Sync everything once:**

```bash
./target/release/ai-config sync
```

**Deploy MCP to one platform:**

```bash
./target/release/ai-config mcp deploy mcp.json cursor
```

**Optional Hermes skills dir:**

```bash
export HERMES_SKILLS_DIR="$HOME/.hermes/skills"
./target/release/ai-config install
```

Put real API keys in `~/.config/ai-config/secrets.env` (see `templates/project` examples). Never commit secrets.

---

## CLI commands

| Command | Purpose |
| --- | --- |
| `install` | One-shot setup: dirs, symlinks, optional migration |
| `uninstall` | Remove managed links; backup MCP files |
| `sync` | Push skills/rules/MCP to all configured platforms |
| `status` | Show last sync state |
| `list` / `show` | List or inspect managed assets |
| `doctor` | Health check (`--json` for agents) |
| `secrets` | Manage `secrets.env` helpers |
| `skill` / `rule` / `mcp` | Per-asset operations (incl. `mcp deploy` / `retract`) |
| `daemon` | Background watcher (when enabled) |
| `gui` | Launch Tauri UI |
| `completion` | Shell completions |

Global flags: `--json`, `--quiet`, `--root <PATH>` (or `AI_CONFIG_ROOT`).

```bash
ai-config --help
ai-config mcp --help
```

---

## Relationship to zh-cloud

[zh-cloud](https://github.com/izhaong/zh-cloud) is a larger monorepo; **ai-config** can live beside it as an optional submodule or standalone clone. This repository is **independently open-sourced** — you do not need zh-cloud to use the CLI. Internal Gitea development may continue in parallel; see **Remotes** below.

---

## Development

```bash
cargo fmt
cargo clippy -p ai-config-core -p ai-config-cli -p ai-config-store -p ai-config-daemon -p ai-config-bus -p ai-config-watcher -- -D warnings
cargo test -p ai-config-core -p ai-config-cli
```

GUI (Tauri) builds locally with extra deps; CI runs Rust library/CLI crates on Ubuntu.

Contributing: [CONTRIBUTING.md](CONTRIBUTING.md) · Security: [SECURITY.md](SECURITY.md)

---

## Git remotes (Gitea + GitHub)

This project is developed on Gitea and mirrored to GitHub.

| Remote | URL |
| --- | --- |
| `origin` | `https://gitea.example.com/your-org/ai-config.git` (primary internal) |
| `github` | `https://github.com/izhaong/ai-config.git` (public mirror) |

```bash
git remote add github https://github.com/izhaong/ai-config.git   # once
git push -u github develop
git push github docs/github-open-source   # feature branches as needed
```

Push to **both** when releasing:

```bash
git push origin develop && git push github develop
```

---

## Troubleshooting

| Issue | Action |
| --- | --- |
| Cursor cannot see a skill | Run `ai-config install`, restart Cursor |
| MCP missing env vars | Fill `secrets.env`, then `ai-config sync` |
| Empty skill list | Add folders under `~/.ai-config/skills/` |

---

## License

MIT — see [LICENSE](LICENSE).
