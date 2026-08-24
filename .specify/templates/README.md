# Spec Kit 模板（agents-manager）

| 模板                       | 用途                                                                                       |
| -------------------------- | ------------------------------------------------------------------------------------------ |
| `spec-template.md`         | `specs/<feature>/spec.md` — 用户场景与可测需求                                             |
| `plan-template.md`         | `specs/<feature>/plan.md` — **详细**技术方案（根因/影响面/边界/测试/回滚）+ **`## Todos`** |
| `checklist-template.md`    | 可选 `checklists/`                                                                         |
| `constitution-template.md` | 新项目宪法参考                                                                             |
| ~~`tasks-template.md`~~    | **已废弃** — Todos 在 `plan.md`                                                            |

工作流：`spec.md` → `plan.md`（含 Todos）→ 按 Todos 实现 → 验证。用户只说任务，Agent 自动落盘。
