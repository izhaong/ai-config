---
description: agent-manager 本地安装与同步冒烟、排障
---

## User Input

```text
$ARGUMENTS
```

## 场景

用户资产在 `~/.agent-manager/`；工具在本仓构建。

## 常用流程

### 构建与安装

```bash
cd <agent-manager-repo-root>
cargo build -p agent-manager-cli --release
./target/release/agent-manager install
```

### 状态与诊断

```bash
agent-manager doctor
agent-manager status
agent-manager project list    # 若已注册项目
```

### 资产子命令

```bash
agent-manager skill list
agent-manager rule list
agent-manager command list
agent-manager mcp list
```

### 同步

```bash
agent-manager sync
# 或针对单项目
agent-manager sync --project <path>
```

## 排障

| 现象                     | 检查                                                 |
| ------------------------ | ---------------------------------------------------- |
| 平台未链接               | `agent-manager status`、项目是否 `agent-manager project add` |
| MCP 未生效               | 目标 IDE 的 mcp 路径；`secrets.env` 权限 0600        |
| Command 仅 Cursor/Claude | 设计如此；Codex/Hermes 不支持 command 下发           |
| GUI 打不开资产           | Tauri 日志；`cmd_read_platform_asset` 只读路径       |

## 注意

- 不输出真实 token
- 修改同步逻辑属 core → 走 spec-kit 与 `rust-agent-manager-dev`
