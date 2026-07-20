# Hermes 入口（ai-config 工具仓）

**Rules 单一源**：本仓使用 `.ai-config/rules/*.mdc`；用户级资产使用 `~/.ai-config/rules/*.mdc`。平台目录由 source-first projection 生成，绝不作为正文副本提交。

在 **ai-config 目录**内改工具代码时，先读项目 `.ai-config/rules/`。

| 规则             | 路径（用户目录）                                |
| ---------------- | ----------------------------------------------- |
| 编码行为准则     | `~/.ai-config/rules/karpathy-guidelines.mdc`    |
| Git 提交规范     | `~/.ai-config/rules/git-commit-conventions.mdc` |
| Notes 文档工作流 | `~/.ai-config/rules/notes-wiki-workflow.mdc`    |

**在 zh-cloud 等业务目录**跑 Hermes：读该目录 `AGENTS.md`，其中指向 `~/.ai-config/rules/` 与项目级 `.cursor/rules/`。

## Skills / Rules / Agents（与官方对齐）

| 资产       | ai-config 下发目标                                        | Hermes 官方消费                                                     | 作用域                       |
| ---------- | --------------------------------------------------------- | ------------------------------------------------------------------- | ---------------------------- |
| **Skills** | 始终 `~/.hermes/skills/<name>/`（或 `HERMES_SKILLS_DIR`） | `$HERMES_HOME/skills` + 可选 `config.yaml` → `skills.external_dirs` | 全局；项目作用域也写 `$HOME` |
| **Rules**  | `<repo>/.cursor/rules/<name>.mdc`                         | CWD 下 `.cursor/rules/*.mdc`（优先级低于 `AGENTS.md` 等）           | **仅已注册项目**             |
| **Agents** | **不下发**                                                | 项目 `AGENTS.md` / 运行时 `delegate_task`                           | —                            |
| **MCP**    | `~/.hermes/config.yaml` → `mcp_servers`                   | 同上                                                                | 全局                         |

自定义 `HERMES_SKILLS_DIR` 时，ai-config 会在 deploy 后把该路径写入 `skills.external_dirs`，否则 Hermes 不会扫描。

遗留的 `~/.hermes/agents/` 或 `<repo>/.hermes/agents/` 软链可手动 `retract` 删除；新版本不再创建。

## MCP 下发路径

Hermes **只读** `~/.hermes/config.yaml` 的 `mcp_servers:` 段（YAML），**不**读取 `~/.hermes/mcp.json`。

| 操作         | ai-config 命令                        | 目标                                                             |
| ------------ | ------------------------------------- | ---------------------------------------------------------------- |
| 单条 deploy  | `ai-config mcp deploy <name> hermes`  | merge `mcp_servers.<name>`                                       |
| 单条 retract | `ai-config mcp retract <name> hermes` | 移除 `mcp_servers.<name>`                                        |
| 遗留迁移     | `ai-config mcp migrate-hermes`        | 将旧 `~/.hermes/mcp.json` 合并进 config.yaml 后 rename 为 `.bak` |

写入前会备份为 `config.yaml.ai-config.bak.<unix_ts>`。YAML 全文件重序列化**可能丢失注释** — 请依赖备份回滚。

下发后在新 Hermes 会话中生效，或在已有会话中发送 `/reload-mcp` 热加载。
