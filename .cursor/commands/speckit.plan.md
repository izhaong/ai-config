---
description: 基于 spec.md 生成 ai-config 技术计划 plan.md
handoffs:
  - label: Create Tasks
    agent: speckit.tasks
    prompt: 将 plan 拆解为 tasks.md
---

## User Input

```text
$ARGUMENTS
```

## 上下文

- 模板：`.specify/templates/plan-template.md`
- 架构：`docs/product/ARCHITECTURE.md`
- 输入：`specs/<feature>/spec.md`
- 输出：`specs/<feature>/plan.md`

## 步骤

1. 若存在 `.specify/scripts/bash/check-prerequisites.sh`，可执行：
   ```bash
   .specify/scripts/bash/check-prerequisites.sh --json
   ```
   解析 `FEATURE_DIR`；否则从 `$ARGUMENTS` 或最近编辑的 `specs/` 子目录推断。
2. 阅读 `spec.md` 与 constitution。
3. 编写 **plan.md**：
   - **Crate 影响面**：core / cli / daemon / gui（勾选）
   - **模块/API**：列 public 函数、CLI 子命令、Tauri command 名
   - **数据模型**：`model.rs` 变更
   - **平台矩阵**：Cursor/Codex/Claude/Hermes 是否受影响
   - **测试策略**：单测路径、`cargo test` 范围
   - **回滚**：配置/链接如何恢复
4. 风险与未决项单独列出。
5. 提示下一步：`/speckit.tasks`

## 约束

- 业务逻辑必须落在 `ai-config-core`
- 遵循 Karpathy：方案取最简单可行路径
