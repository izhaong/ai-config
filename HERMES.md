# Hermes 入口（ai-config 工具仓）

**Rules 单一源**：`~/.ai-config/rules/*.mdc`（与 Cursor / Codex 同源，经 `ai-config install` 链到各 IDE）。

在 **ai-config 目录**内改工具代码时，业务规则仍可读 `~/.ai-config/rules/`。

| 规则             | 路径（用户目录）                                |
| ---------------- | ----------------------------------------------- |
| 编码行为准则     | `~/.ai-config/rules/karpathy-guidelines.mdc`    |
| Git 提交规范     | `~/.ai-config/rules/git-commit-conventions.mdc` |
| Notes 文档工作流 | `~/.ai-config/rules/notes-wiki-workflow.mdc`    |

**在 zh-cloud 等业务目录**跑 Hermes：读该目录 `AGENTS.md`，其中指向 `~/.ai-config/rules/` 与项目级 `.cursor/rules/`。
