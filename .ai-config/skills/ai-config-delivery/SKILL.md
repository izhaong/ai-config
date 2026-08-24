---
name: agent-manager-delivery
description: agent-manager 交付（贴合 Cursor）：用户下发任务 → Agent 自动分支、spec、plan+Todos、实现、验证。
---

# agent-manager 模块交付（贴合 Cursor）

用户只说任务；流程适配 **Cursor Plan + Todo**，Spec Kit 只负责**落盘**。

## Cursor ↔ 本 skill

| 步骤 | Cursor | 落盘 |
| ---- | ------ | ---- |
| 需求 | 用户消息 | `spec.md` |
| 方案与步骤 | Plan + Todos | `plan.md` + `## Todos` |
| 执行 | 按 Todo 写码 | 同步勾选 `plan.md` |
| 收尾 | verify | constitution §5 |

## 触发

- 新子命令、跨 crate、平台适配、GUI 面板等非平凡改动

## Agent 流程

### 0. 分流

平凡改动 → 直接改；否则继续。

### 1. 分支 + FEATURE_DIR

见 `.agent-manager/rules/gitflow-spec-kit.mdc`、`.agent-manager/rules/spec-kit-gate.mdc`。

### 2. spec + plan（含 Todos）

| 文件 | 内容 |
| ---- | ---- |
| `spec.md` | 场景、可测需求、成功标准 |
| `plan.md` | 技术方案（core/CLI/GUI、回滚）+ **`## Todos`** |

Todos：`- [ ] T001 …（验证：…）`；末项 verify。

### 3. 实现

按 `plan.md ## Todos`：core → CLI → Tauri → GUI → CHANGELOG。每项完成 → `- [x]`。

### 4. 验证

`agent-manager-verify` skill；失败则修复重跑。

### 5. 收尾

用户要求时再 commit/PR；汇报分支、`specs/` 路径、验证结果。

## 原则

- Spec Kit 贴合 Cursor，不造平行流程
- core 唯一业务逻辑；Karpathy 最小 diff
