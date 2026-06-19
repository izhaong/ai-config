---
description: 将 plan.md 拆解为可执行 tasks.md
handoffs:
  - label: Implement
    agent: speckit.implement
    prompt: 按 tasks.md 实现
---

## User Input

```text
$ARGUMENTS
```

## 上下文

- 模板：`.specify/templates/tasks-template.md`
- 输入：`specs/<feature>/plan.md`
- 输出：`specs/<feature>/tasks.md`

## 步骤

1. 定位 `FEATURE_DIR`（`check-prerequisites.sh --json` 或用户指定）。
2. 阅读 `spec.md` + `plan.md`。
3. 生成 **tasks.md**：
   - 任务粒度：单次 commit 可描述的一步
   - 顺序：core 测试 → CLI → Tauri → GUI → 文档/CHANGELOG
   - 每项格式：`- [ ] T00n 描述（验证：…）`
   - 末项固定：`- [ ] 运行 ai-config-verify（cargo test + gui build + doctor）`
4. 可选：在 `checklists/` 添加 `test.md`、`security.md` 清单。
5. 提示：`/speckit.implement`
