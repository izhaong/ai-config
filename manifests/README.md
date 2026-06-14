# Manifests 索引

本目录记录 **不纳入 ai-config 仓库** 的外部 Skills / 插件来源，避免与自研 `~/.ai-config/skills/` 混淆。

---

## 原则

| 类型        | 存放位置                      | 纳入 ai-config 仓库 |
| ----------- | ----------------------------- | ------------------- |
| 自研工作流  | `~/.ai-config/skills/<name>/` | ❌ 否（用户目录）   |
| Cursor 插件 | `~/.cursor/plugins/cache/`    | ❌ 否，仅在此记录   |
| Claude 插件 | `~/.claude/plugins/cache/`    | ❌ 否               |

插件 skill **禁止**复制进 `~/.ai-config/` 冒充自研；见 [`plugins.md`](./plugins.md)。

---

## 新机器 Checklist

1. 安装 ai-config，`ai-config install`
2. 在 `~/.ai-config/` 维护或恢复自研 skills
3. 配置 `~/.config/ai-config/secrets.env` 后 `ai-config sync`

自研 Skills 索引见 `~/.ai-config/skills/README.md`。
