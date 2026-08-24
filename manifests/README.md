# Manifests 索引

本目录记录 **不纳入 agent-manager 仓库** 的外部 Skills / 插件来源，避免与自研 `~/.agent-manager/skills/` 混淆。

---

## 原则

| 类型        | 存放位置                      | 纳入 agent-manager 仓库 |
| ----------- | ----------------------------- | ------------------- |
| 自研工作流  | `~/.agent-manager/skills/<name>/` | ❌ 否（用户目录）   |
| Cursor 插件 | `~/.cursor/plugins/cache/`    | ❌ 否，仅在此记录   |
| Claude 插件 | `~/.claude/plugins/cache/`    | ❌ 否               |

插件 skill **禁止**复制进 `~/.agent-manager/` 冒充自研；见 [`plugins.md`](./plugins.md)。

---

## 新机器 Checklist

1. 安装 agent-manager，`agent-manager install`
2. 在 `~/.agent-manager/` 维护或恢复自研 skills
3. 配置 `~/.config/agent-manager/secrets.env` 后 `agent-manager sync`

自研 Skills 索引见 `~/.agent-manager/skills/README.md`。

## 上游平台路径

| 文件                                                                                          | 说明                                                                                                               |
| --------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| [vercel-skills-agents.snapshot.json](./vercel-skills-agents.snapshot.json)                    | [vercel-labs/skills](https://github.com/vercel-labs/skills) `src/agents.ts` 中 Cursor/Codex/Claude/Hermes 路径快照 |
| [docs/reference/vercel-skills-agent-paths.md](../docs/reference/vercel-skills-agent-paths.md) | 下发逻辑说明、与 agent-manager `platform.rs` 的差异                                                                    |
