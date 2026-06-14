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

## Dual remotes (optional)

If you also push to a private Gitea instance:

```bash
git remote -v
# origin → Gitea, github → GitHub (see README)
git push origin develop && git push github develop
```

Questions? Open a [GitHub Discussion](https://github.com/izhaong/ai-config/discussions) or an issue.
