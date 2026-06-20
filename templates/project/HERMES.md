# {{PROJECT_NAME}} — Hermes 项目上下文

在 **本项目根目录** 内跑 Hermes 时加载本文件。

## 规则（两层）

| 层级 | 路径 |
| --- | --- |
| 全局通用 | `~/.ai-config/rules/*.mdc` |
| 本项目 | `.cursor/rules/*.mdc`（`.hermes` 安装的全局 rules 与 ai-config 同源） |

## 入口

1. [`AGENTS.md`](./AGENTS.md) — 项目路由摘要
2. [`.cursor/rules/00-project-core.mdc`](./.cursor/rules/00-project-core.mdc) — 项目 `alwaysApply`

## Skills

- 全局：`~/.ai-config/skills/`（`ai-config install` / `ai-config sync` 下发到各 IDE）
- 项目：`.cursor/skills/`
