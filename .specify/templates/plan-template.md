# Implementation Plan: [FEATURE]

**Branch**: `[###-feature-name]` | **Date**: [DATE] | **Spec**: [spec.md](./spec.md)

> Agent 生成 plan 时须**尽量详细**：让未参与 spec 的开发者仅凭本文即可实现与验收。禁止仅列模块名或一句概括。

## Summary

[1–2 段：从 spec 提炼的核心目标、技术路线、不做什么]

## 背景与根因

[问题如何复现；涉及的错误行为；**具体文件/函数**与调用链；为何现有实现不足]

```text
[可选：调用链或数据流 ASCII / mermaid]
```

## Technical Context

| 项         | 值                                                   |
| ---------- | ---------------------------------------------------- |
| Language   | Rust (workspace edition)                             |
| 主要 Crate | `agent-manager-core` / `agent-manager-cli` / `agent-manager-gui` |
| 依赖模块   | [如 `paths`, `source`, `hook_adapter`, `sync`]       |
| 测试       | `cargo test -p …`；[具体 test 模块名]                |
| 平台矩阵   | Cursor / Codex / Claude / Hermes（勾选本功能涉及者） |

## Constitution Check

- [ ] 业务逻辑仅在 `agent-manager-core`
- [ ] CLI/GUI 薄封装，无重复实现
- [ ] 平台差异收敛在 `platform` / adapter
- [ ] 幂等、可重复 install/sync
- [ ] 验证项对齐 constitution §5

## 影响面

### Crate / 文件

| Crate            | 文件       | 变更类型  | 说明         |
| ---------------- | ---------- | --------- | ------------ |
| `agent-manager-core` | `src/….rs` | 新增/修改 | [职责一句话] |
| `agent-manager-cli`  | `src/….rs` | 修改      | [仅当涉及]   |

### API / 类型（新增或签名变更须写明）

```rust
// 示例：pub fn foo(…) -> Result<…>
```

### 需求追溯（FR → 实现）

| FR     | 实现位置   | 验收方式 |
| ------ | ---------- | -------- |
| FR-001 | `paths::…` | `test_…` |
| FR-002 | …          | …        |

## 方案设计

### [子系统 1 标题]

[行为说明、算法步骤、与周边模块的交互]

**全局 install** vs **项目 install**（若适用）：

| 维度           | 全局 | 项目 (`AI_CONFIG_ROOT=<repo>`) |
| -------------- | ---- | ------------------------------ |
| 资产扫描根     | …    | …                              |
| default 合并源 | …    | …                              |
| deploy_base    | …    | …                              |

### [子系统 2 标题]

[合并策略、跳过条件、`managedBy` 语义等]

## 边界与风险

| 场景     | 期望行为 | 测试名（计划） |
| -------- | -------- | -------------- |
| [边界 1] | …        | `test_…`       |
| [边界 2] | …        | …              |

**非目标 / 不做**：…

## 测试策略

1. **单元测试**（`agent-manager-core`）：[列举用例与断言要点]
2. **CLI / lifecycle**（若涉及）：[列举]
3. **回归**：`cargo test -p agent-manager-core -p agent-manager-cli`
4. **手工**（可选）：`agent-manager doctor`、项目 install 冒烟步骤

## 回滚

[如何 revert；是否有数据迁移；用户侧影响]

## Todos

<!-- 与 Cursor Plan/Todo 一体；每项可独立交付并带验证 -->

- [ ] T001 [core] …（验证：`cargo test …::test_name`）
- [ ] T002 [cli] …（验证：…）
- [ ] T00N 运行 agent-manager-verify（`cargo test` + GUI build + doctor）
