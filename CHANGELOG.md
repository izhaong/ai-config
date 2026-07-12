# Changelog

All notable changes to this project will be documented in this file.

Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Fixed

- **source-first T001 安全止血**：`retract` / `uninstall` 在缺少 marker、精确 legacy link 或 MCP 具名所有权记录时 fail-closed，不再删除普通平台副本或整份 MCP 配置；`status` / `doctor` / scope 解析不再初始化资产根或回写 Hook。项目 MCP 渲染使用项目 deploy base；doctor 仅报告字面 MCP secret 的计数及遗留 secret 文件元数据，不输出值。
- **install 项目级增量合并**（Gitea #22）：`AI_CONFIG_ROOT=<repo>` 时正确解析 `<repo>/.ai-config` + `~/.ai-config` 合并源并下发到仓库根；hook merge 仅移除 `managedBy: ai-config` 条目；项目作用域跳过覆盖非托管文件。

### Added

- **`install` / `sync --workspace`**（zh-cloud #42）：解析 `.gitmodules`，对父仓与已 checkout 子模块逐个下发；子仓无本地 `.ai-config` 时继承父仓资产源。

## [0.3.3] - 2026-06-26

### Added

- **资产同步 Hooks（第六类）+ 生命周期 TTS**：源资产为 Cursor 格式 `hooks/hooks.json` + 扁平脚本（如 `hooks/lifecycle-tts.sh`）；列表项对应 manifest 中各生命周期下的 hook 对象，**title = command 路径中的文件名**，**description = 脚本开头 `"""..."""`**；唯一性按文件名 + 脚本内容。`install` / `sync` 按脚本 merge 下发四平台（adapter 兼容 Codex / Claude / Hermes）。默认 `lifecycle-tts` TTS 播报；`AI_CONFIG_TTS=0` 可关闭。

### Changed

- **GUI 主题默认跟随系统**：首次打开或未设置时主题为 `system`，随系统深浅色切换。
- **资产下发不再生成 `.ai-config-deploy.json`**：同步状态改由内容比对判定；仍兼容读取历史 marker 与 symlink。

## [0.3.2] - 2026-06-24

### Added

- **GUI 跨项目复制资产**：在 ai-config 源视图勾选资产后，顶栏「复制到项目」可将 skill/rule/agent/command/mcp 从全局或其它项目复制到目标项目 `.ai-config/`（实体副本；目标同名时拒绝）。

### Changed

- **Spec Kit 贴合 Cursor**：执行清单并入 `plan.md ## Todos`，不再维护 `tasks.md`；用户自然语言下发任务，Agent 自动分支与落盘 spec/plan。全局 `spec-kit-sdd-gate` 与仓内 rules 已对齐。

### Fixed

- **跨平台同步冲突**：以左侧当前浏览平台为对比基准；目标平台已有同名不同内容时弹出对比框，展示各平台差异并手动选择覆盖来源。
- **平台 icon 对比基准**：浏览某 IDE 平台时，各平台 icon 状态均相对该平台内容计算（不再默认对照 `~/.ai-config`）。
- **GUI 平台 icon**：`synced`（仅存在于某平台、未进 `~/.ai-config`）与 `linked` 一样显示为**激活**；未进则灰显。
- **doctor**：各平台独立硬拷贝 / 自有 `mcp.json` 与源不一致时**不再**计为 `wrong_type` / `wrong_source`；图标状态即真相。
- **GUI 项目级 MCP**：跨平台同步与状态展示修复；**其它平台 icon 可点击切换**，当前浏览平台 icon **仅展示状态**（`cursor-default`），MCP 收回请点其它平台 icon 或行内删除。

## [0.3.1] - 2026-06-22

### Added

- **GUI 自动更新**：启动时从 GitHub Releases 检测新版本并弹窗提示；设置菜单可手动「检查更新」；安装后自动重启；CI 签名并发布 `latest.json`。
- **GUI 主题**：设置菜单支持深色 / 浅色 / 跟随系统，偏好持久化至 `localStorage`。
- **GUI 注册项目**：项目名可选（默认取目录名）；路径输入框支持系统文件夹选择器。
- **MCP 外部控制**：`ai-config serve` 启动 stdio MCP 服务器（list/show/save/deploy/retract/doctor/status/sync/env），供 Cursor / Claude Code 等 Agent 原生操作资产。
- **Agent Skill**：`.cursor/skills/ai-config-agent/SKILL.md` — 教 Agent 何时用 MCP 或 CLI `--json`。
- **GUI 添加 MCP**：顶栏「+」打开 `AddMcpModal`，粘贴 JSON 写入 `~/.ai-config/mcp.json` 并可选 deploy。

### Changed

- **GUI 样式体系**：迁移至 Tailwind v4 + shadcn/ui；弹窗/抽屉/Toast 使用 `motion` 动画；接入 React Compiler。
- **GUI 列表布局**：顶栏与列表行共用滚动容器与三列网格，操作列右缘对齐；切换资产/平台时列表缓存减少重渲染。
- **GUI 本地打包**：`pnpm tauri:build` 自动加载 `~/.tauri/ai-config.key` 签名 updater 产物。

