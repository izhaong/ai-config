# Skills 索引

本目录为 **统一 Skills 资产库**（`global-config/skills/<name>/SKILL.md`）。`ai-config install` 后 symlink 到 `~/.cursor/skills`、`~/.claude/skills`、`~/.codex/skills` 等。

**插件自带 skill 不入库**，见 [`../../manifests/plugins.md`](../../manifests/plugins.md)。

---

## 全量索引

| Skill | 作用 | 何时用 |
| ----- | ---- | ------ |
| [`antd`](./antd/) | 通过 `@ant-design/cli` 离线查询 antd 组件 API、demo、token、迁移与项目用法分析 | 写 antd 组件、调试 antd、版本迁移、`import from 'antd'` |

---

## 按场景分组

### 前端 / UI

| Skill | 作用 | 何时用 |
| ----- | ---- | ------ |
| [`antd`](./antd/) | Ant Design CLI：组件知识库、lint、迁移、bug 上报 | antd 相关开发与排障 |

---

## 来源说明

| 来源 | 说明 |
| ---- | ---- |
| [ant-design/ant-design-cli](https://github.com/ant-design/ant-design-cli) | `skills/antd/SKILL.md`（`antd` skill，随 `@ant-design/cli` 发布） |
| `ai-config/global-config/skills/` | 自研 zh-cloud 工作流、文档归档（其它 skill 待同步入库） |

---

## 维护

1. 新建或更新 `global-config/skills/<kebab-name>/SKILL.md`
2. `ai-config install`（或 GUI 同步）
3. 在本 README **全量索引**与对应分组表中补充一行

**调用**：Cursor 用 `@antd`；无 symlink 时：

```text
请阅读 ~/Code/zh-cloud/ai-config/global-config/skills/antd/SKILL.md
```
