---
name: ai-config-agent
description: 通过 ai-config CLI 或 MCP 服务器管理 skills/rules/agents/commands/MCP 资产，在多 IDE 平台间同步下发。用户要操作 ai-config 资产、同步到 Cursor/Codex/Claude/Hermes、或排查链接状态时激活。
---

# ai-config Agent 外部控制

## 何时使用

- 列出 / 读取 / 编辑 `~/.ai-config/` 或 `<project>/.ai-config/` 下的资产
- 下发（deploy）或收回（retract）到 Cursor、Codex、Claude Code、Hermes
- 跑 `doctor` / `status` / `sync` 排查同步问题
- 用户提到「ai-config MCP」「外部控制 ai-config」

## 方式一：MCP（推荐，IDE 原生集成）

在 Cursor / Claude Code 等 MCP 配置中加入：

```json
{
  "mcpServers": {
    "ai-config": {
      "command": "ai-config",
      "args": ["serve"]
    }
  }
}
```

开发中未安装全局二进制时：

```json
{
  "mcpServers": {
    "ai-config": {
      "command": "/path/to/ai-config/target/debug/ai-config",
      "args": ["serve", "--root", "/path/to/project/.ai-config"]
    }
  }
}
```

### MCP 工具（9 个）

| 工具                | 用途                                 |
| ------------------- | ------------------------------------ |
| `ai_config_list`    | 列出全部纳管资产                     |
| `ai_config_show`    | 读取单条资产正文（kind + name）      |
| `ai_config_save`    | 保存单条资产正文                     |
| `ai_config_deploy`  | 下发到平台（kind + name + platform） |
| `ai_config_retract` | 从平台收回                           |
| `ai_config_doctor`  | 健康检查                             |
| `ai_config_status`  | 各平台链接状态                       |
| `ai_config_sync`    | 批量同步                             |
| `ai_config_env`     | 资产根扫描摘要                       |

**kind**：`skill` | `rule` | `agent` | `command` | `mcp`  
**platform**：`cursor` | `codex` | `claude` | `hermes` | `aiconfig`

## 方式二：CLI + `--json`

所有子命令支持机器可读输出：

```bash
ai-config list --json
ai-config doctor --json
ai-config status --json
ai-config sync --json
ai-config skill show my-skill --json
ai-config mcp deploy my-server cursor
```

项目作用域：

```bash
ai-config --root /path/to/repo/.ai-config list --json
```

## 工作流建议

1. **先诊断**：`ai_config_doctor` 或 `ai-config doctor --json`
2. **看状态**：`ai_config_status`
3. **改源**：`ai_config_show` → 编辑 → `ai_config_save`
4. **下发**：`ai_config_deploy`（单条）或 `ai_config_sync`（批量）
5. **再验证**：`ai_config_status` / `ai_config_doctor`

## 约束

- **不**在输出中暴露 `secrets.env` 明文值
- GUI 给人类；Agent **优先** MCP 或 CLI `--json`
- 项目资产根为 `<repo>/.ai-config/`，与 `~/.ai-config/` 结构相同
