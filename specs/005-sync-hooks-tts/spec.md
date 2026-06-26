# Feature Specification: Hook 第六类资产（逐条源 + 分平台适配）

**Feature Branch**: `005-sync-hooks-tts`  
**Created**: 2026-06-25  
**Updated**: 2026-06-25  
**Status**: Draft  
**Input**: 每条 hook 为独立源资产；可选脚本目录；分平台兼容下发/收回

## 设计决策（已确认方向）

### 物理布局 — 用 `.ai-config/hooks/`，不用仓库根 `hooks/`

与 PRD §1.1 资产根同构一致：

```
~/.ai-config/hooks/<name>/          <repo>/.ai-config/hooks/<name>/
├── HOOK.md                         （元数据：description、enabled_platforms）
├── hook.yaml                       （canonical：events、matcher、script_dir）
└── scripts/                        （默认脚本目录，可在 hook.yaml 覆盖）
    └── lifecycle-tts.sh
```

**不用** `<repo>/hooks/` 作为资产根：会破坏「唯一正文在 `.ai-config/`」约定，也与全局 `~/.ai-config/` 不同构。项目级 hooks 落在 `<repo>/.ai-config/hooks/`。

### 一条 hook = 一项源资产（类 skill 目录）

| 对比     | skill                 | hook                                                |
| -------- | --------------------- | --------------------------------------------------- |
| 一项     | `skills/<name>/`      | `hooks/<name>/`                                     |
| 清单文件 | `SKILL.md`            | `HOOK.md` + `hook.yaml`                             |
| 附属目录 | `scripts/`、`assets/` | `scripts/`（或 `hook.yaml` 指定 `script_dir`）      |
| 下发     | 硬拷贝目录            | **分平台 adapter 合并** + 拷贝脚本到平台 hooks 目录 |

`hook.yaml` 为 **ai-config 唯一 canonical 格式**（事件、matcher、入口脚本相对路径）；下发时由 adapter 转成各平台原生配置，收回时按 `managedBy: ai-config` + hook `name` 摘除条目并删脚本副本。

### 分平台消费（adapter，类 MCP → Hermes）

| 平台       | 下发目标                                                                | 收回                                                          |
| ---------- | ----------------------------------------------------------------------- | ------------------------------------------------------------- |
| **Cursor** | 合并 `hooks.json`；脚本 → `.cursor/hooks/<name>/`                       | 移除托管条目 + 删 `.cursor/hooks/<name>/`                     |
| **Codex**  | 合并 `.codex/hooks.json`（Codex schema）；脚本 → `.codex/hooks/<name>/` | 同上                                                          |
| **Claude** | 合并 `~/.claude/settings.json` 的 `hooks` 段                            | 移除托管条目；脚本 → `.claude/hooks/<name>/`                  |
| **Hermes** | 合并 `~/.hermes/config.yaml` 的 `hooks:`（shell hook）                  | 移除托管段；脚本 → `~/.hermes/agent-hooks/<name>/` 或约定路径 |

事件名映射示例（canonical `after_shell` → 平台原生）：

- Cursor: `afterShellExecution`
- Codex: `PostToolUse`（matcher 限 Bash）或平台文档等价事件
- Claude: `PostToolUse` / 文档等价
- Hermes: `post_tool_call`（YAML）

### 与 v0 整包 `hooks.json` 的关系

当前 `hooks/hooks.json` + 单脚本为 **过渡实现**；迁移为 `hooks/lifecycle-tts/` 单条资产后删除整包 manifest。

## User Scenarios

### US1 - 左侧可见、逐条管理 (P1)

**Given** `~/.ai-config/hooks/lifecycle-tts/`，**When** 打开 GUI Hook 类，**Then** 列表显示 `lifecycle-tts`，可编辑 `hook.yaml` / 脚本，平台 icon 仅展示**支持且已下发**的平台（非 unsupported 灰显）。

### US2 - 分平台 deploy / retract (P1)

**When** 对 `lifecycle-tts` deploy 到 cursor，**Then** Cursor `hooks.json` 合并条目且脚本在 `.cursor/hooks/lifecycle-tts/`；**When** retract，**Then** 仅移除本工具写入的该 hook 条目，不动用户手写 hooks。

### US3 - 生命周期 TTS (P1)

默认资产 `lifecycle-tts`：shell 执行 `ai-config` 相关命令后 TTS 播报；`AI_CONFIG_TTS=0` 静默。

## Requirements

- **FR-001**: `AssetKind::Hook`；`source` 扫描 `hooks/<name>/`（须含 `hook.yaml`）
- **FR-002**: `hook.yaml` 支持 `script_dir`（默认 `scripts`）
- **FR-003**: `hook_deploy` 重构为 per-hook + per-platform adapter（merge / retract）
- **FR-004**: GUI 第六类；平台列按 adapter `supports(hook)` 显示
- **FR-005**: `install` / `sync` / 单条 deploy-retract 走统一 `asset_ops`
- **FR-006**: 托管标记：`managedBy: ai-config` + `hook: <name>`（各平台 manifest 内）

## Success Criteria

- **SC-001**: 四平台 adapter 单测（至少 cursor + codex + claude yaml 片段 + hermes yaml 片段）
- **SC-002**: `lifecycle-tts` 迁移为目录资产；GUI 可见
- **SC-003**: 用户第三方 hooks 在 merge 后仍保留
