---
name: ai-config-delivery
description: ai-config 交付（贴合 Cursor）：用户下发任务 → Agent 自动分支、spec、plan+Todos、实现、验证。
---

# ai-config 模块交付（贴合 Cursor）

用户只说任务；流程适配 **Cursor Plan + Todo**，Spec Kit 只负责**落盘**。

## Cursor ↔ 本 skill

| 步骤       | Cursor       | 落盘                   |
| ---------- | ------------ | ---------------------- |
| 需求       | 用户消息     | `spec.md`              |
| 方案与步骤 | Plan + Todos | `plan.md` + `## Todos` |
| 执行       | 按 Todo 写码 | 同步勾选 `plan.md`     |
| 收尾       | verify       | constitution §5        |

## 触发

- 新子命令、跨 crate、平台适配、GUI 面板等非平凡改动

## Agent 流程

### 0. 分流

平凡改动 → 直接改；否则继续。

### 1. 分支 + FEATURE_DIR

见 `.cursor/rules/gitflow-spec-kit.mdc`、`.cursor/rules/spec-kit-gate.mdc`。

### 2. spec + plan（含 Todos）

| 文件      | 内容                                                                            |
| --------- | ------------------------------------------------------------------------------- |
| `spec.md` | 场景、可测需求、成功标准                                                        |
| `plan.md` | **详细**技术方案 + **`## Todos`**（模板 `.specify/templates/plan-template.md`） |

**plan 详细度（必达）**：写码前先读相关源码。plan 须含：Summary、根因（文件/函数级）、Technical Context、Constitution Check、影响面（含 FR→实现表）、方案设计（含全局/项目对比表若适用）、边界表、测试策略、回滚。禁止仅模块表 + 笼统 Todo。

Todos：`- [ ] T001 [scope] …（验证：具体 test 或命令）`；粒度可独立 review；末项 verify。

### 3. 实现

按 `plan.md ## Todos`：core → CLI → Tauri → GUI → CHANGELOG。每项完成 → `- [x]`。

### 4. 验证

`ai-config-verify` skill；失败则修复重跑。

### 5. 收尾

用户要求时再 commit/PR；汇报分支、`specs/` 路径、验证结果。

## 原则

- Spec Kit 贴合 Cursor，不造平行流程
- core 唯一业务逻辑；Karpathy 最小 diff
