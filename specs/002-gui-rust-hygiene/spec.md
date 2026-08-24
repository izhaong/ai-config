# Feature Specification: GUI / Rust 遗留项收口

**Feature Branch**: `002-gui-rust-hygiene`  
**Status**: Done

## User Story 1 - Core 资产操作收敛 (Priority: P1)

单条 deploy/retract/get/save/delete 收敛至 `agent-manager-core::asset_ops`；GUI `lib.rs` 仅薄封装 invoke。

**Independent Test**: `cargo test -p agent-manager-core` 覆盖 asset_ops；GUI 命令调用 core 无重复逻辑。

## User Story 2 - CLI doctor 统一 (Priority: P2)

`lifecycle::run_doctor` 调用 `core::doctor::compute_report`，与 GUI/CLI 共用诊断逻辑。

**Independent Test**: `cargo test -p agent-manager-cli` doctor 相关用例通过。

## User Story 3 - GUI 工程化 (Priority: P2)

ESLint + Vitest；`canOpenEntry` 等纯函数有单测。

**Independent Test**: `cd apps/agent-manager-gui && npm run lint && npm run test && npm run build`。

## 成功标准

见 [plan.md](./plan.md) 验收节；全部 Todos 已勾选完成。
