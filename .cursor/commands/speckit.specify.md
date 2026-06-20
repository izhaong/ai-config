---
description: 为 ai-config 创建或更新功能规格 spec.md（Spec Kit）
handoffs:
  - label: Build Technical Plan
    agent: speckit.plan
    prompt: 基于 spec 创建 plan.md
  - label: Clarify Spec Requirements
    agent: speckit.clarify
    prompt: 澄清规格需求
    send: true
---

## User Input

```text
$ARGUMENTS
```

## 上下文

- 宪法：`.specify/memory/constitution.md`
- 产品 PRD：`docs/product/PRD.md`
- 架构：`docs/product/ARCHITECTURE.md`
- 模板：`.specify/templates/spec-template.md`
- 输出目录：`specs/<编号>-<短名>/spec.md`

## 步骤

1. **解析输入**：功能名、用户价值、是否影响 core/CLI/GUI/平台适配。
2. **检查分支**：建议在 `feat/*` 或新建 `feat/<N>-<name>`（勿在 develop 直接改）。
3. **确定 FEATURE_DIR**：
   - 若已有 `specs/*` 匹配主题则更新该目录
   - 否则新建 `specs/<三位编号>-<kebab-name>/`（编号取现有最大 +1 或用户指定）
4. **编写 spec.md**（基于 `spec-template.md`）：
   - User Scenarios（P1/P2…，可独立验收）
   - Functional Requirements（须可测试）
   - 明确 **不涉及** 用户 `~/.ai-config/` 资产内容变更（除非工具行为本身）
   - 成功标准对齐 constitution §5 验证
5. **不写实现代码**；仅产出/更新 spec。
6. 提示用户下一步：`/speckit.plan`

## 质量检查

- [ ] 每个 User Story 有 Independent Test
- [ ] 边界与 ARCHITECTURE（core 唯一逻辑）一致
- [ ] 无真实密钥示例
