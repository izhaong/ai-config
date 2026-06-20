---
description: ai-config 本地安装与同步冒烟、排障
---

## User Input

```text
$ARGUMENTS
```

## 场景

用户资产在 `~/.ai-config/`；工具在本仓构建。

## 常用流程

### 构建与安装

```bash
cd <ai-config-repo-root>
cargo build -p ai-config-cli --release
./target/release/ai-config install
```

### 状态与诊断

```bash
ai-config doctor
ai-config status
ai-config project list    # 若已注册项目
```

### 资产子命令

```bash
ai-config skill list
ai-config rule list
ai-config command list
ai-config mcp list
```

### 同步

```bash
ai-config sync
# 或针对单项目
ai-config sync --project <path>
```

## 排障

| 现象                     | 检查                                                 |
| ------------------------ | ---------------------------------------------------- |
| 平台未链接               | `ai-config status`、项目是否 `ai-config project add` |
| MCP 未生效               | 目标 IDE 的 mcp 路径；`secrets.env` 权限 0600        |
| Command 仅 Cursor/Claude | 设计如此；Codex/Hermes 不支持 command 下发           |
| GUI 打不开资产           | Tauri 日志；`cmd_read_platform_asset` 只读路径       |

## 注意

- 不输出真实 token
- 修改同步逻辑属 core → 走 spec-kit 与 `rust-ai-config-dev`
