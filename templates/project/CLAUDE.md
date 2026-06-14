# Claude Code 入口（{{PROJECT_NAME}}）

先读 [`AGENTS.md`](./AGENTS.md)。项目规则与 Skills 正文在 **`.cursor/`**（`.claude/` 为 symlink，内容相同）。

## 常用路径

| 用途 | 路径 |
| --- | --- |
| Codex / 项目总入口 | `AGENTS.md` |
| 项目 AI 总览 | `.cursor/README.md` |
| 核心项目规则 | `.cursor/rules/00-project-core.mdc` |
| 规则索引 | `.cursor/rules/README.md` |
| Plan | `.cursor/plans/README.md` |
| 全局通用规则 | `{{AI_CONFIG_REL}}/rules/` |

检索原则：先 `rg` 定位，再打开少量相关文件。
