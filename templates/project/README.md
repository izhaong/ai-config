# 项目级 AI 配置模板

由 `agent-manager` CLI 或手动复制渲染到目标项目根目录（`scripts/bootstrap-project.sh` 已废弃）。

## 生成后的目录（四端消费）

```text
<project>/
├── AGENTS.md              # Codex 入口
├── CLAUDE.md              # Claude Code 入口
├── HERMES.md              # Hermes 在项目目录内时的入口
├── .cursor/               # ★ 唯一正文（rules / skills / plans / agents）
│   ├── rules/
│   ├── plans/
│   ├── skills/
│   └── mcp.json.example
└── .claude/               # symlink → .cursor 对应子目录（避免双份维护）
    ├── rules -> ../.cursor/rules
    ├── skills -> ../.cursor/skills
    └── ...
```

## 与 agent-manager 的关系

| 层级 | 位置 | 内容 |
| --- | --- | --- |
| 全局 | `~/.agent-manager/{skills,rules,commands,agents,mcp.json}` | 跨项目通用（本机，不在 git） |
| 项目 | `<project>/.agent-manager/` 或 `<project>/.cursor/` | 仅本项目的工作流、技术栈、仓库约定 |
| 本机 | `~/.config/agent-manager/secrets.env` | MCP 密钥 |

项目规则**不要**复制 agent-manager 里的通用 `.mdc`；在 `00-project-core.mdc` 里写指针即可。

## 占位符

模板内 `{{NAME}}` 由 bootstrap 脚本替换：

| 占位符 | 含义 |
| --- | --- |
| `{{PROJECT_NAME}}` | 显示名 |
| `{{PROJECT_SLUG}}` | 目录名 / slug |
| `{{AGENT_MANAGER_REL}}` | 从项目根到 agent-manager 的相对路径 |
| `{{GENERATED_DATE}}` | 生成日期 ISO |
