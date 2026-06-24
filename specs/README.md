# Feature Specs（Spec Kit · 贴合 Cursor）

本目录存放 **ai-config 工具本身** 的功能规格。

## 目录约定

```
specs/
└── <编号>-<短名>/
    ├── spec.md      # 需求与用户场景
    ├── plan.md      # 技术方案 + ## Todos（Plan 与执行清单合一）
    └── checklists/  # 可选
```

## Cursor ↔ Spec Kit

| Cursor | 本目录 |
| ------ | ------ |
| 用户任务 | `spec.md` |
| Plan | `plan.md` 正文 |
| Todos | `plan.md` → `## Todos` |

## 原则

- 宪法：`.specify/memory/constitution.md`
- 模板：`.specify/templates/`
- 非平凡功能：**spec → plan（含 Todos）→ 实现**；用户只说任务，Agent 自动落盘

## 与产品文档

| 文档 | 用途 |
| ---- | ---- |
| `docs/product/PRD.md` | 产品级长期需求 |
| `specs/<feature>/` | 单次迭代的可交付规格与 Plan |
