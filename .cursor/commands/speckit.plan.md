---
description: 基于 spec.md 生成 plan.md（技术方案 + ## Todos，贴合 Cursor Plan）
handoffs:
  - label: Implement
    agent: speckit.implement
    prompt: 按 plan.md ## Todos 实现
---

## User Input

```text
$ARGUMENTS
```

## 上下文

- 模板：`.specify/templates/plan-template.md`
- 架构：`docs/product/ARCHITECTURE.md`
- 输入：`specs/<feature>/spec.md`
- 输出：`specs/<feature>/plan.md`（**含 `## Todos`**）

## 步骤

1. 解析 `FEATURE_DIR`（`check-prerequisites.sh --json` 或分支/specs 推断）。
2. 阅读 `spec.md` 与 constitution。
3. 编写 **plan.md**：
   - Crate 影响面、模块/API、数据模型、平台矩阵、测试策略、回滚
   - **`## Todos`**：可勾选 `- [ ] T00n …（验证：…）`；顺序 core → CLI → Tauri → GUI → 文档；末项 verify
4. Todos 即 Cursor Plan 的执行清单；**不生成独立 `tasks.md`**。
5. 若 Agent 使用 TodoWrite，将 `## Todos` 同步到会话 Todo。

## 约束

- 业务逻辑落在 `ai-config-core`
- Karpathy：最简单可行路径
