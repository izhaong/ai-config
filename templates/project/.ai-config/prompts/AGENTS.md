# {{PROJECT_NAME}} — Codex 项目入口

本文件是 **Codex** 在本项目的原生入口。先读本文件，再按路由读取项目 `.cursor/` 与全局 **ai-config**。

## 入口顺序

1. 本文件（项目摘要与路由）。
2. [`.cursor/README.md`](./.cursor/README.md) — 项目 AI 配置总览。
3. [`.cursor/rules/00-project-core.mdc`](./.cursor/rules/00-project-core.mdc) — 项目核心约束（`alwaysApply`）。
4. 全局通用规则：`~/.ai-config/rules/`（由 `ai-config sync` 下发到各 IDE）。

## 通用规则（ai-config，四端同源）

跨项目编码与提交准则见 **`~/.ai-config/rules/*.mdc`**（用户全局资产，本机不在 git）。ai-config 工具仓自身规则见 **`ai-config/.cursor/rules/`**。勿在本仓库重复粘贴全文。

| 入口 | 路径 |
| --- | --- |
| Codex | `~/.ai-config/` 经 `ai-config install` 链接到各 IDE |
| Hermes（在 ai-config 目录） | [`ai-config/HERMES.md`](../../ai-config/HERMES.md) |
| Hermes（在本项目） | [`HERMES.md`](./HERMES.md) |

## 项目配置（正文在 `.cursor/`）

| 资源 | 路径 |
| --- | --- |
| 项目规则 | `.cursor/rules/` |
| 项目 Skills | `.cursor/skills/` |
| Plan | `.cursor/plans/` |
| MCP 示例 | `.cursor/mcp.json.example`（真实密钥用 ai-config `secrets.env` 渲染全局 MCP） |

Claude Code / Codex 通过 **`.claude/` → `.cursor/` symlink** 读取同一套 rules/skills，无需双份维护。

## 工作流硬约束（摘要）

- 业务变更走 **Issue → 分支 → 小步提交 → PR**，勿直推 `main` / `develop`（若适用）。
- Commit：中文 Conventional Commits，见 `~/.ai-config/rules/` 中提交规范（或 zh-cloud `.claude/rules/conventional-commits.mdc`）。
- 编码纪律：见 `~/.ai-config/rules/` 或 zh-cloud Karpathy 规则。
- 不输出真实 token、密钥；示例仅用占位符。

## 执行约定

- 搜索优先 `rg`；先定位再读文件，避免批量展开无关目录。
- Agent 不默认 `commit` / `push`，除非用户明确要求。

---
生成于 {{GENERATED_DATE}}
