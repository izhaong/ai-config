# Plan: GUI / Rust 遗留项收口

**Branch**: `feat/command-sync`（或 `002-gui-rust-hygiene`）  
**Status**: Done

## 目标

1. **core::asset_ops** — 单条 deploy/retract/get/save/delete 收敛至 core，GUI `lib.rs` 仅薄封装
2. **CLI doctor** — `lifecycle::run_doctor` 调用 `core::doctor::compute_report`
3. **GUI 工程化** — ESLint + Vitest（`canOpenEntry` 等纯函数单测）

## 验收

- `cargo test -p ai-config-core -p ai-config-cli -p ai-config-gui`
- `cd apps/ai-config-gui && npm run build && npm run lint && npm run test`
- `lib.rs` 行数显著下降（目标 <1200）

## 回滚

Revert `asset_ops` 模块与 `lib.rs` 命令薄封装；CLI doctor 可暂回本地 `DoctorReport` 实现。
