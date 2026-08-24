# Feature Specification: 当前分支质量审查与硬化

**Feature Branch**: `fix/gui-unmanaged-copy-actions`  
**Created**: 2026-07-16  
**Status**: Complete
**Input**: 用户要求全面检查此前较弱模型完成的编码并优化。

## User Scenarios & Testing

### User Story 1 - 保留受管配置与资产安全边界 (Priority: P1)

维护者审查当前工作区后，受管 Hook 配置不会因意外空白变更而全部失效；普通同内容平台副本仍不会被误当作可收回资产。

**Why this priority**: Hook 清空会直接改变开发环境行为，错误收回会删除用户外部资产。

**Independent Test**: 校验 `.cursor/hooks.json` 保留受管条目；Rust 现有 platform scan 用例验证无 marker 的同内容副本为 `synced`。

**Acceptance Scenarios**:

1. **Given** HEAD 中有受管 Hook，**When** 审查完成，**Then** 工作区 `.cursor/hooks.json` 不再是空 `hooks` 对象。
2. **Given** 平台副本与源内容相同但无 marker，**When** 扫描状态，**Then** 状态为 `synced` 而非 `linked`。

---

### User Story 2 - 平台操作只暴露可执行动作 (Priority: P1)

用户在平台浏览视图操作垃圾桶时，只能对 core 能证明所有权并支持收回的资产执行该操作。

**Why this priority**: UI 不应暴露后端必然拒绝的动作，也不能引导用户误以为外部副本可安全删除。

**Independent Test**: Vitest 覆盖 materialized asset 与 MCP 的删除可用性分支。

**Acceptance Scenarios**:

1. **Given** 普通 skill 副本无 ownership marker，**When** 用户浏览该平台，**Then** 垃圾桶禁用。
2. **Given** 平台 MCP server，**When** core 尚无 MCP ownership record，**Then** 垃圾桶禁用。

---

### User Story 3 - GUI 回归检查可持续运行 (Priority: P2)

维护者运行 GUI lint 与 E2E 时，测试使用组件仍保留的语义选择器，且不产生现有的 lint 错误。

**Why this priority**: 失效测试和 lint 错误会掩盖后续真实回归。

**Independent Test**: `npm run lint` 与 `npm run test:e2e` 均通过。

**Acceptance Scenarios**:

1. **Given** 顶栏和行均使用共享三列布局，**When** E2E 查询操作列与复选框列，**Then** 找到稳定语义 class 并完成对齐断言。
2. **Given** GUI 源码，**When** 运行 ESLint，**Then** 无 error 或 warning。

### Edge Cases

- MCP 的平台收回在 core 有可验证 ownership 前保持禁用，不以内容相同作为删除授权。
- Hook 权威源在用户全局资产根；本次只恢复仓库中被意外清空的 tracked 平台配置，不覆盖用户全局资产。
- 已有未提交业务改动必须保留，不以格式化或重置方式清理。

## Requirements

### Functional Requirements

- **FR-001**: 仓库 MUST 不提交空白的 `.cursor/hooks.json` 来替代既有受管 Hook 绑定。
- **FR-002**: GUI MUST 仅在资产类型与状态均支持 core 收回时启用平台垃圾桶。
- **FR-003**: GUI MUST 保留稳定的三列布局语义选择器供现有 E2E 使用。
- **FR-004**: GUI MUST 通过 ESLint、Vitest、生产构建和 E2E 回归。
- **FR-005**: 本次审查 MUST 保留无 marker 的同内容副本为 `synced` 的 core 语义。

## Success Criteria

### Measurable Outcomes

- **SC-001**: `cargo test -p agents-manager-core -p agents-manager-cli` 与 Clippy 无失败。
- **SC-002**: `npm run lint`、`npm run test`、`npm run build`、`npm run test:e2e` 全部退出码为 0。
- **SC-003**: 针对普通未托管副本、MCP、受管副本至少各有一条可执行断言。
