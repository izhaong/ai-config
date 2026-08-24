---
description: agents-manager 本地安装与同步冒烟、排障
---

## User Input

```text
$ARGUMENTS
```

## 场景

用户资产在 `~/.agents-manager/`；工具在本仓构建。

## 常用流程

### 构建与安装

```bash
cd <agents-manager-repo-root>
cargo build -p agents-manager-cli --release
./target/release/agents-manager install
```

### 状态与诊断

```bash
agents-manager doctor
agents-manager status
agents-manager project list    # 若已注册项目
```

### 资产子命令

```bash
agents-manager skill list
agents-manager rule list
agents-manager command list
agents-manager mcp list
```

### 同步

```bash
agents-manager sync
# 或针对单项目
agents-manager sync --project <path>
```

## 排障

| 现象                     | 检查                                                 |
| ------------------------ | ---------------------------------------------------- |
| 平台未链接               | `agents-manager status`、项目是否 `agents-manager project add` |
| MCP 未生效               | 目标 IDE 的 mcp 路径；`secrets.env` 权限 0600        |
| Command 仅 Cursor/Claude | 设计如此；Codex/Hermes 不支持 command 下发           |
| GUI 打不开资产           | Tauri 日志；`cmd_read_platform_asset` 只读路径       |

## 注意

- 不输出真实 token
- 修改同步逻辑属 core → 走 spec-kit 与 `rust-agents-manager-dev`
