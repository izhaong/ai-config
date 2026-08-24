# Plan: 跨项目复制资产

## 技术方案

- **Core**：已有 `platform_scan::copy_asset_to_asset_root`；补单元测试（skill 跨根复制、同名冲突、同项目拒绝）。
- **Tauri**：已有 `cmd_assets_transfer`；无需改 Rust 命令层。
- **GUI**：`tauriAssets.transferAssets` → `CopyToProjectModal` → `useAssetOperations` + `RowSyncActions` 批量按钮；仅 `browsingSource` 时可用。
- **i18n**：zh-CN / en-US 文案。

## Todos

- [x] T001 core：`copy_asset_to_asset_root` 单测（验证：skill 复制 + 冲突 + 同项目）
- [x] T002 GUI：`transferAssets` API + `CopyToProjectModal` + 工具栏接线 + i18n
- [x] T003 验证：`cargo test -p agents-manager-core` + `cd apps/agents-manager-gui && npm run test && npm run build` + CHANGELOG
