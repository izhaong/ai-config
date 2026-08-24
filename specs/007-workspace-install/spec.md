# Feature Specification: 聚合仓 Workspace Install

**Feature Branch**: `007-workspace-install`  
**Created**: 2026-06-26  
**Status**: Complete  
**Input**: zh-cloud #42 — 子仓 `.codex` 覆盖不足、bootstrap 与 agent-manager 分拆

## 背景

`scripts/bootstrap-repo-agent-manager.sh` 为每个子仓**手工列举** rules/skills/commands/agents/hooks，与 agent-manager 的 `compute_for_project` 模型并行。用户级 `agent-manager sync` 已健康，但 repo 级需重复跑 bootstrap。

## User Scenarios

### US1 - 一次 install 覆盖父仓 + 子模块 (P1)

**Given** 聚合仓根含 `.gitmodules` 与 `.agent-manager/`，**When** `agent-manager install --workspace <repo>`，**Then** 对父仓及每个已 checkout 的子模块路径执行项目级 install（增量合并，不覆盖非托管文件）。

### US2 - 子仓继承父仓资产源 (P1)

**Given** 子模块无本地 `.agent-manager/`（或为空），**When** workspace install，**Then** 扫描源回落到 `<workspace>/.agent-manager` 合并 `~/.agent-manager`，而非仅全局。

### US3 - bootstrap 渐进迁移 (P2)

**Given** bootstrap 仍负责 per-repo 精选列表与 COMMANDS.md，**When** workspace install 落地，**Then** bootstrap 可瘦身为调用 `agent-manager install --workspace` + 保留 zh-cloud 专有模板（hooks 变体、cursorignore）。

## Requirements

- **FR-001**: 解析 `.gitmodules` 的 `path` 列表（跳过未 checkout 目录）
- **FR-002**: `resolve_member_sync_roots(member, workspace)` — 子仓无资产时继承 workspace `.agent-manager`
- **FR-003**: CLI `install` / `sync` 支持 `--workspace`（隐含 `AI_CONFIG_ROOT=<workspace>`）
- **FR-004**: JSON 报告含 `members[]` 与每仓 outcomes
- **FR-005**: 不替代 bootstrap 的 per-repo 白名单（后续 `repo-manifest.yaml`）

## 与 bootstrap 差异（文档化）

| 能力 | bootstrap | workspace install |
| ---- | --------- | ----------------- |
| 资产范围 | 每仓手写白名单 | 父 `.agent-manager` ∪ `~/.agent-manager` 全量 |
| hooks 模板 | `hooks.repo.json` / parent 变体 | agent-manager hook merge |
| Codex 仅 3 仓 | 手动 `sync_codex_hooks` | 凡有 `.codex/` 即 merge |
| COMMANDS.md | 生成 | 不生成 |

## Success Criteria

- **SC-001**: 单测 — gitmodules 解析、资产继承逻辑
- **SC-002**: `cargo test -p agent-manager-core -p agent-manager-cli`
- **SC-003**: 对 zh-cloud 父仓 + 1 子仓 temp fixture 集成测 install --workspace
