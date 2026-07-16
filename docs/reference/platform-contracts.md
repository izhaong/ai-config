# 四平台资产投影契约

> **状态：2026-07-16 已复核。** 本文是 [Spec 008 平台资产契约](../../specs/008-source-first-projection/spec.md#平台资产契约实现与验收的权威边界) 的便捷索引，不是独立需求来源。实现、测试与迁移有冲突时，必须以 Spec 为准；旧 README、PRD、`platform.rs` 和第三方安装器路径只能用于 legacy inventory。

## 不变边界

- Canonical source 只有 `~/.ai-config/`、workspace layer 与 `<repo>/.ai-config/`；平台目录绝不自动回流。
- 投影顺序固定为 **Skills → Rules → MCP → Agents → Commands → Hooks**。
- `direct_link` 只用于一项专用路径；聚合配置一律以具名 entry `generated`，保留外部字段。
- `unsupported` 是安全结果，不允许 fallback 到用户全局配置、虚构目录或历史路径。
- 真实 HOME 仅在全部本地验证后执行只读 inventory；写入只能是用户再次确认的一项无 secret、无 conflict Skill canary。
- workspace 是独立 deploy root：有 project target 的 Cursor、Codex、Claude 使用 `<workspace>` 替换 `<repo>`；Hermes workspace skill/MCP/Hook 均 `unsupported`，绝不回退到用户全局配置。

## Skills

| 平台 | user / project target | 投影 |
| --- | --- | --- |
| Cursor、Codex | `.agents/skills/<name>/`（同一物理 target） | `direct_link`；显示 shared-target 可见性 |
| Claude | `.claude/skills/<name>/` | `direct_link` |
| Hermes user | `~/.hermes/config.yaml#skills.external_dirs` | `external_directory`，引用 canonical skills 目录 |
| Hermes project | — | `unsupported` |

Cursor 的 `.cursor/skills/` 只是兼容/盘点位置，不能成为默认双投影目标。

## Rules

| 平台 | user / project target | 投影 |
| --- | --- | --- |
| Cursor user | — | `unsupported`，没有稳定文件 target |
| Cursor project | `.cursor/rules/<name>.mdc` | `direct_link` |
| Codex | — | instruction rules `unsupported`；项目指导走 root `AGENTS.md`，绝不写 `.codex/rules` |
| Claude | `.claude/rules/<name>.md` | `generated_markdown` |
| Hermes project | 同 Cursor project physical target | shared `direct_link` |
| Hermes user | — | `unsupported`，绝不写 `SOUL.md` |

## MCP

Canonical MCP 是 `mcp/servers/<name>.json` 加 secret reference；真实 secret 仅位于 `~/.config/ai-config/secrets.env`（0600），不得写入 plan、ledger、日志或备份 manifest。

| 平台 | user / project target | 投影 |
| --- | --- | --- |
| Cursor | `.cursor/mcp.json#mcpServers.<name>` | `generated_json` |
| Codex | `.codex/config.toml#mcp_servers.<name>` | `generated_toml` |
| Claude user | `~/.claude.json#mcpServers.<name>` | `generated_json` |
| Claude project | `.mcp.json#mcpServers.<name>` | `generated_json` |
| Hermes user | `~/.hermes/config.yaml#mcp_servers.<name>` | `generated_yaml` |
| Hermes project | — | `unsupported`；绝不改写 global config |

## Agents

| 平台 | user / project target | 投影 |
| --- | --- | --- |
| Cursor | `.cursor/agents/<name>.md` | `generated_markdown` |
| Codex | `.codex/agents/<name>.toml` | `generated_toml` |
| Claude | `.claude/agents/<name>.md` | `generated_markdown` |
| Hermes | — | `unsupported`，没有静态 agent directory |

`subagents` 是 legacy discovery 名称，不能作为新的 Codex/Claude 投影目录。

## Commands

| 平台 | user / project target | 投影 |
| --- | --- | --- |
| Cursor | `.cursor/commands/<name>.md` | `direct_link` |
| Codex | — | `unsupported`；`.codex/prompts/<name>.md` 仅供 migration inventory，禁止 `.codex/commands` |
| Claude | `.claude/commands/<name>.md` | legacy-compatible `direct_link`；新建时优先推荐 Skill |
| Hermes | — | `unsupported`，建议转换为 Skill |

## Hooks

一个 Hook 必须同时拥有 canonical `hooks.json` 的具名 binding 和 `hooks/<script-or-bundle>` 的脚本单元；不能只生成一半。

| 平台 | binding target | script target | 投影 |
| --- | --- | --- | --- |
| Cursor | `.cursor/hooks.json#hooks.<name>` | `.cursor/hooks/<name>` | binding `generated_json` + script `direct_link` |
| Codex | `.codex/hooks.json#hooks.<name>` | `.codex/hooks/<name>` | binding `generated_json` + script `direct_link`；不写 inline TOML hooks |
| Claude | `.claude/settings.json#hooks.<name>` | `.claude/hooks/<name>` | binding `generated_json` + script `direct_link`，保留其他 settings |
| Hermes user | `~/.hermes/config.yaml#hooks.<name>` | `~/.hermes/hooks/<name>` | binding `generated_yaml` + script `direct_link` |
| Hermes project | — | — | `unsupported`；不得写 global config |

事件、matcher、输入输出或阻断语义无法无损映射时，binding 必须为 `unsupported`。

## 官方资料与维护

上游链接、复核日期和完整 scope 语义在 [Spec 008](../../specs/008-source-first-projection/spec.md#平台资产契约实现与验收的权威边界) 保持唯一维护。本索引或契约测试需要更新时，必须先更新 Spec，再更新 adapter 和测试；不允许仅修改路径代码。
