# Feature Specification: 验证闭环强化

**Feature Branch**: `fix/verify-e2e-closure`
**Created**: 2026-07-29
**Status**: Implemented / Runtime Blocked
**Input**: User description: "按照建议的处理"

## User Scenarios & Testing

### User Story 1 - 安全识别真实运行版本 (Priority: P1)

维护者执行验证时，必须明确区分仓库刚构建的二进制与 PATH 中实际安装的二进制，避免用新代码测试结果替旧运行态背书。

**Why this priority**: 本机已出现仓库 `0.4.0`、已安装 `0.3.3` 的版本漂移，且两者的 `sync` 写入门禁不同。

**Independent Test**: 运行 verify skill 的版本门禁，版本一致时通过，不一致时给出两个路径/版本并阻止运行态闭环结论。

**Acceptance Scenarios**:

1. **Given** PATH 中存在 `agent-manager`，**When** 其版本与 `target/debug/agent-manager` 不同，**Then** 验证必须失败并报告版本漂移。
2. **Given** PATH 中不存在 `agent-manager`，**When** 执行源码验证，**Then** 源码测试可继续，但运行态闭环必须标为未验证。

---

### User Story 2 - 隔离验证同步生命周期 (Priority: P1)

维护者可以在临时 HOME 和临时资产根中验证 plan、apply、status、retract/uninstall，不触碰真实 `~/.agent-manager` 与 IDE 配置。

**Why this priority**: 同步类命令可能因旧版本默认写入，真实环境冒烟不能作为安全默认。

**Independent Test**: 运行隔离闭环脚本，确认 plan 零写入、apply 建立目标、status 健康、uninstall apply 收回目标，并自动清理临时目录。

**Acceptance Scenarios**:

1. **Given** 一个仅含测试 skill 的临时资产根，**When** 执行无 `--apply` 的 sync，**Then** 平台目标不存在。
2. **Given** 已审核的临时计划，**When** 执行 `sync --apply`，**Then** 测试 skill 出现在支持的平台目标。
3. **Given** 已创建的受管目标，**When** 执行 `uninstall --apply`，**Then** 测试源仍保留且平台目标被收回。

---

### User Story 3 - Completion 管道安全退出 (Priority: P2)

用户将 completion 输出交给提前关闭的消费者时，CLI 不应 panic。

**Why this priority**: `agent-manager completion zsh | head` 当前稳定触发 broken-pipe panic 和退出码 101。

**Independent Test**: 使用返回 `BrokenPipe` 的 writer 调用 completion 输出边界，断言按成功处理；其它 IO 错误仍返回失败。

**Acceptance Scenarios**:

1. **Given** 下游提前关闭 stdout，**When** 生成 completion，**Then** CLI 安静退出且不打印 panic。
2. **Given** stdout 返回非 BrokenPipe IO 错误，**When** 写 completion，**Then** CLI 返回运行时错误码。

---

### User Story 4 - 功能矩阵可审计 (Priority: P2)

维护者能从 verify skill 看到自动化覆盖、隔离 E2E、真实运行态与必须人工确认的边界，而不是只得到“测试通过”。

**Independent Test**: 阅读 skill 并执行其命令，能够分别得到 source、sandbox、runtime、manual 四层结论。

### Edge Cases

- PATH 中的 `agent-manager` 是 symlink 或不同安装渠道。
- 临时 HOME 中不存在任何平台父目录。
- completion 消费者完整读取或提前关闭。
- 同步计划包含 blocking reason 时，隔离脚本必须停止，不得强行 apply。
- 验证失败或被中断时，临时目录仍需安全清理。

## Requirements

### Functional Requirements

- **FR-001**: verify skill MUST 比较构建二进制与 PATH 二进制的路径和版本。
- **FR-002**: verify skill MUST 使用临时 HOME 执行同步生命周期，不得把真实 HOME 作为写入目标。
- **FR-003**: 隔离 E2E MUST 验证 plan 零写入、apply、status、uninstall/retract 和清理。
- **FR-004**: completion 输出 MUST 将 BrokenPipe 视为正常的消费者关闭，不得 panic。
- **FR-005**: 非 BrokenPipe 的 completion 输出错误 MUST 返回非零退出码并输出简洁错误。
- **FR-006**: verify skill MUST 明确列出 GUI 视觉、IDE 重启可见性等人工/外部门禁。
- **FR-007**: 验证过程 MUST 不读取或输出 secret 值。

## Success Criteria

### Measurable Outcomes

- **SC-001**: 新增 completion 回归测试通过，`completion zsh | head` 退出码为 0 且 stderr 无 panic。
- **SC-002**: 隔离 E2E 完成一次 plan → apply → status → uninstall apply，真实 HOME 无测试资产。
- **SC-003**: 版本不一致时门禁稳定失败，版本一致时通过。
- **SC-004**: `cargo test --workspace`、GUI 92 个现有测试和 GUI build 均 0 失败。
- **SC-005**: 仓库外真实运行态检查只读，除用户明确授权的二进制升级外不执行真实 sync apply。
