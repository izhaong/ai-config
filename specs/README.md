# Feature Specs（Spec Kit）

本目录存放 **ai-config 工具本身** 的功能规格，遵循 [Spec-Driven Development](https://github.com/github/spec-kit)。

## 目录约定

```
specs/
└── <编号>-<短名>/
    ├── spec.md      # 需求与用户场景（/speckit.specify）
    ├── plan.md      # 技术方案（/speckit.plan）
    ├── tasks.md     # 可执行任务清单（/speckit.tasks）
    └── checklists/  # 可选：UX / 安全 / 测试清单
```

## 原则

- 宪法：`.specify/memory/constitution.md`
- 模板：`.specify/templates/`
- 非平凡功能 **必须先有 spec**，再 plan → tasks → implement

## 与产品文档关系

| 文档                  | 用途                      |
| --------------------- | ------------------------- |
| `docs/product/PRD.md` | 产品级长期需求            |
| `specs/<feature>/`    | 单次迭代/特性的可交付规格 |
