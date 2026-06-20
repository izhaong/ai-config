---
description: 按 tasks.md 实现 ai-config 功能并验证
---

## User Input

```text
$ARGUMENTS
```

## 步骤

1. **前置**（若脚本存在）：
   ```bash
   .specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks
   ```
   得到 `FEATURE_DIR`；必须有 `tasks.md`。
2. **Checklist**（若 `FEATURE_DIR/checklists/` 存在）：统计 `- [ ]` / `- [x]`；有未完成项时询问用户是否继续。
3. **按 tasks.md 顺序实现**：
   - 遵守 `.cursor/rules/` 与 constitution
   - core 优先；GUI 不重复业务逻辑
4. **每完成一项**：在 `tasks.md` 勾选 `- [x]`（用户要求提交时再 commit）。
5. **验证**：执行 `/ai-config-verify` 或 skill `ai-config-verify` 全部命令。
6. **文档**：更新 `CHANGELOG.md`；行为/API 变化时更新 `AGENTS.md`。
7. **汇报**：已完成任务、验证结果、未决项。

## 停止条件

- 任务要求与用户输入冲突 → 先澄清
- 验证失败 → 修复后重跑，不得标 completed
