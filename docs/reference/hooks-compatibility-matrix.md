# Hooks 三端兼容矩阵

本文定义 `agents-manager` 在 Cursor / Claude / Codex 三端的 Hook 下发兼容策略，目标是让同一份 `.agents-manager/hooks.json` 在不同平台可预测执行。

## 统一原则

- 源资产唯一入口：`.agents-manager/hooks.json` + `.agents-manager/hooks/`
- 平台差异收敛在 `agents-manager-core` 的 `hook_adapter`，不在平台配置手工分叉维护
- 脚本输出统一 JSON（建议 `{"continue":true}`），`stderr` 仅用于错误信息

## 生命周期映射

| Canonical 生命周期     | Cursor                 | Codex                          | Claude                         |
| ---------------------- | ---------------------- | ------------------------------ | ------------------------------ |
| `sessionStart`         | `sessionStart`         | `SessionStart`                 | `SessionStart`                 |
| `beforeSubmitPrompt`   | `beforeSubmitPrompt`   | `UserPromptSubmit`             | `UserPromptSubmit`             |
| `beforeShellExecution` | `beforeShellExecution` | `PreToolUse` + `matcher=Bash`  | `PreToolUse` + `matcher=Bash`  |
| `afterShellExecution`  | `afterShellExecution`  | `PostToolUse` + `matcher=Bash` | `PostToolUse` + `matcher=Bash` |
| `preToolUse`           | `preToolUse`           | `PreToolUse`                   | `PreToolUse`                   |
| `postToolUse`          | `postToolUse`          | `PostToolUse`                  | `PostToolUse`                  |
| `postToolUseFailure`   | `postToolUseFailure`   | `PostToolUse`（降级）          | `PostToolUse`（降级）          |
| `subagentStart`        | `subagentStart`        | `SubagentStart`                | `SubagentStart`                |
| `subagentStop`         | `subagentStop`         | `SubagentStop`                 | `SubagentStop`                 |
| `preCompact`           | `preCompact`           | `PreCompact`                   | `PreCompact`                   |
| `postCompact`          | `postCompact`          | `PostCompact`                  | `PostCompact`                  |
| `stop`                 | `stop`                 | `Stop`                         | `Stop`                         |

说明：无法原生一一对应的事件会做“近似映射 + matcher 限定”，以稳定触发优先。

## 路径策略

- Cursor 项目级：`.cursor/hooks/<script>`
- Codex 项目级：`.codex/hooks/<script>`（避免绝对路径绑定机器）
- Claude 项目级：`${CLAUDE_PROJECT_DIR}/.claude/hooks/<script>`（避免 cwd 漂移）
- 全局级（HOME 下部署）保持绝对路径，确保在任意 cwd 下可执行

## 默认 matcher 策略

- Shell 生命周期：`Bash`
- 文件编辑事件：`Edit|Write`
- MCP 事件：`mcp__.*`
- Cursor 的 shell 默认 matcher 维持 `agents-manager\\b`，兼容历史行为

## 回滚策略

当新映射行为不符合预期时，可按以下顺序回滚：

1. 从平台配置删除本次托管条目（保留第三方 hooks）
2. 重新从 `.agents-manager/hooks.json` 生成旧策略
3. 必要时在源 manifest 显式指定 matcher 覆盖默认值
