# Implementation Plan: 当前分支质量审查与硬化

**Branch**: `fix/gui-unmanaged-copy-actions` | **Date**: 2026-07-16 | **Spec**: [spec.md](./spec.md)

## Summary

本计划审查当前 feature 分支与工作区未提交变更，修复已证实的质量退化，而不重写 source-first 的平台所有权模型。核心安全语义保持不变：没有 marker 或 legacy symlink 的同内容副本为 `synced`，不可作为普通资产收回。

范围包括被意外清空的 tracked Hook 配置、GUI 平台垃圾桶的 core 能力对齐、共享列表布局的 E2E 选择器恢复、已存在的 GUI lint 错误，以及 Rust 单测的进程级环境变量隔离。默认平台为 agent-manager 和详情抽屉滚动修复保留并回归验证。

## 背景与根因

- `.cursor/hooks.json` 当前仅留下空 `hooks` 对象，删除了 HEAD 中所有 `managedBy: agent-manager` 的 Hook 绑定，且产生尾随空格；依据 hooks asset layout，它是平台投影，不能以空白内容替代受管绑定。
- `materialize::retract` 明确拒绝没有 ownership record 的 MCP，而 `canRemoveFromPlatform` 只看状态，可能在 MCP 为 `linked` 时暴露垃圾桶。
- `ListColCheck` 和 `ListColActions` 的组件重构移除了 `list-col-check`、`list-col-actions` 语义 class，现有 Playwright 布局测试等待这些 class 而超时。
- ESLint 报告 `buttonVariants` 的 Fast Refresh 导出、未使用的 `activePlatform` 参数，以及 `useAssetBrowser` 中由 `?? []` 导致的 memo dependency 警告。
- `paths` 单测使用与其余 core 测试不同的 `HOME` 锁，导致并发执行时读取了另一个测试的临时目录并进一步锁中毒。
- 008 source-first 规格规定无 ownership evidence 的同内容副本不可自动接管；GUI 仍会把 `synced` 或 MCP 条目解析为 deploy/retract，导致 no-op 或必失败请求。

```text
平台行垃圾桶
  AssetRow → canRemoveFromPlatform(kind, state) → useAssetOperations.retractAsset
  MCP → asset_ops::retract_mcp → ownership error

目标：在 GUI predicate 阶段禁止 MCP 收回入口，避免进入必然失败的 core 路径。
```

## Technical Context

| 项 | 值 |
| --- | --- |
| Language | Rust + TypeScript |
| Core crate | `crates/agent-manager-core` |
| GUI | React 19 + Tauri 2 + Vitest + Playwright |
| 关键文件 | `materialize.rs`、`platform_scan.rs`、`types.ts`、`ListRowShell.tsx` |
| 测试 | Cargo test、Clippy、ESLint、Vitest、Playwright |

## Constitution Check

- [x] 业务所有权语义保留在 `agent-manager-core`；GUI 只根据已知 core 能力决定是否启用操作。
- [x] 不新增平台路径或同步规则。
- [x] 不改密钥、用户资产或外部平台实际文件。
- [x] 每项变更有对应测试或工具验证。

## 影响面

| 文件 | 变更类型 | 说明 |
| --- | --- | --- |
| `.cursor/hooks.json` | 恢复配置 | 恢复被误删的 tracked Hook 投影 |
| `.cursor/skills/agent-manager-delivery/SKILL.md` | 恢复规则 | 恢复详细 Plan 门禁 |
| `apps/.../types.ts` | GUI predicate | 删除能力按 asset kind 与状态判定 |
| `apps/.../ListRowShell.tsx` | 语义 class | 恢复 E2E 选择器 |
| `apps/.../button.tsx` | lint | 移除无消费者的导出 |
| `apps/.../canOpenEntry.ts` | lint | 删除未使用参数并更新调用者 |
| `apps/.../useAssetBrowser.ts` | lint | 稳定 capability issue 依赖 |
| `apps/.../*.test.ts(x)` | 测试 | 覆盖 MCP 与普通资产操作边界 |
| `crates/.../test_env.rs`、`paths.rs` | 测试隔离 | 串行化进程级环境变量变更 |

