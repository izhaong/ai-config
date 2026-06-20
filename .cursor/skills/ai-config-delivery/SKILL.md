---
name: ai-config-delivery
description: ai-config 功能交付：Spec Kit → 分支 → core/CLI/GUI 实现 → 验证 → CHANGELOG。非平凡改动先 spec/plan/tasks。
---

# ai-config 模块交付

端到端在 **ai-config 仓库**交付工具能力（非用户 `~/.ai-config/` 资产内容）。

## 何时使用

- 新子命令、新资产类型、新平台适配、GUI 新面板
- 跨 core + CLI + GUI 的联动功能

## 流程

### 0. 门禁

- 读 `.specify/memory/constitution.md`
- 非平凡功能：先 `/speckit.specify` → `specs/<N-name>/spec.md`

### 1. 规格与计划

```
/speckit.plan  → plan.md（crate 边界、API、回滚）
/speckit.tasks → tasks.md（可勾选任务）
```

### 2. 分支

```bash
git checkout develop && git pull --ff-only origin develop
git checkout -b feat/<issue>-<short-name>
```

### 3. 实现顺序（默认）

1. `ai-config-core` + 测试
2. `ai-config-cli` 子命令（若需要）
3. Tauri `lib.rs` commands（若需要）
4. React GUI（若需要）
5. `CHANGELOG.md`、`AGENTS.md`（路由变化时）

### 4. 验证

执行 skill **`ai-config-verify`** 或：

```bash
cargo test -p ai-config-core -p ai-config-cli
cd apps/ai-config-gui && npm run build
ai-config doctor
```

### 5. 收尾

- 用户要求时再 commit / push / PR → `develop`
- PR 描述链到 spec 路径或 Issue

## 原则

- Karpathy：最小 diff、可验证完成
- core 唯一业务逻辑归宿
