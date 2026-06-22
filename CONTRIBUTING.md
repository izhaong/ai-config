# Contributing to ai-config

Thank you for improving ai-config. This project uses **Git Flow** with **`develop`** as the integration branch.

## Getting started

1. Fork [izhaong/ai-config](https://github.com/izhaong/ai-config) on GitHub (or clone from your Gitea fork if you use the dual-remote setup).
2. Create a branch from `develop`: `feat/123-short-name` or `fix/123-short-name`.
3. Install [Rust 1.78+](https://rustup.rs/) and run:

   ```bash
   cargo fmt
   cargo test -p ai-config-core -p ai-config-cli
   ```

4. Open a Pull Request **into `develop`**.

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

- Business logic belongs in `crates/ai-config-core`; CLI/GUI call core, do not duplicate sync rules.
- Do **not** commit real tokens, passwords, or private hostnames — use placeholders in tests and docs.
- User assets live under `~/.ai-config/`; this repo ships code and templates only.

## Pull requests

- Link related issues (`Fixes #n` or `Refs #n`).
- Keep PRs focused; prefer several small PRs over one large diff.
- Ensure CI passes (fmt, clippy, tests).

## Releases (GitHub)

Version numbers must stay in sync across:

- `Cargo.toml` (`[workspace.package].version`)
- `apps/ai-config-gui/package.json`
- `apps/ai-config-gui/tauri.conf.json`（**tauri-action / GUI 安装包版本**）
- `apps/ai-config-gui/src-tauri/tauri.conf.json`

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

5. GitHub Actions **Release** workflow validates the tag, builds CLI + Tauri GUI artifacts, and publishes [GitHub Releases](https://github.com/izhaong/ai-config/releases) with CHANGELOG notes.

### GUI auto-update signing

Desktop builds use [Tauri updater](https://v2.tauri.app/plugin/updater/). The **public key** is in `apps/ai-config-gui/tauri.conf.json`; maintainers must add repository secrets before release:

| Secret                               | Value                                                                                                                                                |
| ------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `TAURI_SIGNING_PRIVATE_KEY`          | Contents of `~/.tauri/ai-config.key` (generate: `cd apps/ai-config-gui && CI=true npm run tauri signer generate -- -w ~/.tauri/ai-config.key -p ""`) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Empty string if the key has no password                                                                                                              |

`tauri-action` signs update bundles and uploads `latest.json` to the GitHub Release. Installed apps check `https://github.com/izhaong/ai-config/releases/latest/download/latest.json` on startup.

To rebuild an existing tag manually: Actions → **Release** → **Run workflow** → enter `vX.Y.Z`.

## Dual remotes (optional)

If you also push to a private Gitea instance:

```bash
git remote -v
# origin → Gitea, github → GitHub (see README)
git push origin develop && git push github develop
```

Questions? Open a [GitHub Discussion](https://github.com/izhaong/ai-config/discussions) or an issue.
