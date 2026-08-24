# {{PROJECT_NAME}} — 项目 AI 配置

本目录为 **Cursor / Claude / Codex / Hermes** 共用的项目级配置**唯一正文**。

- **`.claude/`** 下 `rules`、`skills`、`plans`、`agents` 等通过 symlink 指向此处，请勿在 `.claude/` 单独改副本。
- **全局** 通用规则在 `~/.agent-manager/rules/`；此处只放**本项目特有**约定。

## 目录

| 路径 | 用途 |
| --- | --- |
| `rules/` | 项目 `.mdc` 规则 |
| `plans/` | Cursor Plan 落盘（`*.plan.md`） |
| `skills/` | 仅本项目的工作流 Skill |
| `agents/` | 可选：子 Agent 角色说明 |
| `mcp.json.example` | MCP 占位示例（密钥用 agent-manager 全局渲染） |

## 维护

1. 改项目规则 → 只编辑 `rules/*.mdc`
2. 改通用提交/编码准则 → 编辑 `~/.agent-manager/rules/`，再 `agent-manager sync`
3. 新增全局 skill → `~/.agent-manager/skills/`，再 `agent-manager sync`
