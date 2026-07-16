# Specification Quality Checklist: 单一事实源与安全投影

**Purpose**: 验证规格完整性与平台资产契约，防止实现偏离官方路径、作用域和所有权边界。
**Created**: 2026-07-16
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] 规格聚焦用户价值、平台消费契约与安全边界
- [x] 所有强制章节完整
- [x] 平台路径、格式和作用域只作为外部产品契约描述
- [x] 没有把当前代码结构误写成用户需求

## Requirement Completeness

- [x] 无 `[NEEDS CLARIFICATION]` 标记
- [x] Requirements 可测试且无歧义
- [x] Success Criteria 可量化
- [x] 所有主要用户场景有验收条件
- [x] Edge Cases 覆盖 legacy、foreign、冲突、scope 越界与 secret
- [x] 范围和 Out of Scope 明确
- [x] 依赖与 Assumptions 已记录

## Platform Contract Coverage

- [x] Skills 明确 Cursor/Codex 共享 `.agents/skills` 与 Claude/Hermes 边界
- [x] Rules 区分 instruction rules 与 Codex execution-policy rules
- [x] MCP 明确 Cursor JSON、Codex TOML、Claude user/project JSON、Hermes YAML
- [x] Agents 明确 Cursor/Claude Markdown 与 Codex TOML，禁止 `subagents` 假路径
- [x] Commands 明确 Cursor/Claude 支持与 Codex/Hermes unsupported/迁移路径
- [x] Hooks 明确脚本单元、聚合配置、事件适配、trust 与 project scope 边界
- [x] Legacy/alternate 路径只用于 inventory/migration，不作为默认写目标
- [x] 固定 Skills → Rules → MCP → Agents → Commands → Hooks 的 TDD/验收顺序、逐任务 commit checkpoint，并将真实环境限制为只读盘点加单项确认 canary

## Feature Readiness

- [x] 所有功能需求均可追溯到用户场景或平台契约
- [x] 契约变化有先失败的测试要求
- [x] 防偏航规则覆盖路径、格式、共享 target、聚合容器与 unsupported
- [x] 规格可进入 plan 对齐和 T003 平台契约测试阶段

## Notes

- 本 checklist 只验证规格质量，不表示 T002–T013 已实现。
- `docs/product/PRD.md`、`ARCHITECTURE.md` 与现有 adapter 中仍可能存在硬拷贝、旧路径和 `subagents` 等历史语义；实现前必须以本 Spec 为准逐项清理。
