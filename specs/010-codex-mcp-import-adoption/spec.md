# Feature Specification: Codex MCP source-first 导入与等价采纳

**Feature Branch**: `fix/gui-unmanaged-copy-actions`  
**Created**: 2026-07-23  
**Status**: Approved  
**Input**: 修复 ai-config，使项目现有 `.codex/config.toml` 中的 MCP 可无损导入项目 source、在 GUI 显示，并在语义等价时建立受管所有权而不改写目标配置。

## User Scenarios & Testing

### User Story 1 - 从 Codex 项目配置导入 MCP (Priority: P1)

用户在任意目录克隆项目后，可以把 `.codex/config.toml` 中指定的 `[mcp_servers.<name>]` 导入 `<repo>/.ai-config/mcp/servers/<name>.json`，不依赖旧电脑绝对路径，也不写入真实凭据。

**Independent Test**: 在临时项目写入包含 stdio、HTTP bearer env 和 env header 的 Codex TOML，逐项执行 import plan/apply，断言生成 canonical JSON 且原 TOML 字节不变。

**Acceptance Scenarios**:

1. **Given** Codex MCP 使用 `bearer_token_env_var`，**When** 导入，**Then** canonical source 保留环境变量名并以 `targets=["codex"]` 路由。
2. **Given** Jenkins 使用 `env_http_headers.Authorization`，**When** 导入，**Then**只保存变量名，不读取或序列化变量值。
3. **Given** 同一 TOML 含其他 MCP 和注释，**When** 导入一个 server，**Then** 原文件及其他 entry 零变化。

### User Story 2 - 等价目标无写入采纳 (Priority: P1)

canonical source 与现有 Codex entry 语义等价时，用户可以显式确认 adopt，ai-config 只建立逐 server ownership ledger，不重写 `.codex/config.toml`。

**Independent Test**: 快照目标 hash/mode，执行 reviewed adopt，断言 hash/mode 不变且 ledger 新增精确 `mcp_servers.<name>` 记录；再次执行为幂等 noop。

**Acceptance Scenarios**:

1. **Given** target 与 canonical 等价且未托管，**When** plan，**Then** 返回可选择 `AdoptEquivalent` action。
2. **Given** target 与 canonical 不同，**When** plan/apply，**Then** fail-closed，不建立 ownership、不覆盖目标。

### User Story 3 - GUI 完成项目 MCP 管理 (Priority: P2)

用户在已注册项目页看到 Codex MCP；未导入的 Codex entry 可执行 Import，等价未托管 entry 可执行 Adopt，成功后显示 managed。

**Independent Test**: GUI command bridge 与前端状态测试覆盖 Missing/Equivalent/Foreign/Managed 四种状态，以及 stale digest 拒绝。

## Edge Cases

- TOML 无 `mcp_servers`、指定 entry 不存在、entry 不是 table：只读报错。
- legacy `type`、Codex 支持字段和未知非凭据字段：导入时去除 `type`，其余字段保持；目标文件不改写。
- 静态 `Authorization`、疑似 token/password、URL userinfo、凭据 query/args：计划只报告 key/reason，不包含值，apply 拒绝。
- source 已存在且不同：必须显式 `--replace` 并经过新 plan；默认零写。
- plan digest、源 fingerprint、目标 fingerprint 任一漂移：apply 拒绝。

## Requirements

### Functional Requirements

- **FR-001**: Core MUST 支持从 Codex `.codex/config.toml` 读取指定 named MCP table，并生成 canonical per-server JSON。
- **FR-002**: 导入 MUST 保留 `bearer_token_env_var`、`env_http_headers` 和相对 command/args，且 MUST NOT 解析环境变量值。
- **FR-003**: 静态或疑似凭据 MUST 触发 redacted blocking preflight，plan/日志不得包含值。
- **FR-004**: CLI `import mcp <name> --from codex --to project` MUST 支持 review plan、digest-bound apply 和 rollback。
- **FR-005**: 等价未托管 Codex entry MUST 产生逐 entry adopt action；adopt MUST 不改写目标内容。
- **FR-006**: GUI MUST 复用 core plan/apply，不复制 TOML 解析或所有权逻辑。
- **FR-007**: AI-Maker-Market 当前 11 个 Codex MCP MUST 可作为项目 source 被 CLI/GUI 列出。
- **FR-008**: CLI、GUI、Tauri 与 workspace 发布版本 MUST 一致为 `0.4.0`。

## Success Criteria

- **SC-001**: Codex MCP import 主路径、凭据阻断、漂移拒绝、等价采纳测试全部通过。
- **SC-002**: AI-Maker-Market 11/11 MCP source 可列出，`.codex/config.toml` 在导入/采纳前后 hash 不变。
- **SC-003**: GUI 项目页可显示 11 个 Codex MCP，并对 Equivalent 提供显式 Adopt。
- **SC-004**: `cargo test -p ai-config-core -p ai-config-cli`、GUI Rust tests、Vitest 和 build 均为退出码 0。

