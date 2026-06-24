# Feature Specification: 跨项目复制资产

**Feature Branch**: `003-asset-copy-project`  
**Created**: 2026-06-23  
**Status**: Draft  
**Input**: 将全局（或其它项目）的 skill 等资产复制到某一项目的 `.ai-config/` 中

## User Scenarios & Testing

### User Story 1 - 全局 skill 复制到项目 (Priority: P1)

用户在「用户全局」源视图勾选 `git-sync-github` 等 skill，选择目标已注册项目，一键复制到该项目 `skills/` 目录。

**Why this priority**: 最常见场景——把个人全局资产下发到具体仓库项目。

**Independent Test**: 全局存在 skill → GUI 源视图批量复制 → 目标项目 `.ai-config/skills/<name>/` 出现实体副本。

**Acceptance Scenarios**:

1. **Given** 全局有 skill `git-sync-github`，目标项目无同名 skill，**When** 用户勾选并复制到该项目，**Then** 目标 `skills/git-sync-github/` 含 `SKILL.md` 且与源内容一致。
2. **Given** 用户未勾选任何行，**When** 点击复制到项目，**Then** 提示先选择资产。

---

### User Story 2 - 项目间互拷 (Priority: P2)

用户可在项目 A 源视图将资产复制到项目 B 或用户全局（反向共享）。

**Acceptance Scenarios**:

1. **Given** 项目 A 有 rule `foo.mdc`，全局无同名 rule，**When** 从 A 复制到全局，**Then** 全局 `rules/foo.mdc` 创建成功。

---

### Edge Cases

- 源与目标为同一项目 → 拒绝并提示。
- 目标已存在同名资产 → 拒绝并提示先删除或重命名。
- 仅在 **ai-config 源视图**（非平台浏览）提供复制入口。

## Requirements

### Functional Requirements

- **FR-001**: 系统 MUST 支持将已纳管 skill/rule/agent/command/mcp 从源项目资产根复制到目标项目资产根。
- **FR-002**: GUI MUST 在源视图批量操作区提供「复制到项目」，弹窗选择目标（排除当前项目）。
- **FR-003**: 复制 MUST 写实体副本（沿用 `copy_asset_to_asset_root`），不创建 symlink。
- **FR-004**: 目标已存在同名资产时 MUST 失败并返回可读错误。
- **FR-005**: 复制成功后 MUST 刷新列表并清空勾选。

### Key Entities

- **源/目标项目**：`user-global` 或已注册项目名，映射到各自 `.ai-config/` 资产根。
- **传输项**：`{ kind, name }` 批量列表。

## Success Criteria

- **SC-001**: 用户可在 3 次点击内完成「全局 skill → 项目」复制。
- **SC-002**: `cargo test` 覆盖 skill 跨项目复制主路径与「目标已存在」边界。
- **SC-003**: `npm run test && npm run build` 通过。
