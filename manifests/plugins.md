# 官方 / 插件 Skills（不纳入本仓库，仅记录安装来源）

本仓库只维护**自研 workflow Skills**。下列为各 IDE 自带或插件 cache，升级插件时会覆盖，请勿复制进 `~/.agent-manager/skills/`。

## npm / 上游发布

| 来源           | Skill 路径 | 说明 |
| -------------- | ---------- | ---- |
| ant-design-cli | `skills/antd` | 随 `@ant-design/cli` 发布；用 Cursor `@antd` 或插件 cache，勿 vendored 进本仓 |

## Cursor 插件（`~/.cursor/plugins/cache/`）

| 插件            | 用途                                         |
| --------------- | -------------------------------------------- |
| cursor-team-kit | create-rule、create-skill、fix-ci、review 等 |
| harness         | Harness MCP 治理与 skills                    |
| prisma          | Prisma CLI / Client skills                   |
| context7-plugin | 库文档检索                                   |
| cloudflare      | Workers / Wrangler skills                    |
| figma           | Figma MCP skills                             |
| browse          | 浏览器自动化                                 |

## Cursor 内置（`~/.cursor/skills-cursor/`）

automate、canvas、loop、sdk、split-to-prs 等 — 随 Cursor 更新。

## Claude 插件 cache（`~/.claude/plugins/cache/`）

| 插件              | 示例 Skill                        |
| ----------------- | --------------------------------- |
| superpowers       | brainstorming、TDD、writing-plans |
| frontend-design   | UI 生成规范                       |
| claude-code-setup | automation recommender            |

## Codex 系统（`~/.codex/skills/.system/`）

skill-creator、openai-docs 等 — 勿纳入 agent-manager。

## 新机器 checklist

1. 克隆本仓库，`cargo build -p agent-manager-cli --release && ./target/release/agent-manager install`
2. 在 Cursor 扩展市场安装上表所需插件
3. 核对 `~/.config/agent-manager/secrets.env` 后 `agent-manager sync`