### Fixed

- **MCP `serve` 启动崩溃**：工具返回 `serde_json::Value` 时 `outputSchema` 不符合 MCP 规范；改为具名类型 + `JsonSchema` 派生。
- **GUI 检查更新**：远程无 `latest.json` 时提示更新清单尚未发布，而非笼统网络错误。

## [0.3.0] - 2026-06-21

### Added

- **Agent Skill**：`.cursor/skills/ai-config-agent/SKILL.md` — 教 Agent 何时用 CLI `--json` 管理资产。
- **GUI 五平台对等**：ai-config 与 4 IDE 并列；平台 icon deploy / retract / import / 跨平台硬拷贝（`deploy_from_platform`）。
- **链接态 `synced`**：外部安装（如 `npx skills`）内容一致但无 marker；点击可覆盖为 `linked`。
- **列表行「更新」**：从 ai-config 平台副本重新 deploy 到各已激活平台。
- **GUI Skill 添加**：列表顶栏「+」；`npx skills add` 拉取到 ai-config 源，IDE 目标则再 `deploy` 下发。
- **GUI Claude Marketplace 导入**：顶栏「购物袋」；多选 Skill 与平台后，先写入 ai-config 源再 deploy 到各 IDE。
- **参考文档**：`docs/reference/vercel-skills-agent-paths.md`、`manifests/vercel-skills-agents.snapshot.json`。
- **AI 协作配置**：`.cursor/`（rules、agents、skills、commands）、`.specify/`（constitution、Spec Kit 模板与脚本）、`specs/`；扩充 `AGENTS.md`、`CLAUDE.md`。
- **Commands 同步**：新增第 5 类资产 `command`（`~/.ai-config/commands/<name>.md`），下发至 **Cursor** 与 **Claude**。
- CLI `ai-config command list|show|reveal`；`sync` / GUI 与 rules 同链路（实体复制 + `.ai-config-deploy.json`）。
- CLI `ai-config mcp migrate-hermes`：将遗留 `~/.hermes/mcp.json` 合并进 `~/.hermes/config.yaml` 的 `mcp_servers`。
- Core `hermes_config` 模块：YAML 读/写/比对、原子写、备份、`normalize_server_for_hermes`、`ensure_external_skills_dir`。

### Changed

- **ai-config 平台 icon 收回**：`retract(..., AiConfig)` 仅删 ai-config 平台目录；源视图浏览时点击 ai-config icon 不收回。
- **GUI 侧边栏可拖拽调节**：项目区高度、资产/平台宽度、侧栏总宽可手动调整，布局持久化至 `localStorage`。
- **Core 收敛**：新增 `asset_scope`、`doctor`、`asset_ops`、`path_independence`、`skills_add`；GUI 经 `command_bridge` 调 core。
- **产品文档**：PRD **v0.5**、DESIGN **v0.2**、ARCHITECTURE 模块图对齐 `materialize` / `asset_ops`。
- **GUI 工程化**：`eslint.config.js`、`vitest` + `canOpenEntry` 单测；`npm run lint|test` 脚本。
- Hermes 整文件 `uninstall` retract 跳过 `config.yaml`（MCP 须 per-server `mcp retract <name> hermes`）。
- 写 `config.yaml` 前自动备份；全文件 YAML 重序列化可能丢失注释（见 `HERMES.md`）。
- CI：`cargo test --test-threads=1`、Ubuntu runner `git config`、clippy `-Dwarnings` 门禁。

### Fixed

- **多平台 Skill 安装**：Marketplace / 批量添加时 IDE 平台改为「先写 ai-config 源 → deploy」。
- **平台视图删除**：外部 / `synced` 的 Hermes（及其它 IDE）skill 可从平台目录删除，无需 ai-config 源。
- **Hermes MCP 路径**：deploy/retract/sync 写入 `~/.hermes/config.yaml` 的 `mcp_servers:`，不再写 `~/.hermes/mcp.json`。
- **Hermes Skills**：项目作用域始终写 `$HOME/.hermes/skills`；`HERMES_SKILLS_DIR` 时写入 `config.yaml` → `skills.external_dirs`。
- **Hermes Rules**：项目级 deploy 目标改为 `<repo>/.cursor/rules/`。
- **Hermes Agents**：关闭 deploy（官方无 `~/.hermes/agents` 静态目录）。

### Removed

- **`global-config/`**：早期仓库内资产目录已废弃；用户全局资产统一在 `~/.ai-config/`。

### Breaking

- 若曾依赖 ai-config 写入的 `~/.hermes/mcp.json`，请运行 `ai-config mcp migrate-hermes` 或手动合并到 `config.yaml`。

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

[0.3.3]: https://github.com/izhaong/ai-config/releases/tag/v0.3.3
[0.3.2]: https://github.com/izhaong/ai-config/releases/tag/v0.3.2
[0.3.1]: https://github.com/izhaong/ai-config/releases/tag/v0.3.1
[0.3.0]: https://github.com/izhaong/ai-config/releases/tag/v0.3.0
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
