# Implementation Plan: Codex MCP source-first 导入与等价采纳

**Branch**: `fix/gui-unmanaged-copy-actions` | **Date**: 2026-07-23 | **Spec**: [spec.md](./spec.md)

> **For agentic workers:** REQUIRED SUB-SKILL: Use `executing-plans` to implement this plan task-by-task. Steps use checkbox syntax for tracking.

## Summary

在现有 reviewed import、migration inventory、projection ledger 和 GUI command bridge 上补齐 Codex MCP。Core 负责 TOML named-entry 提取、canonical normalization、secret preflight 和等价采纳；CLI/GUI 只传请求、展示 plan 并提交 digest/action IDs。目标配置始终保留为平台文件，导入与 adopt 都不反向覆盖它。

## 背景与根因

- `projection::migration_action::import_source_location` 对 MCP 硬编码仅允许 Cursor。
- `platform_mcp_entry_from_bytes` 只解析 JSON `mcpServers`，无法读取 Codex TOML。
- migration inventory 未把 Codex named MCP entry 与 canonical source 做字段级 normalized comparison，因此没有 `AdoptEquivalent`。
- GUI 已有通用 import command bridge，但 core 返回 NotImplemented。
- Tauri `Cargo.toml` 仍为 0.3.3，导致已安装 CLI/GUI 能力与版本认知分裂。

```text
.codex/config.toml#mcp_servers.<name>
  -> reviewed ImportPlan (paths/fingerprints/key names only)
  -> .agents-manager/mcp/servers/<name>.json
  -> normalized equivalence check
  -> explicit AdoptEquivalent
  -> per-entry projection ledger (target bytes unchanged)
```

## Technical Context

| 项 | 值 |
| --- | --- |
| Language | Rust 2021 + React/TypeScript |
| 主要 Crate | `agents-manager-core` / `agents-manager-cli` / `agents-manager-gui` |
| 依赖模块 | `projection::migration_action`, `projection::migration`, `projection::mcp::codex_toml`, ledger/store |
| 测试 | `migration_import`, `projection_migration`, GUI command bridge/Vitest |
| 平台矩阵 | Codex project scope；Cursor 回归不退化 |

## Constitution Check

- [x] 业务逻辑仅在 `agents-manager-core`
- [x] CLI/GUI 薄封装，无重复解析
- [x] 平台差异收敛在 projection MCP adapter
- [x] plan/apply digest-bound、foreign fail-closed
- [x] 不提交真实 secret

## 影响面与接口

| 文件 | 职责 |
| --- | --- |
| `crates/agents-manager-core/src/projection/migration_action.rs` | Codex import source path、TOML entry normalization、secret preflight |
| `crates/agents-manager-core/src/projection/migration.rs` | Codex MCP inventory/equivalence/adopt action |
| `crates/agents-manager-core/tests/migration_import.rs` | import RED/GREEN 与安全边界 |
| `crates/agents-manager-cli/tests/projection_migration.rs` | CLI plan/apply/rollback、零目标写入 |
| `apps/agents-manager-gui/src/command_bridge.rs` | 复用 core 的 GUI import/adopt 测试 |
| `apps/agents-manager-gui/src/**` | Import/Adopt 状态与交互测试，必要的最小 UI 接线 |
| `apps/agents-manager-gui/src-tauri/Cargo.toml` | 版本统一为 0.4.0 |
| `CHANGELOG.md` | Codex MCP 导入与采纳说明 |

新增/扩展内部接口：

```rust
fn platform_mcp_entry_from_bytes(request: &ImportRequest, source_bytes: &[u8]) -> Result<Value, CoreError>;
fn codex_mcp_entry_from_toml(name: &str, source_bytes: &[u8]) -> Result<Value, CoreError>;
```

## 边界与风险

| 场景 | 期望行为 | 测试 |
| --- | --- | --- |
| env indirection | 保留变量名，不当成 literal secret | `codex_mcp_env_indirection_imports_without_secret_values` |
| static Authorization | plan redacted + blocked | `codex_mcp_literal_authorization_blocks_import_without_leak` |
| 等价 target | ledger-only adopt、hash 不变 | `codex_equivalent_mcp_adopt_is_zero_target_write` |
| drift | stale plan 拒绝 | `codex_mcp_import_rejects_source_or_target_drift` |
| foreign entry | 不 adopt、不覆盖 | `codex_foreign_mcp_remains_unmanaged` |

**非目标**：本任务不实现 Claude/Hermes MCP 反向导入，不把真实 secret 写入 canonical source，不自动接管整份平台配置。

## 回滚

Import 使用现有 durable transaction/rollback；adopt 只新增 ledger，可通过精确 retract ownership/reviewed rollback 撤销。任何失败前后均校验 source/target fingerprint；AI-Maker-Market 的现有 `.codex/config.toml` 保持可独立使用。

## Todos

- [x] **T001 Core import RED**：在 `migration_import.rs` 添加 Codex stdio、bearer env、env header、literal Authorization、missing entry、drift 测试；运行 `cargo test -p agents-manager-core --test migration_import codex_mcp -- --nocapture`，预期因 NotImplemented/缺解析失败。
- [x] **T002 Core import GREEN**：扩展 `import_source_location` 和 `platform_mcp_entry_from_bytes`，用 `toml_edit` 读取 named table、删除 legacy `type`、生成 `targets:["codex"]` canonical；复跑 T001 全绿。
- [x] **T003 Adopt RED→GREEN**：为 migration inventory/plan 添加 canonical 与 Codex entry normalized equivalence 测试；实现逐 server `AdoptEquivalent`，apply 仅写 ledger且目标 hash/mode 不变；运行相关 core tests。
- [x] **T004 CLI RED→GREEN**：新增 CLI 集成测试，覆盖 `import mcp --from codex --to project` plan/apply/rollback 与平台目标零写入；薄接 core 并验证结构化输出。
- [x] **T005 GUI RED→GREEN**：补 command bridge Rust 测试和 Import/Adopt 前端状态测试；确保 Codex MCP 可 reviewed import、Equivalent 经确认后显式 adopt、目标文件零写入。
- [x] **T006 版本与文档**：统一所有 0.4.0 manifest 与界面版本，更新 CHANGELOG，并运行 tag-version 脚本。
- [x] **T007 AI-Maker-Market canary**：用 0.4 CLI 对 11 个 Codex MCP 完成 equivalence/adopt/rollback/re-adopt；前后校验 `.codex/config.toml` hash 相同且无凭据输出。
- [x] **T008 全量验证**：`cargo test -p agents-manager-core -p agents-manager-cli`、`cargo test -p agents-manager-gui`、`npm run test`、`npm run build`、debug CLI doctor/status；全部退出码 0，代码审查无剩余 Critical/Important。
