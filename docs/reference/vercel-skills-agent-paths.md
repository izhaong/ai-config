# vercel-labs/skills 路径快照（历史盘点）

> **不是 agent-manager 投影契约。** `vercel-labs/skills` 是第三方安装器；它的 `src/agents.ts` 只能帮助识别历史安装或做 migration inventory，不能决定 agent-manager 的新写入路径。当前唯一需求来源是 [Spec 008](../../specs/008-source-first-projection/spec.md#平台资产契约实现与验收的权威边界)，便捷索引见 [platform-contracts.md](./platform-contracts.md)。

本文件保留旧 `npx skills add` 快照，便于解释旧目录为何出现。不要依据本页修改 `crates/agent-manager-core/src/platform.rs`、新的 projection adapter 或 GUI；上游官方契约变化时，应先更新 Spec 008、契约测试和 `platform-contracts.md`。

## 上游文件

| 文件                                                                                               | 说明                                                        |
| -------------------------------------------------------------------------------------------------- | ----------------------------------------------------------- |
| [`src/agents.ts`](https://github.com/vercel-labs/skills/blob/main/src/agents.ts)                   | 各 agent 的 `skillsDir`（项目）与 `globalSkillsDir`（全局） |
| [`src/add.ts`](https://github.com/vercel-labs/skills/blob/main/src/add.ts)                         | `skills add` 安装：全局/项目、symlink vs copy               |
| [`scripts/sync-agents.ts`](https://github.com/vercel-labs/skills/blob/main/scripts/sync-agents.ts) | 从 `agents.ts` 刷新 README 表格                             |

### 刷新快照（本仓）

```bash
curl -fsSL https://raw.githubusercontent.com/vercel-labs/skills/main/src/agents.ts \
  -o /tmp/vercel-agents.ts
# 人工对照 agents.ts 中 cursor / codex / claude-code / hermes-agent 四段，更新 manifests/vercel-skills-agents.snapshot.json 与下表
```

## agent-manager 关心的 4 平台（vercel `--agent` 键）

| agent-manager `PlatformId` | vercel `--agent` | 项目路径 `skillsDir` | 全局路径 `globalSkillsDir` | 环境变量覆盖                            |
| ---------------------- | ---------------- | -------------------- | -------------------------- | --------------------------------------- |
| `cursor`               | `cursor`         | `.agents/skills/`    | `~/.cursor/skills/`        | —                                       |
| `codex`                | `codex`          | `.agents/skills/`    | `~/.codex/skills/`         | `CODEX_HOME`（默认 `~/.codex`）         |
| `claude`               | `claude-code`    | `.claude/skills/`    | `~/.claude/skills/`        | `CLAUDE_CONFIG_DIR`（默认 `~/.claude`） |
| `hermes`               | `hermes-agent`   | `.hermes/skills/`    | `~/.hermes/skills/`        | `HERMES_HOME`（默认 `~/.hermes`）       |

路径均相对于 **项目根**（`skills add` 的 cwd）或 **用户主目录**（`-g` / `--global`）。

## `npx skills add` 下发逻辑（摘要）

与 agent-manager 的「硬拷贝 + `.agent-manager-deploy.json` marker」不同，vercel CLI 默认行为如下：

1. **作用域**
   - 无 `-g`：写入当前目录下的 `skillsDir`（项目级）。
   - `-g` / `--global`：写入 `globalSkillsDir`（若该 agent 支持；`eve` 等仅项目级为 `N/A`）。

2. **安装模式**（`add.ts`）
   - 默认 **symlink**；`--copy` 强制拷贝。
   - 所选 agent 的 `skillsDir` 全部相同时，无 symlink 意义，**默认 copy**。
   - **Universal agents**（`skillsDir === '.agents/skills'`）：多 agent 共享同一项目目录，安装一次即可（`getUniversalAgents()`）。
   - **Non-universal**：canonical 内容在 `.agents/skills` 时，向各 agent 专有目录 **symlink**（Windows 无 Developer Mode 时可能退化为 copy）。

3. **与 agent-manager 扫描的关系**
   - 经 `npx skills` 装入、无 agent-manager marker 的副本，在 GUI 中为 **`synced`**（内容一致、非本工具下发），点击可覆盖为 **`linked`**。
   - 全局路径与上表一致时，agent-manager 与 vercel CLI 可共存；**项目级**路径见下节差异。

## agent-manager 与 vercel CLI 的路径差异

`platform.rs` 在 **项目作用域**（`deploy_base = 仓库根`）与 vercel 不完全一致：

| 平台   | 作用域 | agent-manager（当前）                          | vercel-labs/skills       |
| ------ | ------ | ------------------------------------------ | ------------------------ |
| Cursor | 全局   | `~/.cursor/skills/`                        | 同左                     |
| Cursor | 项目   | `<repo>/.cursor/skills/`                   | `<repo>/.agents/skills/` |
| Codex  | 全局   | `~/.codex/skills/`                         | `$CODEX_HOME/skills/`    |
| Codex  | 项目   | `<repo>/.codex/skills/`                    | `<repo>/.agents/skills/` |
| Claude | 全局   | `~/.claude/skills/`                        | 同左                     |
| Claude | 项目   | `<repo>/.claude/skills/`                   | 同左                     |
| Hermes | 全局   | `HERMES_SKILLS_DIR` 或 `~/.hermes/skills/` | `$HERMES_HOME/skills/`   |
| Hermes | 项目   | **仍写全局** `~/.hermes/skills/`           | `<repo>/.hermes/skills/` |

**说明**

- Cursor 官方文档同时认可 `~/.cursor/skills` 与项目内 `.cursor/skills`；vercel 对 Cursor/Codex **项目级**统一用 `.agents/skills/`（与 Amp、OpenCode、Gemini CLI 等 universal 组一致）。
- agent-manager 项目级沿用各 IDE 自有目录（`.cursor`、`.codex`、`.claude`），与 vercel **全局**路径对齐；若需与 `npx skills` 项目安装互操作，后续可考虑增加 `.agents/skills` 扫描或双写下发（产品决策，非本文范围）。

## 其它 agent（扩展用）

vercel 在 `agents.ts` 中还定义了 70+ agent（OpenCode、Windsurf、Cline、Gemini CLI 等）。完整列表以 upstream `agents.ts` 为准；本仓仅快照 agent-manager 已支持的 4 平台，见 [`manifests/vercel-skills-agents.snapshot.json`](../../manifests/vercel-skills-agents.snapshot.json)。

## 本仓实现入口

- 平台路径实现：[`crates/agent-manager-core/src/platform.rs`](../../crates/agent-manager-core/src/platform.rs)
- 下发 / 收回 / 跨平台拷贝：[`crates/agent-manager-core/src/asset_ops.rs`](../../crates/agent-manager-core/src/asset_ops.rs)
- 链接状态（`linked` / `synced`）：[`crates/agent-manager-core/src/materialize.rs`](../../crates/agent-manager-core/src/materialize.rs)、[`platform_scan.rs`](../../crates/agent-manager-core/src/platform_scan.rs)
