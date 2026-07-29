---
description: 按 plan.md ## Todos 实现 ai-config 功能并验证
---

## User Input

```text
$ARGUMENTS
```

## 步骤

1. **前置**（若脚本存在）：
   ```bash
   .specify/scripts/bash/check-prerequisites.sh --json --require-tasks
   ```
   得到 `FEATURE_DIR`；`plan.md` 须含 `## Todos`。
2. **Checklist**（若 `FEATURE_DIR/checklists/` 存在）：统计未完成项。
3. **按 `plan.md ## Todos` 顺序实现**；遵守 rules 与 constitution；core 优先。
4. **每完成一项**：`plan.md` 勾 `- [x]`；更新会话 Todo（若使用）。
5. **验证**：`/ai-config-verify` 或 skill `ai-config-verify`。
6. **文档**：`CHANGELOG.md`；API 变化时 `AGENTS.md`。

## 停止条件

- 无 `plan.md ## Todos` → 先补 plan，再实现
- 验证失败 → 修复重跑
