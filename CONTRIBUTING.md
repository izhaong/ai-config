# Contributing to agents-manager

Thank you for improving agents-manager. This project uses **Git Flow** with **`develop`** as the integration branch.

## Getting started

1. Fork [izhaong/agents-manager](https://github.com/izhaong/agents-manager) on GitHub (or clone from your Gitea fork if you use the dual-remote setup).
2. Create a branch from `develop`: `feat/123-short-name` or `fix/123-short-name`.
3. Install [Rust 1.78+](https://rustup.rs/) and run:

   ```bash
   cargo fmt
   cargo test -p agents-manager-core -p agents-manager-cli
   ```

4. Open a Pull Request **into `develop`**.

## Spec Kit（非平凡功能）

本仓已接入 [Spec Kit](https://github.com/github/spec-kit)，**贴合 Cursor Plan/Todo**：

- `specs/<编号>-<名>/spec.md` — 需求与验收
- `specs/<编号>-<名>/plan.md` — 技术方案 + `## Todos`（执行清单）
- 宪法：`.specify/memory/constitution.md`；门禁：`.cursor/rules/spec-kit-gate.mdc`

贡献者只需用自然语言描述需求；Agent/维护者负责落盘 spec/plan。PR 请链到对应 `specs/` 目录。

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/) with a **Chinese summary**:

```text
feat(cli): 支持 mcp.json 单文件下发
fix(sync): 修复 Hermes 路径重复链接
docs: 更新 README Quick Start
```

- `type`: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`, …
- `scope`: optional module name (`cli`, `core`, `gui`, …)

## Code guidelines

- Business logic belongs in `crates/agents-manager-core`; CLI/GUI call core, do not duplicate sync rules.
- Do **not** commit real tokens, passwords, or private hostnames — use placeholders in tests and docs.
- User assets live under `~/.agents-manager/`; this repo ships code and templates only.

## Pull requests

- Link related issues (`Fixes #n` or `Refs #n`).
- Keep PRs focused; prefer several small PRs over one large diff.
- Ensure CI passes (fmt, clippy, tests).

## Releases (GitHub)

Version numbers must stay in sync across:

- `Cargo.toml` (`[workspace.package].version`)
- `apps/agents-manager-gui/package.json`
- `apps/agents-manager-gui/tauri.conf.json`（**tauri-action / GUI 安装包版本**）
- `apps/agents-manager-gui/src-tauri/tauri.conf.json`

Release checklist:

1. Move notes under `## [Unreleased]` into a new `## [X.Y.Z] - YYYY-MM-DD` section in [CHANGELOG.md](CHANGELOG.md).
2. Bump all version fields above to `X.Y.Z`.
3. Commit on `develop`, push to Gitea (`origin`) and GitHub (`github`).
4. Create and push an annotated tag:

   ```bash
   git tag -a vX.Y.Z -m "release: vX.Y.Z"
   git push origin vX.Y.Z
   git push github vX.Y.Z
   ```

5. GitHub Actions **Release** workflow validates the tag, builds CLI + Tauri GUI artifacts, and publishes [GitHub Releases](https://github.com/izhaong/agents-manager/releases) with CHANGELOG notes.

### GUI auto-update signing

Desktop builds use [Tauri updater](https://v2.tauri.app/plugin/updater/). The **public key** is in `apps/agents-manager-gui/tauri.conf.json`; maintainers must add repository secrets before release:

| Secret                               | Value                                                                                                                                                |
| ------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `TAURI_SIGNING_PRIVATE_KEY`          | Contents of `~/.tauri/agents-manager.key` (generate: `cd apps/agents-manager-gui && CI=true npm run tauri signer generate -- -w ~/.tauri/agents-manager.key -p ""`) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Empty string if the key has no password                                                                                                              |

Local `pnpm tauri:build` loads `~/.tauri/agents-manager.key` via `scripts/tauri-build.mjs` (no manual `export` needed).

`tauri-action` signs update bundles and uploads `latest.json` to the GitHub Release when `TAURI_SIGNING_PRIVATE_KEY` is configured. Installed apps check `https://github.com/izhaong/agents-manager/releases/latest/download/latest.json` on startup.

Releases published **before** updater signing (e.g. v0.3.0) do not include `latest.json`; cut a new tag after secrets are set, or upload a manifest manually:

```bash
cd apps/agents-manager-gui
node scripts/generate-latest-json.mjs \
  --version 0.3.1 --tag v0.3.1 \
  --platform darwin-aarch64 \
  --asset agents-manager.app.tar.gz \
  --sig ../../target/release/bundle/macos/agents-manager.app.tar.gz.sig
gh release upload v0.3.1 latest.json \
  ../../target/release/bundle/macos/agents-manager.app.tar.gz \
  ../../target/release/bundle/macos/agents-manager.app.tar.gz.sig --clobber
```

To rebuild an existing tag manually: Actions → **Release** → **Run workflow** → enter `vX.Y.Z`.

## Dual remotes (optional)

If you also push to a private Gitea instance:

```bash
git remote -v
# origin → Gitea, github → GitHub (see README)
git push origin develop && git push github develop
```

Questions? Open a [GitHub Discussion](https://github.com/izhaong/agents-manager/discussions) or an issue.
