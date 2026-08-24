# Feature Specification: install 项目级增量合并

**Feature Branch**: `006-install-project-merge`  
**Created**: 2026-06-26  
**Status**: Complete  
**Input**: Gitea #22 — `AGENT_MANAGER_ROOT=<repo> agents-manager install` 破坏项目 MCP / hooks / 命令

## User Scenarios

### US1 - 项目 install 不破坏手写配置 (P1)

**Given** 父仓已有 `.cursor/hooks.json`、`.claude/settings.json`（含 `mcpServers`）与完整 MCP 列表，**When** `AGENT_MANAGER_ROOT=<repo> agents-manager install`，**Then** 仅合并 agents-manager 托管条目，用户 hooks / MCP / 命令不被整文件覆盖或误删。

### US2 - 正确解析项目作用域 (P1)

**Given** `AGENT_MANAGER_ROOT` 指向仓库根（非 `~/.agents-manager`），**When** install，**Then** 资产源 = `<repo>/.agents-manager` 合并 `~/.agents-manager`，下发目标 = `<repo>/.cursor` 等（非 `$HOME`）。

### US3 - Claude 共享 `.cursor/hooks/` (P2)

**Given** 项目 `settings.json` 已引用 `.cursor/hooks/*.py`，**When** install 下发 hook，**Then** 不创建冲突的 `.claude/hooks/` 注册路径。

## Requirements

- **FR-001**: `resolve_sync_roots` 区分全局 / 项目 install 的 repo、asset、global default
- **FR-002**: `load_context` 使用 `scan_with_override(asset, global)` + 正确 `compute_for_project`
- **FR-003**: hook merge 仅移除 `managedBy: agents-manager` 条目，禁止子串误伤 `.cursor/hooks/`
- **FR-004**: 项目作用域对非托管 skill/rule/command/agent 目标文件跳过覆盖
- **FR-005**: `merge_claude_settings` 显式保留顶层 `mcpServers`
- **FR-006**: 项目 Claude hook 命令路径检测共享 `.cursor/hooks/`

## Success Criteria

- **SC-001**: 单测覆盖 hook 误删防护、settings mcpServers 保留、项目根解析
- **SC-002**: `cargo test -p agents-manager-core -p agents-manager-cli` 通过
