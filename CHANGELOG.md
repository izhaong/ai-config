# Changelog

All notable changes to this project will be documented in this file.

Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Removed

- **`global-config/`**：早期仓库内资产目录已废弃；用户全局资产统一在 `~/.ai-config/`。原 vendored `antd` skill 改记于 `manifests/plugins.md`（上游 ant-design-cli）。

### Changed

- **GUI 规范**：组件迁入 `components/`；抽 `utils/canOpenEntry`；批量操作 toast 按成功/失败计数；`cmd_doctor` 接 core 真值；侧边栏改用 `<button>` + a11y。
- **Core 收敛**：新增 `asset_scope`、`doctor`、`asset_ops`；`platform` 统一解析/能力说明；GUI `lib.rs` 单条资产命令经 `command_bridge` 调 core；CLI `lifecycle::run_doctor` 改调 `doctor::compute_report`。
- **GUI 工程化**：`eslint.config.js`、`vitest` + `canOpenEntry` 单测；`npm run lint|test` 脚本。

### Added

- **AI 协作配置**：`.cursor/`（rules、agents、skills、commands）、`.specify/`（constitution、Spec Kit 模板与脚本）、`specs/`；扩充 `AGENTS.md`、`CLAUDE.md`。
- **Commands 同步**：新增第 5 类资产 `command`（`~/.ai-config/commands/<name>.md`），下发至 **Cursor**（`.cursor/commands/`）与 **Claude**（`.claude/commands/`）；Codex / Hermes 不支持。
- CLI `ai-config command list|show|reveal`；`sync` / GUI 与 rules 同链路（实体复制 + `.ai-config-deploy.json`）。
- CLI `ai-config mcp migrate-hermes`：将遗留 `~/.hermes/mcp.json` 合并进 `~/.hermes/config.yaml` 的 `mcp_servers`。
- Core `hermes_config` 模块：YAML 读/写/比对、原子写、备份、`normalize_server_for_hermes`、`ensure_external_skills_dir`。

### Fixed

- **Hermes MCP 路径**：deploy/retract/sync 改为写入官方 `~/.hermes/config.yaml` 的 `mcp_servers:`，不再写 `~/.hermes/mcp.json`（Hermes 不读取该文件）。项目作用域同样写 `$HOME`，不再创建 `<repo>/.hermes/mcp.json`。
- **Hermes Skills**：项目作用域改为始终写 `$HOME/.hermes/skills`；自定义 `HERMES_SKILLS_DIR` 时自动写入 `config.yaml` → `skills.external_dirs`。
- **Hermes Rules**：项目级 deploy 目标改为 `<repo>/.cursor/rules/`（与 Hermes CWD 加载一致）；user-global 不下发。
- **Hermes Agents**：关闭 deploy（官方无 `~/.hermes/agents` 静态目录）。
- **Breaking**：若你曾依赖 ai-config 写入的 `~/.hermes/mcp.json`，请运行 `ai-config mcp migrate-hermes` 或手动合并到 `config.yaml`。请手动 retract 遗留 `~/.hermes/agents` 软链。

### Changed

- Hermes 整文件 `uninstall` retract 跳过 `config.yaml`（MCP 须 per-server `mcp retract <name> hermes`）。
- 写 `config.yaml` 前自动备份；全文件 YAML 重序列化可能丢失注释（见 `HERMES.md`）。

## [0.2.0] - 2026-06-14

### Added

- GUI **项目作用域**：资产从 `<repo>/.ai-config/` 读取，下发到 `<repo>/.cursor` 等平台目录（与全局 `~/.ai-config` 同构）。
- GUI 注册/移除项目改用应用内模态框；注册时自动创建 `.ai-config` 标准目录树。
- GUI 工具栏「刷新」按钮；资产根目录文件监听（含 `mcp.json` 原子写入）。
- CLI `agent` 子命令；core `for_scope` / `ensure_asset_layout` / `link_src_for_create`。
- GUI i18n 与界面优化。

### Changed

- PRD 升至 v0.4（项目作用域、目录同构、实时刷新等决策）。
- `SyncAction::Create` 携带 `src`，install 建链不再反推路径。

### Fixed

- 全局 `install` 在资产根直挂 `skills/` 时不再自环链接。
- CI：`cargo fmt`、clippy 未使用参数、daemon/Windows 交叉编译。
- GUI Agent/Rule 描述解析与 agent 读取路径。

[0.2.0]: https://github.com/izhaong/ai-config/releases/tag/v0.2.0

## [0.1.0] - 2026-06-14

### Added

- GitHub CI: fmt / clippy / test on Ubuntu & macOS; GUI frontend smoke build.
- GitHub Release workflow: tag `vX.Y.Z` builds CLI (Linux/macOS/Windows) + Tauri GUI installers; release notes from `CHANGELOG.md`.
- GitHub open-source docs: README, CONTRIBUTING, issue/PR templates.
- MIT `LICENSE`.

### Changed

- MCP user assets consolidated to a **single** `~/.ai-config/mcp.json` (legacy `mcp/servers/` migrated on install).
- Test MCP fixtures sanitized (placeholders only; no real tokens or private hosts).

### Security

- Documented vulnerability reporting via GitHub Security Advisories.

[0.1.0]: https://github.com/izhaong/ai-config/releases/tag/v0.1.0
