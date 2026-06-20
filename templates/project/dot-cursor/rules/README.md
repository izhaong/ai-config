# 项目规则（{{PROJECT_NAME}}）

正文目录：**`.cursor/rules/`**（`.claude/rules` 为 symlink，内容相同）。

## 始终生效

| 文件 | 作用 |
| --- | --- |
| `00-project-core.mdc` | 本项目 `alwaysApply`：仓库事实、Git/PR、Plan、敏感信息 |

## 全局规则（不在此目录）

见 **`~/.ai-config/rules/README.md`**（若已维护）或 zh-cloud `.claude/rules/`：提交规范、Karpathy 准则、Spec Kit 门禁等。

## 新增规则

1. 在本目录新建 `*.mdc`，写清 `description` 与是否需要 `alwaysApply`。
2. 同步更新本 README 表格。
3. 若规则适用于全公司所有项目，应放到 `ai-config/rules/` 而非此处。
