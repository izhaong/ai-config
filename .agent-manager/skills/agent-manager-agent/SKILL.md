---
name: agent-manager-agent
description: 通过 agent-manager CLI 或 MCP 服务器管理 skills/rules/agents/commands/MCP 资产，在多 IDE 平台间同步下发。用户要操作 agent-manager 资产、同步到 Cursor/Codex/Claude/Hermes、或排查链接状态时激活。
---

# agent-manager Agent 外部控制

## 何时使用

- 列出 / 读取 / 编辑 `~/.agent-manager/` 或 `<project>/.agent-manager/` 下的资产
- 下发（deploy）或收回（retract）到 Cursor、Codex、Claude Code、Hermes
- 跑 `doctor` / `status` / `sync` 排查同步问题
- 用户提到「agent-manager MCP」「外部控制 agent-manager」

## 方式一：MCP（推荐，IDE 原生集成）

在 Cursor / Claude Code 等 MCP 配置中加入：

```json
{
  "mcpServers": {
    "agent-manager": {
      "command": "agent-manager",
      "args": ["serve"]
    }
  }
}
```

开发中未安装全局二进制时：

```json
{
  "mcpServers": {
    "agent-manager": {
      "command": "/path/to/agent-manager/target/debug/agent-manager",
      "args": ["serve", "--root", "/path/to/project/.agent-manager"]
    }
  }
}
```

### MCP 工具（9 个）

| 工具                | 用途                                 |
| ------------------- | ------------------------------------ |
| `agent_manager_list`    | 列出全部纳管资产                     |
| `agent_manager_show`    | 读取单条资产正文（kind + name）      |
| `agent_manager_save`    | 保存单条资产正文                     |
| `agent_manager_deploy`  | 下发到平台（kind + name + platform） |
| `agent_manager_retract` | 从平台收回                           |
| `agent_manager_doctor`  | 健康检查                             |
| `agent_manager_status`  | 各平台链接状态                       |
| `agent_manager_sync`    | 批量同步                             |
| `agent_manager_env`     | 资产根扫描摘要                       |

**kind**：`skill` | `rule` | `agent` | `command` | `mcp`  
**platform**：`cursor` | `codex` | `claude` | `hermes` | `agentmanager`

## 方式二：CLI + `--json`

所有子命令支持机器可读输出：

```bash
agent-manager list --json
agent-manager doctor --json
agent-manager status --json
agent-manager sync --json
agent-manager skill show my-skill --json
agent-manager mcp deploy my-server cursor
```

项目作用域：

```bash
agent-manager --root /path/to/repo/.agent-manager list --json
```

## 工作流建议

1. **先诊断**：`agent_manager_doctor` 或 `agent-manager doctor --json`
2. **看状态**：`agent_manager_status`
3. **改源**：`agent_manager_show` → 编辑 → `agent_manager_save`
4. **下发**：`agent_manager_deploy`（单条）或 `agent_manager_sync`（批量）
5. **再验证**：`agent_manager_status` / `agent_manager_doctor`

## 约束

- **不**在输出中暴露 `secrets.env` 明文值
- GUI 给人类；Agent **优先** MCP 或 CLI `--json`
- 项目资产根为 `<repo>/.agent-manager/`，与 `~/.agent-manager/` 结构相同
