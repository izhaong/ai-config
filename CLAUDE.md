# Claude Code 入口（agent-manager）

与 Codex 共用 **`AGENTS.md`** 路由；本文件为 Claude Code 原生入口别名。

## 必读

1. [AGENTS.md](./AGENTS.md)
2. `.agent-manager/rules/00-agent-manager-core.mdc`
3. `.specify/memory/constitution.md`

## 工作方式

用户**直接说任务**。Agent 自动：切分支 → `spec.md` → `plan.md`（含 Todos）→ 实现 → 验证。

- Rust：agent `rust-agent-manager-dev`
- GUI：agent `gui-agent-manager-dev`
- 非平凡交付：skill `agent-manager-delivery`（内化 Spec Kit，勿让用户跑 `/speckit.*`）

Commands 与 Skills 位于 `.agent-manager/commands/`、`.agent-manager/skills/`；Claude 的运行时目录由 source-first projection 本地生成，不再追踪副本。
