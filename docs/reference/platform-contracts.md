# 四平台资产投影契约

> **状态：2026-07-16 已复核。** 本文是 [Spec 008 平台资产契约](../../specs/008-source-first-projection/spec.md#平台资产契约实现与验收的权威边界) 的便捷索引，不是独立需求来源。实现、测试与迁移有冲突时，必须以 Spec 为准；旧 README、PRD、`platform.rs` 和第三方安装器路径只能用于 legacy inventory。

## 不变边界

- Canonical source 只有 `~/.agent-manager/`、workspace layer 与 `<repo>/.agent-manager/`；平台目录绝不自动回流。
- 投影顺序固定为 **Skills → Rules → MCP → Agents → Commands → Hooks**。
- `direct_link` 只用于一项专用路径；聚合配置一律以具名 entry `generated`，保留外部字段。
- `unsupported` 是安全结果，不允许 fallback 到用户全局配置、虚构目录或历史路径。
- planner 只能消费由 `EffectiveAsset` 生成的契约：其中 `source` 保留 global/workspace/project provenance，`deployment_scope` 单独说明写入边界；两者绝不能互相推断。
- 每个 target 都必须声明完整 consumer set。`.agents/skills` 固定由 Cursor、Codex 共同消费；`.cursor/rules/*.mdc` 只在 **project** scope 由 Cursor、Hermes 共同消费，workspace Cursor Rule 不能声称 Hermes 可见。
- Codex workspace/project MCP 需要 `trusted_project`；Codex workspace/project Hook 需要 `trusted_project_with_independent_review`。adapter 仅声明该条件，planner 决定本机 trust/review 证据是否足够。
- `legacy_inventory` 是结构化盘点信息，永远不能回填为普通 target。
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

Cursor/Codex 的 shared skill target consumer set 固定为 `{cursor, codex}`；收回前 planner 必须确认另一消费者没有仍然请求此 target。Codex 的 `.codex/skills/<name>/` 仅供 legacy inventory。

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

Canonical MCP 是 `mcp/servers/<name>.json` 加 secret reference；真实 secret 仅位于 `~/.config/agent-manager/secrets.env`（0600），不得写入 plan、ledger、日志或备份 manifest。

| 平台 | user / project target | 投影 |
| --- | --- | --- |
| Cursor | `.cursor/mcp.json#mcpServers.<name>` | `generated_json` |
| Codex | `.codex/config.toml#mcp_servers.<name>` | `generated_toml` |
| Claude user | `~/.claude.json#mcpServers.<name>` | `generated_json` |
| Claude project | `.mcp.json#mcpServers.<name>` | `generated_json` |
| Hermes user | `~/.hermes/config.yaml#mcp_servers.<name>` | `generated_yaml` |
| Hermes project | — | `unsupported`；绝不改写 global config |

Codex 的 user MCP 不要求 project trust；workspace/project MCP 必须声明 `trusted_project`。`.codex/mcp.json` 只供 legacy inventory，不能作为新 target。

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

`.codex/prompts/<name>.md` 是 deprecated-but-consumed 的 migration inventory；即使发现它，也不得被普通 projection 写入或覆盖。

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

Codex user Hook 无附加 trust；workspace/project Hook 的 binding 与 script 都必须声明 `trusted_project_with_independent_review`。旧 inline `.codex/config.toml` hooks 只供 inventory，新投影固定 `hooks.json`。

## Legacy inventory 状态

| 路径 | 状态 | 普通 projection 行为 |
| --- | --- | --- |
| Cursor `.cursor/skills/<name>/` | alternate still consumed | 只盘点，默认仍投影到 `.agents/skills` |
| Codex `.codex/skills/<name>/`、`.codex/subagents/<name>`、`.codex/mcp.json` | inventory only | 不写入；显式 migration 才可处理 |
| Codex `.codex/prompts/<name>.md` | deprecated but consumed | 只建议迁移为 Skill，不创建/覆盖 |
| Codex `.codex/rules/*.rules` | different semantic asset | 执行策略，不当作 instruction Rule 处理 |
| Codex inline `config.toml` hooks | inventory only | 不写入；新投影使用 `hooks.json` |

## 官方资料与维护

上游链接、复核日期和完整 scope 语义在 [Spec 008](../../specs/008-source-first-projection/spec.md#平台资产契约实现与验收的权威边界) 保持唯一维护。本索引或契约测试需要更新时，必须先更新 Spec，再更新 adapter 和测试；不允许仅修改路径代码。