## 方案设计

### 操作能力对齐

`canRemoveFromPlatform` 接收 `AssetKind` 与 `LinkState`。只有非 MCP 且 state 为 `linked` 时返回真；这与当前 `asset_ops::retract_mcp` 的 fail-closed 行为一致。`AssetRow` 与批量/单条平台收回调用点都传入 entry kind，保证按钮可用性与执行路径一致。

### 布局与 lint

共享列表列组件重新输出稳定的 `list-col-check` / `list-col-actions` class；CSS 布局仍由现有 Tailwind class 控制。lint 修复只移除未使用接口和导出，或以稳定值作为 memo 依赖，不改变浏览、抽屉或数据请求行为。

### 配置恢复

恢复 HEAD 中的 tracked `.cursor/hooks.json` 和仓库 delivery skill 详细度文本；不运行 sync、不修改 `~/.agent-manager`，避免越过用户资产边界。

## 边界与风险

| 场景 | 期望行为 | 验证 |
| --- | --- | --- |
| 同内容未托管 skill | 显示 `synced`，垃圾桶禁用 | core + Vitest |
| 受管 skill | `linked`，垃圾桶可用 | Vitest |
| MCP server | 垃圾桶禁用 | Vitest |
| 列表无资产 | E2E 仍可定位顶栏列 | Playwright |
| 详情内容超过窗口 | 预览区滚动 | Playwright |

**非目标 / 不做**：不实现 MCP ownership ledger，不改变 `retract_mcp`，不修改用户全局 Hook 源，不重构平台 icon 状态机。

## 测试策略

1. `types.test.ts`：普通受管副本、未托管副本、MCP 的删除能力。
2. `platform_scan.rs`：保持同内容无 marker 副本为 `Synced`。
3. `npm run lint`：0 error、0 warning。
4. `npm run test`、`npm run build`、`npm run test:e2e`。
5. `cargo test -p agent-manager-core -p agent-manager-cli` 与 `cargo clippy -p agent-manager-core -p agent-manager-cli -- -D warnings`。

## 回滚

全部为代码、测试和 tracked 配置恢复；可通过 revert 本次提交恢复。无数据迁移、无用户资产写入。

## Todos

- [x] T001 [config] 恢复被清空的 Hook 投影与 delivery skill 的详细 Plan 约束（验证：JSON parse + `git diff --check`）。
- [x] T002 [GUI] 先补 MCP/普通资产垃圾桶能力的失败测试，再按 asset kind 对齐 GUI predicate（验证：`npm run test -- src/types.test.ts`）。
- [x] T003 [GUI] 恢复列表列语义 class 并消除 ESLint 报告（验证：`npm run lint` + `npm run test:e2e`）。
- [x] T004 [verify] 运行 Rust、GUI、E2E 全量验证并回写结果（验证：本计划测试策略全部命令）。
- [x] T005 [GUI] 禁止 `synced` 资产隐式接管，禁止 MCP 触发无 ownership 的 retract；平台批量删除仅在存在可收回项时启用（验证：Vitest）。
- [x] T006 [core-test] 用可恢复的多变量 EnvGuard 取代裸环境变量修改（验证：core test）。

## 验证记录

- 2026-07-16：`cargo test -p agent-manager-core -p agent-manager-cli`（0 failures）、`cargo clippy -p agent-manager-core -p agent-manager-cli -- -D warnings`（exit 0）、`cargo fmt --check`（exit 0）。
- 2026-07-16：`npm run lint`（exit 0）、`npm run test`（19 files / 88 tests passed）、`npm run build`（exit 0）、`GUI_E2E_URL=http://127.0.0.1:5174 npm run test:e2e`（4 passed）。
