# 单一事实源与安全投影 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `subagent-driven-development` (recommended) or `executing-plans` to implement this plan task-by-task. All execution tracking stays in this file under `## Todos`; do not create `tasks.md`.

**Goal:** 将 ai-config 重构为 skills、rules、commands、agents、入口提示词、hooks 与 MCP 的唯一事实源，并以“同格式逐项链接、异构格式安全生成”的单向投影替换五平台对等硬拷贝。

**Architecture:** Core 先把 `global → workspace → project` 解析为带 provenance 的 effective assets，再由平台 adapter 生成只读 `ProjectionPlan`。Apply 阶段只创建精确软链或修改具名生成条目；正确链接本身可证明所有权，生成物由可丢失但安全降级的 projection ledger 辅助证明，ledger 缺失时一律保守视为 foreign。CLI、MCP server 和 GUI 只调用同一 planner/executor。

**Tech Stack:** Rust workspace、camino、serde/serde_json、`sha2 = "0.10"`（lock 0.10.9）、`hex = "0.4"`（lock 0.4.3）、`toml_edit = "0.25"`（lock 0.25.12）、`yaml-edit = "0.2.3"`、rusqlite、Tauri 2、React、Vitest。

**Branch:** `008-source-first-projection`

**Date:** 2026-07-12

**Spec:** [spec.md](./spec.md)

## Global Constraints

- `~/.ai-config/` 是 user-global 唯一源；`<repo>/.ai-config/` 是 project override；平台目录永远不是自动回流源。
- Effective precedence 固定为 `project > workspace > global`，同 kind + name 整项覆盖，异名并集。
- Direct assets 只允许逐项链接，禁止链接整个 `skills/`、`rules/`、`agents/`、`commands/`、`prompts/` 或 `hooks/` 根目录。
- 内容相等只表示 `equivalent`，不能单独证明 ai-config 所有权。
- `list`、`status`、`doctor`、inventory 和 dry-run plan 必须零写入。
- Foreign、第三方软链、插件/builtin 和 `.cc-switch` 资产默认只报告，不覆盖、不删除、不 adopt。
- MCP canonical source 必须是 `mcp/servers/<name>.json` + secret references；真实 secret 只在 `~/.config/ai-config/secrets.env`，权限 0600。
- Plan/JSON/log/GUI/ledger/backup manifest 不得包含 secret 明文；含 secret 的目标备份必须位于 0700 目录且文件为 0600。
- Windows fallback 必须显式显示为 `copied`；绝不伪装成 `managed_link`。
- 第一阶段安全正确性不得依赖 daemon、watcher、event bus 或 ledger 可用；ledger 丢失必须安全降级为“不删除”。
- 每个 tranche 均须独立通过测试和 review，禁止一次性切换真实 HOME 中所有资产。

---

## Summary

当前实现同时保留三套互斥模型：README/HERMES 描述单一源软链，PRD v0.5 描述五平台对等硬拷贝，代码则以 `materialize` 实体复制、内容比对和残留 `link` 检查混合运行。结果是所有权无法可靠判断、平台副本漂移、CLI/GUI/doctor 状态不一致，并且平台 adapter 已偏离 Codex、Claude 的当前官方路径。

本计划分为三个可独立发布的 tranche：

1. **Tranche A（T001–T005）— 安全止血 + read-only planner**：先封死误删、整文件 MCP 删除和读操作写入；建立统一状态模型、官方平台契约、overlay 和只读 plan。
2. **Tranche B（T006–T009）— transactional projection**：实现逐项链接、generated target batching、逐 server MCP、Hook/Prompt adapter，以及 CLI/MCP API 的显式 apply。
3. **Tranche C（T010–T013）— migration cutover**：完成 legacy/.cc-switch 迁移、GUI source-first、仓库 dogfood、旧代码删除和全量门禁。

每个 tranche 都能单独合并；Tranche A 合并后即使后续暂停，也会显著降低现有命令的数据破坏风险。

## 背景与根因

### 当前错误调用链

```text
source::scan_project_root
  ├─ 读取资产
  └─ reconcile_orphan_hook_scripts（读操作中写源，错误被吞）

sync::compute_for_* → SyncAction::Create
  → cli::lifecycle::execute_all_actions
  → materialize::deploy
  ├─ 旧 symlink 被删除
  ├─ 普通目标可被直接删除
  └─ 创建独立硬拷贝

status / doctor / GUI platform_scan
  ├─ 分别使用不同状态算法
  ├─ content-equal 有时叫 linked、有时叫 synced
  └─ doctor 可报告 0 问题，而 status 同时报告 wrong_source/wrong_type

uninstall / retract
  ├─ materialize::retract 可删除普通文件/目录
  └─ MCP 可删除整份平台配置
```

### 文件与函数级根因

| 根因 | 当前位置 | 影响 |
| --- | --- | --- |
| 强制硬拷贝、无事务覆盖 | `crates/ai-config-core/src/materialize.rs::deploy` | 产生多份事实、失败后可能留下半份目标 |
| 无 marker 普通目标也可删除 | `materialize.rs::retract`、`asset_ops::remove_native_platform_copy` | 无法证明所有权仍会删除 |
| 内容相等误作 managed | `materialize.rs::check` | 外部安装可被 retract/uninstall |
| link helper 直接替换普通目标 | `crates/ai-config-core/src/link.rs::link/remove_dest_path` | 错源或 foreign 可被覆盖 |
| status/doctor/list 扫描有副作用 | `source.rs::scan_project_root`、`paths.rs::discover_global_asset_root` | 声称只读但可能迁移/播种/写 hook |
| 项目 MCP 丢失 deploy_base | `crates/ai-config-cli/src/lifecycle.rs::execute_all_actions` | project sync 可写到 HOME |
| MCP 整文件模型与逐 server 模型并存 | `mcp_json.rs`、`template.rs`、`mcp.rs` | overlay、secrets、retract 语义冲突 |
| Codex/Claude adapter 过期 | `platform.rs::{CodexAdapter,ClaudeAdapter}` | 写到平台不消费的位置 |
| 五平台互拷 | `asset_ops::deploy_from_platform`、`sync_conflict.rs`、GUI toggle | 反向/横向复制扩大状态机 |
| store/daemon 未完成却进入产品语义 | `ai-config-store/src/schema.rs`、daemon/bus/watcher | 增加状态源但不能提供可靠所有权 |

### 目标数据流

```text
~/.ai-config (global canonical)
            +
<workspace>/.ai-config (optional workspace defaults)
            +
<repo>/.ai-config (project override)
            │
            ▼
source::resolve_effective_assets  ── read only, keeps provenance
            │
            ▼
platform adapter + ownership classifier
            │
            ▼
ProjectionPlan ── deterministic, serializable, no secret values
            │ explicit --apply / GUI confirm
            ▼
transactional executor
  ├─ DirectLink: skill/rule/command/compatible agent/script
  ├─ Generated: Cursor JSON / Codex TOML / Claude JSON / Hermes YAML
  ├─ ExternalDirectory: Hermes skills.external_dirs
  └─ CopyFallback: explicit Windows fallback only
```

## Technical Context

| 项 | 值 |
| --- | --- |
| Language | Rust workspace edition 2021；TypeScript/React GUI |
| 主要 Crate | `ai-config-core`、`ai-config-store`、`ai-config-cli`、`ai-config-gui` |
| 关键现有模块 | `source`、`paths`、`platform`、`link`、`materialize`、`sync`、`asset_ops`、`platform_scan`、`mcp_json`、`template`、`hook_adapter` |
| 新模块 | `projection/*`、`mcp/*`、`migration`、store `projection_repo` |
| 测试 | Core unit/integration + CLI integration + GUI Vitest + Tauri Rust tests |
| 平台 | Cursor / Codex / Claude / Hermes；macOS/Linux symlink，Windows 显式 fallback |
| 官方契约复核日期 | 2026-07-12；记录在 `docs/reference/platform-contracts.md` |

## Constitution Check

- [x] 业务判断、规划和 apply 逻辑仅在 `ai-config-core`。
- [x] CLI/MCP/GUI 仅传参、确认和展示，不复制 planner/executor。
- [x] 平台差异收敛在 adapter 与 `mcp/*` renderer。
- [x] 所有 destructive 操作均有所有权证明、precondition 和回滚。
- [x] 计划按 TDD 顺序组织，末项包含完整验证。
- [x] 真实用户资产与 secret 不进入工具仓。
- [x] 修订 constitution 中“直接替换、不备份”的旧铁律后再切换实现。

## 影响面与目标文件结构

### Core projection 边界

```text
crates/ai-config-core/src/projection/
├── mod.rs          # 对外 re-export；不含业务实现
├── model.rs        # ProjectionId/Mode/State/Action/Plan/Report
├── source.rs       # global/workspace/project effective resolver
├── fingerprint.rs  # 文件、目录、symlink 的稳定 fingerprint
├── ownership.rs    # 只读状态分类；content equality 与 ownership 分离
├── planner.rs      # effective assets × adapters → deterministic plan
├── executor.rs     # precondition、锁、backup、atomic apply、rollback
└── migration.rs    # legacy/.cc-switch inventory、plan、rollback
```

### MCP 边界

```text
crates/ai-config-core/src/mcp/
├── mod.rs
├── source.rs       # mcp/servers/*.json、overlay、secret reference validation
├── adapter.rs      # GeneratedConfigAdapter + mutation model
├── cursor_json.rs  # ~/.cursor/mcp.json / project .cursor/mcp.json
├── codex_toml.rs   # ~/.codex/config.toml / project .codex/config.toml
├── claude_json.rs  # ~/.claude.json / project .mcp.json
└── hermes_yaml.rs  # ~/.hermes/config.yaml
```

### 文件变更表

| Crate/范围 | 文件 | 变更类型 | 单一职责 |
| --- | --- | --- | --- |
| core | `src/projection/*.rs` | Create | 统一投影模型、规划、执行和迁移 |
| core | `src/mcp/*.rs` | Create | 逐 server source 与四平台 renderer |
| core | `src/platform.rs` | Refactor | 只定义 capability/target，不写文件 |
| core | `src/source.rs`, `src/paths.rs`, `src/workspace.rs` | Refactor | 纯读取扫描与三层 overlay |
| core | `src/link.rs` | Refactor | 仅安全建立/删除精确链接 |
| core | `src/hook_adapter.rs`, `src/hook.rs` | Refactor | 生成具名 Hook mutation，扫描零写入 |
| core | `src/asset_ops.rs`, `src/platform_scan.rs` | Refactor | 显式 import 与统一 ProjectionState |
| core | `src/materialize.rs`, `src/sync_conflict.rs` | Delete late | legacy 迁移完成后删除 |
| store | `src/projection_repo.rs`, `src/schema.rs` | Create/Modify | 可选 ledger；丢失时安全降级 |
| CLI | `src/projection.rs`, `src/migration.rs` | Create | core plan/apply 的薄封装 |
| CLI | `src/main.rs`, `src/lifecycle.rs`, `src/mcp.rs`, `src/agent_api.rs`, `src/serve.rs` | Modify | 移除硬拷贝与手写平台循环 |
| GUI Rust | `src/command_bridge.rs`, `src/lib.rs` | Modify | 暴露 plan/apply/import/retract Tauri commands |
| GUI TS | `src/types.ts`, `api/tauriAssets.ts`, `hooks/useAssetOperations.ts` | Modify | 使用统一状态和计划 |
| GUI UI | asset components + conflict modal | Modify | source-first 操作，不再平台互拷 |
| docs | PRD/ARCHITECTURE/DESIGN/README/HERMES/CHANGELOG/constitution | Modify | 冻结新产品语义 |
| repo | `.ai-config/*`, `.gitignore`, generated platform dirs | Migrate/Delete | 本仓 dogfood canonical source，去除实体副本 |

## 核心 API / 类型

现有 `AssetKind` 增加 `Prompt`；source path 为 `prompts/<name>.md`，使入口提示词与其他资产进入同一 overlay、状态和 plan，而不是由模板旁路管理。

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum AssetKind {
    Skill,
    Rule,
    Mcp,
    Agent,
    Command,
    Prompt,
    Hook,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum ProjectionSurface {
    Platform(PlatformId),
    ProjectEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ProjectionId {
    pub scope_key: String,
    pub kind: AssetKind,
    pub name: String,
    pub surface: ProjectionSurface,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceLayer {
    Global,
    Workspace,
    Project,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionMode {
    DirectLink,
    GeneratedJson,
    GeneratedToml,
    GeneratedYaml,
    ExternalDirectory,
    CopyFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionState {
    Missing,
    ManagedLink,
    ManagedGenerated,
    Copied,
    Equivalent,
    Foreign,
    Conflict,
    Drifted,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectiveAsset {
    pub kind: AssetKind,
    pub name: String,
    pub source_path: Utf8PathBuf,
    pub layer: SourceLayer,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRef {
    pub layer: SourceLayer,
    pub absolute_path: Utf8PathBuf,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentScope {
    User,
    Workspace,
    Project,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionRequest {
    pub source_roots: SyncRoots,
    pub deployment_scope: DeploymentScope,
    pub deploy_base: Utf8PathBuf,
    pub platforms: Vec<PlatformId>,
    pub operation: ProjectionOperation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionTarget {
    pub path: Utf8PathBuf,
    pub entry_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PathFingerprint {
    pub entry_type: FingerprintType,
    pub digest: Option<String>,
    pub link_target: Option<Utf8PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionMember {
    pub id: ProjectionId,
    pub source: Option<SourceRef>,
    pub state: ProjectionState,
    pub reason_code: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedEntryIntent {
    pub projection_id: ProjectionId,
    pub domain: GeneratedIntentDomain,
    pub entry_key: String,
    pub operation: GeneratedEntryOperation,
    pub source: Option<SourceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneratedIntentDomain {
    Mcp,
    Hook,
    ExternalDirectory,
    PromptWrapper,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneratedEntryOperation {
    Upsert,
    Remove { expected_fingerprint: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportOnlyKind {
    Conflict,
    Unsupported,
    OrphanCandidate,
    MissingDependency,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProjectionActionKind {
    CreateLink,
    RemoveLink { expected_source: Utf8PathBuf },
    ApplyGeneratedBatch { container_renderer: String, intents: Vec<GeneratedEntryIntent> },
    CopyFallback,
    AdoptEquivalent,
    CleanupOrphan { ownership_fingerprint: String },
    Noop,
    ReportOnly { report: ReportOnlyKind },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionAction {
    pub action_id: String,
    pub members: Vec<ProjectionMember>,
    pub mode: Option<ProjectionMode>,
    pub target: Option<ProjectionTarget>,
    pub target_precondition: Option<PathFingerprint>,
    pub action: ProjectionActionKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionPlan {
    pub schema_version: u32,
    pub plan_digest: String,
    pub actions: Vec<ProjectionAction>,
    pub summary: ProjectionPlanSummary,
}

pub enum LedgerMutation {
    Upsert(ProjectionRecord),
    Remove(ProjectionId),
}

pub trait ProjectionLedger: Send + Sync {
    fn get(&self, id: &ProjectionId) -> Result<Option<ProjectionRecord>, CoreError>;
    fn get_many(&self, ids: &[ProjectionId]) -> Result<Vec<ProjectionRecord>, CoreError>;
    fn apply_batch(&self, mutations: &[LedgerMutation]) -> Result<(), CoreError>;
}

pub struct PlannerContext<'a> {
    pub platform_adapters: &'a PlatformAdapterRegistry,
    pub generated_adapters: &'a GeneratedAdapterRegistry,
    pub ledger: &'a dyn ProjectionLedger,
    pub secret_metadata_provider: &'a dyn SecretMetadataProvider,
    pub runtime_paths: &'a RuntimePaths,
}

pub struct ExecutorContext<'a> {
    pub source_resolver: &'a dyn SourceResolver,
    pub generated_adapters: &'a GeneratedAdapterRegistry,
    pub secret_provider: &'a dyn SecretProvider,
    pub ledger: &'a dyn ProjectionLedger,
    pub runtime_paths: &'a RuntimePaths,
}

pub enum ApplyAuthorization {
    Standard,
    SelectedActions { plan_digest: String, action_ids: Vec<String> },
}

pub struct ApplyOptions {
    pub authorization: ApplyAuthorization,
}

pub fn resolve_effective_assets(roots: &SyncRoots) -> Result<Vec<EffectiveAsset>, CoreError>;
pub fn build_projection_plan(
    request: &ProjectionRequest,
    context: &PlannerContext<'_>,
) -> Result<ProjectionPlan, CoreError>;
pub fn apply_projection_plan(
    plan: &ProjectionPlan,
    context: &ExecutorContext<'_>,
    options: &ApplyOptions,
) -> Result<ApplyReport, CoreError>;
```

约束：`ProjectionRequest` 可序列化且不携带服务对象；所有只读依赖由调用方通过 `PlannerContext` 注入，core 不自行打开 SQLite 或猜测 runtime roots。ledger 读取失败时 planner 必须带 warning 安全降级：link 仍按真实 target 判断，generated/copy/external-dir 一律不视为 owned；`SecretMetadataProvider` 只暴露 key 是否存在与权限状态，不暴露值。`ProjectionPlan` 只包含 source locator、路径、key、状态、reason、hash 和 action metadata；不得包含渲染后 MCP body、secret value 或平台配置原文。`plan_digest` 由 schema version、稳定 reason code、规范化 action/member/source/target/precondition 计算，不包含本地化展示文案。Apply 必须通过 `SourceResolver` 重读 allowlisted source、复核 source fingerprint，再由 adapter/secret provider 在内存中构造 generated target。

### Platform adapter

```rust
pub struct TargetContext<'a> {
    pub home: &'a Utf8Path,
    pub deploy_base: &'a Utf8Path,
    pub deployment_scope: DeploymentScope,
}

pub enum PlatformCapability {
    Supported { mode: ProjectionMode },
    Unsupported { reason_code: String, reason: String },
}

pub trait PlatformAdapter: Send + Sync {
    fn id(&self) -> PlatformId;
    fn capability(&self, target: &TargetContext<'_>, kind: AssetKind) -> PlatformCapability;
    fn target(
        &self,
        target: &TargetContext<'_>,
        asset: &EffectiveAsset,
    ) -> Result<ProjectionTarget, CoreError>;
}
```

`EffectiveAsset.layer` 只表示“内容来自哪里”；`DeploymentScope/deploy_base` 只表示“投影写到哪里”，两者不得互相推导。比如 project 请求继承 global skill 时，`SourceRef.layer = Global`，但 target 仍必须位于 project root。

### Generated config adapter

```rust
pub trait GeneratedConfigAdapter: Send + Sync {
    fn container_renderer_id(&self) -> &'static str;
    fn inspect_target(
        &self,
        target: &ProjectionTarget,
    ) -> Result<GeneratedTargetSnapshot, CoreError>;
    fn render_batch(
        &self,
        target: &ProjectionTarget,
        intents: &[GeneratedEntryIntent],
        context: &GeneratedRenderContext<'_>,
    ) -> Result<GeneratedMutation, CoreError>;
}
```

Planner 必须先规范化 target path，再按 `target.path` 聚合所有 generated intents；一个目标文件只能绑定一个 container renderer，若多个 domain adapter 对同一路径声明不同 renderer，plan 必须 blocking conflict。MCP、Hook、ExternalDirectory、PromptWrapper 通过 `GeneratedIntentDomain` 合入同一个 `ApplyGeneratedBatch`，因此同一 JSON/TOML/YAML 文件只有一个 precondition、一次 parse/render/backup/rename。`GeneratedMutation` 的 `Debug`、`Serialize` 和 error surface 必须 redacted；中间渲染 bytes 只在 apply 进程内存中短暂存在，最终 bytes 仅出现在目标配置和权限受控的 transaction backup。

## 需求追溯

| FR | 实现位置 | 验收 |
| --- | --- | --- |
| FR-001, FR-002, FR-003, FR-004, FR-005 | `projection/source.rs`、`source.rs`、prompt projection | overlay/provenance/per-server/prompt-entry tests |
| FR-006, FR-007, FR-008, FR-009, FR-010, FR-011, FR-012, FR-013, FR-014 | `platform/*`、`projection/planner.rs`、`link.rs` | platform contract + idempotent direct-link tests |
| FR-015, FR-016, FR-017, FR-018, FR-019, FR-020 | `projection/ownership.rs`、`fingerprint.rs`、executor | foreign/equivalent/drift/read-only/retract safety tests |
| FR-021, FR-022, FR-023, FR-024, FR-025, FR-026 | planner/executor、CLI projection、import | dry-run/stale-plan/report/import integration tests |
| FR-027, FR-028, FR-029, FR-030, FR-031, FR-032 | `mcp/source.rs`、四 renderer、secrets | sentinel secret + permissions + per-server migration tests |
| FR-033, FR-034, FR-035, FR-036, FR-037, FR-038 | `projection/migration.rs`、repo hygiene | legacy/.cc-switch inventory + rollback + Git hygiene tests |
| FR-039 | CLI/MCP/GUI bridges | cross-surface plan JSON contract tests |

| Success criterion | 验收位置 |
| --- | --- |
| SC-001, SC-011 | T003 platform contract fixtures + T013 temp-HOME matrix |
| SC-002, SC-003 | T006 immediate-propagation/idempotency tests + canary |
| SC-004, SC-009, SC-010 | T001 safety tests + T010 mixed migration corpus |
| SC-005, SC-006 | T004/T005 100-run purity checks + 1,000-item benchmark gate |
| SC-007, SC-008 | T008 sentinel/permission tests + T013 captured-output scan |
| SC-012 | T012 repository hygiene test |

## 方案设计

### 1. Source resolution

`source::scan_project_root` 拆为无副作用扫描函数；创建目录、seed、legacy migration 必须移入显式 `init` 或 `migrate --apply`。Resolver 使用 `(AssetKind, name)` 作为 key：先 global，再 workspace，最后 project 覆盖。MCP 以 server name 为 key，不再用 `Option<mcp.json>` 整文件遮蔽；Prompt 扫描 `prompts/*.md`，默认项目入口名为 `AGENTS`。

| 维度 | Global | Workspace | Project/member |
| --- | --- | --- | --- |
| source root | `~/.ai-config` | `<workspace>/.ai-config` | `<repo>/.ai-config` |
| deploy base | `$HOME` | workspace root | repo root |
| 同名优先级 | 低 | 中 | 高 |
| 缺失资产 | — | 继承 global | 继承 workspace/global |
| Hermes project skill | — | unsupported | unsupported，禁止写全局 |
| Hermes generated config | user scope only | unsupported | unsupported，MCP/Hook 禁止写全局 `config.yaml` |

### 2. Platform contract matrix

`spec.md` 的“平台资产契约”是路径、scope、格式和 unsupported 边界的权威来源；本表只记录实现摘要，不得独立扩展平台路径。

| Platform | Skills | Rules | MCP | Agents | Commands | Hooks |
| --- | --- | --- | --- | --- | --- | --- |
| Cursor | user/repo `.agents/skills/<name>`，与 Codex 共享 target | global User Rules 无文件投影；project `.cursor/rules/<name>.mdc` | user/project `.cursor/mcp.json#mcpServers` | user/project `.cursor/agents/<name>.md` | user/project `.cursor/commands/<name>.md` | user/project `.cursor/hooks.json` + hooks unit |
| Codex | user/repo `.agents/skills/<name>`，与 Cursor 共享 target | instruction rule unsupported；项目指导走 `AGENTS.md`，绝不写 `.codex/rules` | user/project `.codex/config.toml#mcp_servers` | user/project `.codex/agents/<name>.toml` | unsupported；仅盘点 legacy `~/.codex/prompts` | user/project `.codex/hooks.json` + hooks unit |
| Claude | user/project `.claude/skills/<name>` | user/project `.claude/rules/<name>.md` | user `~/.claude.json`；project `.mcp.json` | user/project `.claude/agents/<name>.md` | user/project `.claude/commands/<name>.md`（legacy-compatible） | user/project `.claude/settings.json#hooks` + hooks unit |
| Hermes | user `config.yaml#skills.external_dirs`；project unsupported | global unsupported；project 兼容消费 `.cursor/rules` shared target | user `config.yaml#mcp_servers`；workspace/project unsupported | static agent unsupported | unsupported，转为 Skill | user `config.yaml#hooks`；workspace/project unsupported |

Legacy `.codex/skills`、`.codex/mcp.json`、`.claude/mcp.json` 只进入 inventory，不再作为 deploy target。

`Prompt` 不由四个平台各生成一份：planner 为它创建唯一 `ProjectionSurface::ProjectEntry`，把 `<repo>/AGENTS.md` 精确链接到 effective `.ai-config/prompts/AGENTS.md`。各平台 adapter 只声明是否消费该 shared entry；Claude 额外生成最小 `CLAUDE.md` entry。若根入口已是普通文件或其他来源链接，状态为 Equivalent/Foreign/Conflict，必须显式 import/adopt，绝不模板覆盖。

### 3. Ownership and ledger

- 正确 symlink/junction 指向 effective source：`ManagedLink`，即使 ledger 丢失也可证明。
- 普通文件/目录内容相同：`Equivalent`，不可 retract；只有显式 adopt + backup 才可替换。
- 普通文件/目录内容不同且无 ownership：目标状态 `Foreign`，对应 create/update plan 为 blocking `Conflict`。
- 有 ledger/legacy ownership 证据但 link target 或 generated fingerprint 偏离：`Drifted`；没有 ownership 的错源 link 为 `Foreign`，对应 plan 为 blocking `Conflict`。
- 指向 `.cc-switch` 或其他已知外部 root 的 link：`Foreign`/`external_owned` reason。
- 有 ledger record 的 generated entry 且 fingerprint 一致：`ManagedGenerated`。
- 有 record 但目标变化：`Drifted`；默认禁止 update/retract，需显式 repair plan。
- ledger 丢失：generated entry 退化为 `Foreign`，不删除。

SQLite `projections` 表由 `ai-config-store` 实现，但 core 只依赖 `ProjectionLedger` trait；core 测试使用 `MemoryProjectionLedger`。读失败时 planner 安全降级为“不证明 generated ownership”；apply 阶段 `apply_batch` 必须在一个 SQLite transaction 内提交全部 upsert/remove，任何写失败都视为 transaction failure 并触发文件系统回滚，不能静默降级。

### 4. Plan/apply transaction

Dry-run 不创建目录、锁、DB、backup 或日志。`ReportOnly::Conflict` 是 blocking；默认 apply 发现任一 blocking action 时整份计划零写入，用户只能调整选择后重新生成新 plan，不能在 apply 时临时跳过 precondition。`Unsupported`、`OrphanCandidate` 和 `MissingDependency` 是明确的 non-executable skipped item，其中缺失 secret 的 member 不进入 generated batch，不影响不依赖该 key 的 member。`AdoptEquivalent` 也是默认 non-executable candidate，只有 `ApplyOptions` 同时携带当前 plan digest 与该 action_id 时才进入 executable set；未选择 candidate 保持不变。其余 executable actions 按整份计划全原子执行：

1. 获取 `~/.config/ai-config/apply.lock` 排他锁。
2. 验证 plan schema、digest、每条 source/target precondition。
3. 在 `~/.config/ai-config/backups/<transaction-id>/` 建立 0700 transaction 目录。
4. 快照所有待修改 target 的文件、目录、symlink target、mode 和 hash；含 secret 的文件强制 0600。
5. 在目标同目录创建临时 link/file/tree。
6. 原子 rename；逐项验证状态。
7. 所有文件 action 成功后调用 ledger `apply_batch`，由 store 在一个 SQLite transaction 内提交。
8. 文件或 ledger 任一步失败，按逆序恢复文件快照；ledger batch 自身必须全回滚。文件回滚也校验 post-apply fingerprint。

成功报告使用 `changed/unchanged/skipped`；预检阻塞使用 `conflict/not_applied`；执行期失败使用 `failed/rolled_back/not_applied`。只有最终仍存在的 mutation 可计入 `changed`，回滚成功的项不得显示为成功。CLI 的 exit 3 表示“存在 blocking conflict 或事务失败”，不表示允许留下部分写入。

CLI breaking contract：`install/sync/uninstall/migrate` 默认只输出 plan；只有显式 `--apply` 才写入。MCP tool 的 `apply` 参数默认 `false`。GUI 必须显示 plan modal 并确认。

Adopt/takeover 不提供平台级或来源级总开关。用户必须提交当前 `plan_digest` 并逐个选择 `action_id`；executor 再校验该 action 仍为 Equivalent 或经 import 后可安全接管，先备份目标再替换。内容不同的 foreign 不能直接 adopt；`.cc-switch` 也只能逐项选择。

### 5. MCP canonical model and rendering

Canonical layout：

```text
<asset-root>/mcp/servers/<name>.json
```

```json
{
  "name": "example",
  "enabled": true,
  "targets": ["cursor", "codex", "claude", "hermes"],
  "transport": "stdio",
  "config": {
    "command": "example-mcp",
    "args": [],
    "env": { "API_TOKEN": "${EXAMPLE_API_TOKEN}" }
  }
}
```

- `toml_edit = 0.25`（当前 lock 0.25.12）用于 Codex，保留 comments、unknown tables 和非 MCP 配置。
- `yaml-edit = 0.2.3` 用于 Hermes lossless edit；semantic parse 仍用于结果校验。
- Cursor/Claude 使用 JSON object-level merge，保留非 ai-config server 和顶层字段。
- Source migration 默认 dry-run；`--extract-secrets --apply` 才把 literal env/header 写入 0600 secrets 文件并替换为引用。
- env key 直接作为 secret key；header 生成 `<SERVER>_<HEADER>` uppercase key。派生 key 冲突或现有值不同则整项 conflict，不输出值。
- URL 中嵌入凭据不自动抽取，只 redacted 报告并阻止迁移。

### 6. Hooks and entry prompts

新增 `AssetKind::Prompt`，canonical 正文位于 source layer 的 `prompts/*.md`。项目默认把 effective `prompts/AGENTS.md` 以 `ProjectionSurface::ProjectEntry` 逐项链接到 root `AGENTS.md`；Cursor、Codex、Hermes 共同消费该入口。Claude 只生成最小 `CLAUDE.md` wrapper/import；平台专属内容放 canonical rules 或独立 prompt，再由 adapter 引用，禁止三份完整正文。

Hook 暂保留 canonical `hooks.json + hooks/<asset>` 布局，避免同时做第二次 source schema 迁移；脚本/bundle 逐资产链接，平台 hooks/settings/config 由 adapter 产生具名 mutation。移除 scan 中的 reconcile；显式 migration 才登记 orphan。

### 7. GUI source-first interaction

- ai-config source icon 永远只读，不再可“retract source”。
- `ManagedLink/ManagedGenerated`：可 plan update/retract。
- `Missing`：可 plan create。
- `Equivalent`：只允许基于 plan digest + action_id 显式 adopt with backup。
- `Foreign/Conflict/Drifted`：只允许 diff、import 或 repair plan，不允许直接覆盖。
- 删除平台到平台 copy；平台视图唯一反向操作为“Import to ai-config”。
- 所有批量按钮先展示 plan summary、conflicts、backup 路径和 secret key names。

## 边界与风险

| 场景 | 期望行为 | 计划测试 |
| --- | --- | --- |
| content-equal 普通副本 | Equivalent；normal apply 零写入，仅 selected action-id 可备份后 adopt | `content_equality_is_equivalent_not_managed` |
| `.cc-switch` symlink | Foreign/external_owned | `classifies_cc_switch_symlink_as_external_owned` |
| wrong-source symlink | 有 ownership 为 Drifted；无 ownership 为 Foreign；两者 plan 均 blocking Conflict | `wrong_source_link_state_depends_on_ownership` |
| ledger 丢失 | link 可重建 ownership；generated 不删除 | `missing_ledger_degrades_generated_to_foreign` |
| status 运行 100 次 | hash/mtime/state 零变化 | `status_is_strictly_read_only` |
| project local asset | 仍继承 workspace/global 其他项 | `member_local_assets_do_not_drop_workspace_assets` |
| project MCP | 有 project native config 的平台只写 repo；Hermes 为 Unsupported | `project_mcp_targets_stay_under_project_scope` |
| stale plan | apply 拒绝 | `apply_rejects_stale_plan` |
| apply 中途失败 | 自动回滚已改 target | `apply_failure_rolls_back_prior_actions` |
| Codex user skill | `.agents/skills` link | `codex_user_skills_use_agents_skills` |
| Claude user MCP | merge `~/.claude.json` | `claude_user_mcp_uses_claude_json` |
| Hermes global skill | external_dirs，不复制 | `hermes_uses_external_dirs_for_global_skills` |
| Hermes project skill/MCP/Hook | Unsupported，不污染 global config | `hermes_project_generated_assets_never_touch_global_config` |
| MCP literal secret | redacted conflict or explicit extraction | `literal_secret_never_serializes_in_plan` |
| uninstall | 仅删 managed link/entry | `uninstall_preserves_all_foreign_content` |
| Windows link failure | explicit copied/unsupported | `windows_fallback_never_reports_managed_link` |
| tracked platform projections | CI fail | `repository_contains_no_tracked_platform_copies` |

**非目标:** 双向实时同步；平台 Provider/账号设置；云同步；OS Keychain；自动 token rotation；未经授权重写 Git 历史；本 feature 内完善 daemon/watch/event bus；把整个平台 skills 根链接到 canonical 根。

## 测试策略

1. **Core unit**：model serde、overlay、fingerprint、ownership、adapter target、plan determinism、executor rollback。
2. **Core integration**：临时 HOME + global/workspace/project + legacy/.cc-switch/plugin fixture；不访问真实 HOME。
3. **CLI integration**：默认 dry-run、`--apply`、JSON schema、exit code、stale plan、uninstall safety、migration rollback。
4. **MCP sentinel**：唯一 secret 值走 plan/apply/status/doctor/error/JSON/MCP API，扫描所有输出不得出现。
5. **GUI unit**：状态到按钮行为、plan modal、import-only reverse flow。
6. **Tauri Rust**：bridge 仅调用 core；没有 materialize/platform-copy 逻辑。
7. **Platform fixtures**：Cursor JSON、Codex TOML、Claude user/project JSON、Hermes YAML，断言 comments/unknown fields/foreign entries 保留。
8. **Repository hygiene**：Git tracked paths 不含平台 skill 副本、generated hooks/MCP 或 `/Users/<name>` 绝对路径。
9. **Manual canary**：发布前只迁移一个无 secret 的非关键 skill，四平台新会话确认发现；随后才扩大。

## Rollout Gates

| Gate | 条件 | 允许下一步 |
| --- | --- | --- |
| G0 Safety | destructive fallback 封死；tracked MCP 仅 placeholder；用户确认外部 token rotation | inventory-only |
| G1 Inventory | 真实 HOME 前后 tree hash 完全一致；plan deterministic | temp HOME apply |
| G2 Direct link | fault injection rollback 全绿；单 skill canary 四平台可发现 | 批量普通资产 |
| G3 Ordinary assets | skills/rules/agents/commands 无 foreign 修改；第二次 sync noop | MCP/Hook |
| G4 Generated configs | 四格式可解析；foreign 保留；sentinel 不泄露；MCP/Hook 冒烟 | GUI default |
| G5 GUI | 所有 UI 写入都经过 plan/confirm/apply | legacy cleanup |
| G6 Cleanup | 连续两个版本 inventory 无 legacy managed copies | 删除 legacy modules |

## 回滚

- 每个 T00x 为独立 commit/PR review 单元；任一 tranche 可整体 revert，不依赖后续 tranche。
- 真实 HOME cutover 前保留 legacy 代码的 inventory/read 支持，但关闭其 apply。
- 每次 apply 都生成 transaction backup + manifest；`ai-config migrate rollback <transaction-id>` 仅在 post-apply fingerprint 未漂移时自动恢复。
- 目标在 apply 后被用户修改时 rollback 返回 `rollback_conflict`，不得覆盖；force restore 不在本 feature 自动开放。
- MCP source schema migration 保留原 `mcp.json` 备份至少一个发布周期；不自动改写 Git 历史。
- ledger 损坏时删除 ledger 即可安全降级；系统不根据 content equality 推断可删除 ownership。

## Todos

### T001 — P0：冻结破坏性路径和 secret 泄露面

**Files:**

- Modify: `crates/ai-config-core/src/materialize.rs`
- Modify: `crates/ai-config-core/src/link.rs`
- Modify: `crates/ai-config-core/src/asset_ops.rs`
- Modify: `crates/ai-config-core/src/asset_scope.rs`
- Modify: `crates/ai-config-core/src/mcp_json.rs`
- Modify: `crates/ai-config-core/src/source.rs`
- Modify: `crates/ai-config-core/src/paths.rs`
- Modify: `crates/ai-config-core/src/doctor.rs`
- Modify: `crates/ai-config-cli/src/lifecycle.rs`
- Test: `crates/ai-config-cli/tests/deploy_retract.rs`
- Test: core module tests adjacent to modified code

**Interfaces:** 保持现有 CLI 表面；只把 destructive fallback 改为 fail-closed，为后续 planner 留安全基线。

- [x] **T001.1 Write failing safety tests**：新增 `retract_refuses_unowned_regular_directory`、`uninstall_preserves_unmanaged_skill_directory`、`uninstall_preserves_platform_mcp_file_and_foreign_servers`、`uninstall_preserves_same_name_foreign_mcp_server`、`doctor_is_read_only`、`project_mcp_uses_project_deploy_base`。测试先快照目标 tree hash 和 mode，再调用命令，断言 foreign 零变化。
- [x] **T001.2 Verify RED**：运行 `cargo test -p ai-config-core retract_refuses_unowned -- --nocapture` 与 `cargo test -p ai-config-cli --test deploy_retract`；预期至少上述新测试因普通目标被删除、MCP 整文件删除或 scope 错误而失败，且测试过滤结果必须显示至少 1 个用例执行。
- [x] **T001.3 Implement fail-closed guards**：`materialize::retract` 只接受精确 symlink 或 legacy marker；删除 `remove_native_platform_copy`/`remove_platform_copy_best_effort` 的硬删除 fallback；在 T002 ledger/具名 ownership 可用前，MCP retract/uninstall 默认拒绝删除，只有明确 legacy marker 证据才允许移除具名 server，绝不凭同名判断；`RenderMcp` 使用 `ctx.deploy_base`；doctor 移除 `--materialize` 写路径。
- [x] **T001.4 Make read APIs pure**：从 `scan_project_root` 移除 hook reconcile，从 `discover_global_asset_root` 分离 ensure/seed/migrate；list/status/doctor 只调用纯 read variant。
- [x] **T001.5 Add secret diagnostics without values**：doctor 仅报告 asset-root 中 literal env/header 数、历史 `mcp-secrets.env*` 路径和 mode；不得读取后输出值。增加 sentinel test 扫描 stdout/stderr/JSON。
- [x] **T001.6 Verify GREEN**：运行 `cargo test -p ai-config-core -p ai-config-cli`；预期全部通过，且 `git status --short` 只包含本任务源码/测试变更。
- [x] **T001.7 Commit checkpoint**：`git commit -m "fix(core): 封死未托管资产误删与只读写入"`。
- [x] **T001.8 Audit regression closure**：补充并先验证 RED：删除 source 时保留未托管普通目录、第三方软链接和无 ownership 的同名平台 MCP；`materialize::retract` 无 expected source 时拒绝任何软链接，只有 canonical-equivalent 的精确 legacy symlink 可收回。验证：`cargo test -p ai-config-core source_delete_preserves_ -- --nocapture`、`cargo test -p ai-config-core retract_ -- --nocapture`、`cargo test -p ai-config-core -p ai-config-cli -- --test-threads=1`。
- [x] **T001.9 Follow-up commit checkpoint**：仅提交 T001.8 的安全回归、最小修复和计划记录；不得夹带当前 GUI 或其它 feature 的工作树改动。证据：`ba45589 fix(core): 补齐源删除安全边界`。
- [x] **T001.10 Hook ownership regression closure**：先复现 Cursor 外部 Hook 仅有第三方 binding 时，`hook_adapter::retract` 仍会删除脚本；现在未发现 `ai-config` managed binding 就整体跳过，配置与脚本都保持不变。验证：`retract_preserves_external_hook_script_without_managed_binding` RED→GREEN，随后 `cargo test -p ai-config-core -p ai-config-cli` 全绿。

### T002 — 统一 projection model、fingerprint 与 optional ledger

**Files:**

- Create: `crates/ai-config-core/src/projection/mod.rs`
- Create: `crates/ai-config-core/src/projection/model.rs`
- Create: `crates/ai-config-core/src/projection/fingerprint.rs`
- Create: `crates/ai-config-core/src/projection/ownership.rs`
- Create: `crates/ai-config-store/src/projection_repo.rs`
- Modify: `crates/ai-config-core/src/lib.rs`, `model.rs`, `error.rs`
- Modify: `crates/ai-config-store/src/lib.rs`, `schema.rs`
- Modify: `Cargo.toml`, `crates/ai-config-core/Cargo.toml` (`sha2 = "0.10"`, `hex = "0.4"`)

**Produces:** 本计划“核心 API/类型”中的全部 model（含 `AssetKind::Prompt`、`ProjectionSurface`）；`ProjectionLedger` trait；`SqliteProjectionRepo`；`MemoryProjectionLedger` test double。

- [ ] **T002.1 Write model/ownership tests**：覆盖 Prompt/ProjectionSurface serde、ProjectionId 稳定性、目录 digest 包含 dotfiles 且只排除 legacy marker、正确 link 为 ManagedLink、等内容普通副本为 Equivalent、有 ownership 的错源 link 为 Drifted、无 ownership 的错源 link 为 Foreign、ledger 缺失 generated 为 Foreign。
- [x] **T002.1a Projection fingerprint/ownership foundation**：已先验证目录 semantic digest 包含 dotfiles、仅忽略根 legacy marker、忽略 mode；`ProjectionSurface::SharedTarget` 防止 Cursor/Codex 共享 `.agents/skills` 产生两份可独立收回的 ownership；已覆盖 exact link→ManagedLink、equal copy→Equivalent、wrong link→Foreign/Drifted、missing→Missing。`Prompt` 的 legacy enum 迁移保持在协调门禁之后，避免在旧 hard-copy lifecycle 中产生路径或写入语义。
- [x] **T002.1b Ledger contract and SQLite persistence**：`ProjectionLedger` 作为 core trait，`MemoryProjectionLedger` 与 SQLite `projection_ledger` 都拒绝同一 batch 重复 mutation；SQLite 在单一 transaction 内提交，批次验证失败时不留下部分写入。generated target 仅在 id、mode 和 target fingerprint 都匹配账本时为 `ManagedGenerated`，账本缺失仍为 `Foreign`。
- [x] **T002.1c Generic semantic fingerprint**：补齐单文件/目录/软链接的 `path_content_digest`；规则、agent、command 与后续 prompt 的普通文件对比不包含目标文件名，内容相等只能得到 `Equivalent`，不会成为可收回 ownership。软链接只记录自身 target，绝不跟随未知外部目录。
- [x] **T002.1d Prompt cross-layer migration gate**：只读盘点已确认 Prompt 会经过 core `asset_ops` 的 deploy/retract/read/save/delete、`sync`、`platform_scan`、CLI lifecycle 与 GUI 操作分支。`AssetKind::Prompt` 进入旧枚举时必须在全部旧 lifecycle 分支显式 fail-closed 为 `UnsupportedAsset`，不得扫描、导入、硬拷贝或写任何平台路径；只有 T008 的 canonical `prompts/AGENTS.md` + ProjectEntry projection 与 T009 public lifecycle 同时完成后，才可解除该门禁。当前这些共享文件有并行未提交变更，故本 checkpoint 只冻结约束，不重写其工作树。
- [x] **T002.1e Planner precondition model**：新增 `SourceLayer`、`DeploymentScope`、`ProjectionTarget` 和 `PathFingerprint`；后者将 semantic digest 与 entry type、link target、Unix mode 分离，已经验证“内容相等但 mode 变动”不会改变 Equivalent 判定，却会阻止旧 plan 继续 apply。
- [x] **T002.1f Missing/rollback/provenance regressions**：`path_fingerprint` 对不存在目标返回 `FingerprintType::Missing`（不是 IO error），为 create-link plan 保留可重验前置条件；补齐 `EffectiveAsset → SourceRef` provenance、三种 `ProjectionSurface` identity round-trip、不可读/账本 mode 不匹配目标保持 `Foreign`，以及 SQLite batch 第二项真实写失败时第一项也完整回滚的临时数据库证据。
- [ ] **T002.2 Verify RED**：`cargo test -p ai-config-core projection:: -- --nocapture`，预期因模块/类型不存在失败。
- [ ] **T002.3 Implement model and fingerprints**：使用 `sha2` 对排序后的相对路径、entry type 和 bytes 计算 semantic digest；mode 仅作为严格 apply precondition，不能混入 Equivalent 判定；symlink fingerprint 记录 target，不跟随未知 root 外链。
- [ ] **T002.4 Implement ownership classifier**：签名 `classify_projection(expected, actual, record) -> ProjectionState`；content comparison 与 ownership 分支分离，任何不确定状态返回 Foreign/Conflict 而非 managed。
- [ ] **T002.5 Implement optional ledger**：新增 `projections(scope_key, kind, name, surface, mode, source_path, target_path, entry_key, fingerprint, applied_at)`；实现 read APIs 与单 SQLite transaction 的 `apply_batch`；core 不直接依赖 store；read error 不得让 classifier 推断可删除，batch 任一 mutation 失败不得留下部分 record。
- [ ] **T002.6 Verify**：`cargo test -p ai-config-core projection::`、`cargo test -p ai-config-store projection_repo`。
- [ ] **T002.7 Commit checkpoint**：`git commit -m "feat(core): 建立投影所有权与健康状态模型"`。

### T003 — 官方平台契约与纯 target adapter

**Files:**

- Refactor: `crates/ai-config-core/src/platform.rs` into `src/platform/{mod,cursor,codex,claude,hermes}.rs`
- Create: `docs/reference/platform-contracts.md`
- Modify: `docs/reference/vercel-skills-agent-paths.md`
- Test: platform module tests

**Consumes:** `EffectiveAsset`、`DeploymentScope`、`ProjectionMode`、`ProjectionTarget`。

**Produces:** `TargetContext`、`PlatformCapability`、新 `PlatformAdapter` trait。

- [ ] **T003.1 Write failing contract tests in fixed asset order**：按 `Skills → Rules → MCP → Agents → Commands → Hooks` 逐类覆盖 `spec.md` 中四个平台的 global/project target、格式、projection mode、shared target、trust 与 unsupported 边界；至少保留 `codex_user_skills_use_agents_skills`、`cursor_and_codex_share_one_agents_skill_target`、`codex_instruction_rule_never_uses_execution_rules`、`codex_mcp_uses_config_toml`、`claude_user_mcp_uses_claude_json`、`claude_project_mcp_uses_project_dot_mcp_json`、`codex_agent_uses_toml_not_subagents`、`codex_command_is_unsupported_not_codex_commands`、`hermes_uses_external_dirs_for_global_skills`、`hermes_project_skill_is_unsupported`、`hermes_project_mcp_is_unsupported_and_never_touches_global_config`、`hermes_project_hook_is_unsupported_and_never_touches_global_config`、`legacy_paths_are_inventory_only`、`platform_target_cannot_escape_scope`、`global_source_project_projection_stays_under_project_root`、`workspace_or_project_generated_target_never_escapes_deploy_base`。每一类先记录 RED 证据，上一类未通过不得进入下一类。
- [x] **T003.1a Skills contract RED→GREEN**：新增完全独立于旧 hard-copy lifecycle 的 `projection::platform_adapter`，验证 Codex user `.agents/skills`、Cursor/Codex 同一 project physical target 与 `SharedTarget` identity、Claude native `.claude/skills`、Hermes user `config.yaml#skills.external_dirs` 和 Hermes project scope `unsupported`。所有用例只做纯路径/能力计算，未创建链接或目录；旧 `platform.rs` 保持兼容入口，等待 T009 统一切换。
- [x] **T003.1b Rules contract RED→GREEN**：Codex instruction Rule 明确 `unsupported` 并禁止 `.codex/rules`；Cursor user Rule 无稳定文件目标而 `project .cursor/rules/<name>.mdc` 为 direct-link candidate；Hermes project 复用该 Cursor physical target（`SharedTarget`），user scope 明确不写 `SOUL.md`；Claude user/project Rule 指向 `.claude/rules/<name>.md` 且声明 `GeneratedMarkdown`，不把 canonical `.mdc` 伪装为直接链接。
- [x] **T003.1c MCP contract RED→GREEN**：MCP 一律声明为具名 `Generated` 容器 entry：Cursor user/project `.cursor/mcp.json#mcpServers.<name>`、Codex user/project `.codex/config.toml#mcp_servers.<name>`、Claude user `~/.claude.json` 与 project `.mcp.json` 的 JSON entry、Hermes user `config.yaml#mcp_servers.<name>`；Hermes project MCP 明确 `unsupported`，无法经 fallback 改写用户全局配置。
- [x] **T003.1d Agents contract RED→GREEN**：Cursor/Claude agent 都声明为 `.md` `GeneratedMarkdown`（保留平台 schema 转换空间），Codex 固定 `.codex/agents/<name>.toml` `GeneratedToml` 并禁止 legacy subagents 目录；Hermes user/project 都明确无静态 agent directory、fail-closed `unsupported`。
- [x] **T003.1e Commands contract RED→GREEN**：Cursor/Claude Command 都是 user/project `.cursor|.claude/commands/<name>.md` 的逐文件 `DirectLink`；Codex 明确 `unsupported`，历史 `.codex/prompts` 仅供后续 inventory、绝不写 `.codex/commands`；Hermes Command 明确 `unsupported` 并提示改用 Skills。所有分支均为纯 capability 计算，不触发旧 hard-copy 或 GUI 操作。
- [x] **T003.1f Hooks contract RED→GREEN**：新增复合 `HookCapability`，强制一个 Hook 同时拥有具名 generated binding 和逐项 direct-link 脚本单元，禁止半投影。Cursor `.cursor/hooks.json` + `.cursor/hooks/`、Codex 固定 `.codex/hooks.json`（不写 inline TOML）+ `.codex/hooks/`、Claude `settings.json` + `.claude/hooks/`、Hermes user `config.yaml` + `.hermes/hooks/` 都有 target/format 断言；Hermes project 明确 `unsupported`。所有 adapter 入口同时拒绝空名、`.`、`..`、斜杠、反斜杠与 NUL，不能借资产名越出 deploy scope。
- [x] **T003.1g 需求文档防偏航索引**：新增 `docs/reference/platform-contracts.md`，以 Spec 008 为唯一权威链接，按同一固定资产顺序收录 global/project target、格式、投影模式、legacy inventory 与 unsupported 边界；`docs/README.md` 与旧 `vercel-skills-agent-paths.md` 明确标注硬拷贝/第三方路径只用于历史盘点，禁止据此修改新 adapter。
- [x] **T003.1h 补齐 user/project 目标矩阵回归**：在原有 fixed-order RED→GREEN 用例基础上，补充 user MCP 三个平台容器、user Agent 三个平台格式及 Cursor user/Claude project Command 的精确 target 断言；`projection::platform_adapter` 34 项通过。其余 trust/source-scope/workspace policy 仍是 T003 未完成的显式缺口，不能据此标记 T003 完成。
- [x] **T003.1i Workspace deploy-root contract RED→GREEN**：冻结 workspace 不是 HOME fallback：Cursor Rule 与 Claude MCP 使用 workspace root 的 project-compatible target，Hermes workspace MCP/Hook 仍明确 `unsupported`。先验证旧 Cursor Rule 无契约的 RED，再以最小 scope guard 调整为非 user target；`projection::platform_adapter` 35 项通过。
- [ ] **T003.2 Verify RED**：`cargo test -p ai-config-core platform::`；预期 Codex/Claude 当前旧路径断言失败。
- [ ] **T003.3 Implement scoped adapters**：平台对象只返回 capability/target；不创建目录、不读取 secret、不写配置。Codex rules 不映射 `.codex/rules`；Hermes workspace/project skills、MCP、Hook 不再写全局。
- [ ] **T003.4 Record official contracts**：文档写明 URL、verified date、global/project path、format、symlink guarantee、legacy discovery path；测试 fixture 与文档表保持一致。
- [ ] **T003.5 Verify**：`cargo test -p ai-config-core platform::`。
- [ ] **T003.6 Commit checkpoint**：`git commit -m "feat(platform): 对齐四平台官方资产契约"`。

### T004 — 纯 source resolver 与三层 overlay

**Files:**

- Create: `crates/ai-config-core/src/projection/source.rs`
- Modify: `crates/ai-config-core/src/source.rs`, `paths.rs`, `workspace.rs`
- Test: `crates/ai-config-core/tests/source_overlay.rs`

**Produces:** `resolve_effective_assets(&SyncRoots) -> Vec<EffectiveAsset>`，排序键固定为 `(kind, name)`。

- [x] **T004.1 Write failing overlay tests**：global fills workspace/project missing、workspace overrides global、project overrides both、member 有本地一项仍继承其余资产、MCP 同名逐 server 整项覆盖、Prompt 同名覆盖、hook bundle 覆盖、每项保存 provenance、project target 不越界。
- [x] **T004.2 Verify RED**：在 `projection::source` 最初只有 `todo!` 的实现下运行定向测试，确认 resolver 尚未实现后再进入最小实现。
- [x] **T004.3 Implement pure scanners**：skills/rules/agents/commands/prompts/hooks/MCP 每类扫描只读；MCP 同时读取新 per-server source，legacy monolith 只作为 migration candidate，不自动删除。
- [x] **T004.4 Implement deterministic resolver**：按 global→workspace→project 覆盖 map，保留被覆盖来源用于 diff，但只输出一个 effective asset。
- [x] **T004.5 Verify read-only invariant**：对临时根运行 resolver 100 次，断言 tree digest 不变；resolver 不创建目录、不写 SQLite。
- [x] **T004.6 Verify**：`cargo test -p ai-config-core projection::source::tests`（4 passed）与 `cargo test -p ai-config-core projection::`（63 passed）。
- [x] **T004.7 Commit checkpoint**：已提交 `1f2aad0 feat(core): 新增三层 source resolver`。

### T005 — 只读 Projection Planner

**Files:**

- Create: `crates/ai-config-core/src/projection/planner.rs`
- Modify: `crates/ai-config-core/src/projection/mod.rs`
- Compatibility modify: `crates/ai-config-core/src/sync.rs`
- Test: `crates/ai-config-core/tests/projection_plan.rs`

**Produces:** `PlannerContext`、`build_projection_plan(request, context)` 与
`sync::CompatibilityProjectionRoots` / `build_compatibility_plan`。旧
`sync::compute_*` / `SyncAction` 仍仅服务 T009 前的 legacy lifecycle：它们不能把
source-first `ProjectionPlan` 降级后交给硬拷贝 executor；T009 在 T007/T008 完成后统一切换
公开 CLI/MCP 调用面。

- [x] **T005.1 Write failing plan tests**：missing→CreateLink、correct link→Noop、wrong source state 按 ownership 分类但 action→ReportOnly Conflict、equal foreign→Equivalent + gated AdoptEquivalent candidate、different foreign→ReportOnly Conflict、unsupported→ReportOnly Unsupported、generated+record→batch upsert/noop、missing ledger→Foreign、orphan→ReportOnly OrphanCandidate、同一规范化 target 只有一个 batch、renderer disagreement→blocking conflict、plan action 排序/digest 确定、plan JSON 无 secret sentinel。
- [x] **T005.2 Verify RED**：定向用例先后确认 facade、entry key、目标规范化与 orphan cleanup 在实现前均编译失败。
- [x] **T005.3 Implement planner**：接受注入的 `PlannerContext`，遍历 effective assets × requested platforms，并为 Prompt 去重生成唯一 ProjectEntry surface；调用 adapter target、ledger/secret metadata 与 ownership classifier；按规范化 `target.path` 聚合跨 domain generated intents并校验唯一 container renderer；action/member 包含 SourceRef、target fingerprint、reason code/说明，不包含 body/value；任何 adapter parse/renderer disagreement 错误变为 ReportOnly Conflict。
- [x] **T005.4 Add operations**：Sync 只创建/更新 desired；Retract 只生成 managed removal；Uninstall 是全 scope Retract；orphan 在普通 Sync 只产 ReportOnly OrphanCandidate，单独 cleanup 请求且有 ownership 证据时才产 CleanupOrphan；Import/Migrate 暂只产明确 ReportOnly Unsupported，后续任务实现。
- [x] **T005.5 Verify deterministic, read-only and budget**：同 fixture 连续 plan 100 次得到同 digest且文件树与 DB 不变；对 1,000 个发现项运行 release-mode benchmark，记录 100 次样本并断言至少 95 次小于 2 秒。
- [x] **T005.6 Verify**：`cargo test -p ai-config-core --test projection_plan`（20 passed）；release 模式的 1,000 项 × 100 次 p95 < 2 秒门禁通过。
- [x] **T005.7 Commit checkpoint**：`feat(core): 增加只读投影计划器`。

### T006 — Transactional direct-link executor

**Files:**

- Create: `crates/ai-config-core/src/projection/executor.rs`
- Refactor: `crates/ai-config-core/src/link.rs`
- Modify: `crates/ai-config-core/src/asset_ops.rs`, `asset_scope.rs`, `skills_add.rs`
- Test: `crates/ai-config-core/tests/projection_apply.rs`

**Produces:** `apply_projection_plan`、transaction manifest、精确 link/unlink、rollback。

- [x] **T006.1 Write failing executor tests**：覆盖缺失/正确/冲突/陈旧 plan、精确 link retract、adopt、锁、ledger rollback 与显式 copy fallback；copy 测试另覆盖 adapter/policy/授权三重门禁、source/target symlink 拒绝、漂移拒删与 refresh rollback。
- [x] **T006.2 Verify RED**：各 executor/copy contract 先以缺 action、错误 fallback 契约或错误 lifecycle 行为实际失败后再实现；证据保留在定向测试与对应 checkpoint。
- [x] **T006.3 Implement safe link primitives**：创建 sibling temp link 后 rename；target parent canonical path 必须在 allowlisted platform root；删除前用 `symlink_metadata + read_link` 再校验 expected source；禁止通用 `remove_dest_path` 进入正常路径。
- [x] **T006.4 Implement transaction**：排他锁、blocking preflight、source/target precondition、0700 backup dir、snapshot、apply、postcondition、ledger `apply_batch`、reverse rollback；ledger write error 回滚文件，backup manifest 只记录 hash/mode/path，报告最终状态而非中间成功。
- [x] **T006.5 Implement explicit copy fallback**：仅 adapter 显式 `copy_fallback_allowed`、Windows policy、用户授权且 link unavailable 时生成；使用 temp path + backup-swap，拒绝 source/target tree symlink；ledger 记录 `CopyFallback` source/target digest，ledger 缺失或 digest 漂移时为 Foreign/Drifted 且不可删除；普通 sync 默认不启用。
- [x] **T006.6 Verify immediate propagation**：修改 canonical SKILL.md 后平台 direct link 可读取新正文，不再次 sync。
- [x] **T006.7 Verify**：`cargo test -p ai-config-core --test projection_apply`（25 passed），`projection_plan`（20 passed）与 `projection::platform_adapter`（42 passed）。
- [x] **T006.8 Commit checkpoint**：`e3a0672 feat(core): 增加显式安全拷贝回退`；此前 direct transaction checkpoints 为 `b6e6adb`、`58ef496`、`57b70a3`。

### T007 — MCP per-server source、secret migration 与四平台 renderer

**Files:**

- Create: `crates/ai-config-core/src/mcp/{mod,source,adapter,cursor_json,codex_toml,claude_json,hermes_yaml}.rs`
- Modify: workspace/core `Cargo.toml` (`toml_edit = "0.25"`, `yaml-edit = "0.2.3"`)
- Modify: `crates/ai-config-core/src/model.rs`, `secrets.rs`, `projection/planner.rs`, `projection/executor.rs`, `platform/*`
- Modify: `crates/ai-config-cli/src/mcp.rs`
- Test: `crates/ai-config-core/tests/mcp_projection.rs`
- Fixtures: `crates/ai-config-core/tests/fixtures/mcp/*`

**Produces:** `load_mcp_definitions`、`GeneratedConfigAdapter` implementations、legacy monolith migration plan。

**执行状态（2026-07-17，防偏航）**：T007 已通过 source-first 的内部 MCP plan/apply
入口接通；四平台 renderer 以单条语义 fingerprint 证明 ownership，并以一次
parse/backup-swap 写入同一 target。缺失 secret 只产生该 server 的 `skipped` 明细，错误不含
value；migration 仅在 `--extract-secrets --apply` 下写入 0600 store。CLI 的 `deploy/retract`
不切换 `install/sync` 公共生命周期：没有持久 ledger 的独立进程 retract 必须报告
`generated_ownership_unproven` 并零写，不能从容器内容猜测 ownership。持久 ledger 与统一
public lifecycle 切换仍留给 T009。

- [x] **T007.1 Write failing source/security tests**：per-server parse、overlay whole-entry override、disabled/targets filtering、placeholder validation、literal value redacted detection、0600 enforcement、plan/Debug/Serialize 无 sentinel；缺失 secret 只阻塞引用该 key 的 server，错误仅含 key name。
- [x] **T007.2 Write failing renderer/batch tests**：每个平台同一 target 内两个托管 server + 一个 foreign server + unknown top-level fields 只产生一个 batch/一次替换；同一规范化 path 不允许第二 renderer；Cursor foreign/top-level 保留；Codex comments/unknown TOML 保留；Claude user projects/settings 保留、project `.mcp.json` scoped；Hermes user provider/model/comments 保留且 project MCP Unsupported/global config 零变化；remove only owned unchanged server；drift blocks remove。
- [x] **T007.3 Verify RED**：`cargo test -p ai-config-core --test mcp_projection`。
- [x] **T007.4 Implement canonical source**：每个 JSON 转为现有 `McpServer` domain model；`env` 与 HTTP header value 必须是 `${VAR}` 引用，command/args/URL 等非 secret 字段可保留普通字面量；literal env/header、URL userinfo 和疑似 credential args 返回 redacted migration issue。
- [x] **T007.5 Implement lossless container renderers**：JSON object merge、`toml_edit`、`yaml-edit`；planner 按规范化 target path 聚合 intents并绑定唯一 container renderer，adapter 每个目标文件只 parse/render 一次；semantic validate 后一次 backup-swap，保留 unknown/foreign；中间 bytes 只在 executor 内存，最终 bytes 只在目标和 0600 backup。
- [x] **T007.6 Implement explicit secret extraction**：`mcp migrate --extract-secrets --apply` 按 spec 的 key 规则写入 secrets；existing differing value/key collision/URL credential 均 conflict；旧 `mcp.json` 只备份不删除。
- [x] **T007.7 Route MCP domain CLI without lifecycle cutover**：list/show/add/remove/enable/disable 操作 per-server files；deploy/retract 先接入 core plan 的内部/测试入口但不切换 install/sync public lifecycle，保留 legacy read-only import 一个发布周期；统一公开切换留到 T009。
- [x] **T007.8 Verify**：`mcp_projection` 35 passed；CLI `mcp_migrate`、`deploy_retract`、`mcp_plan_apply`、`mcp_source_readonly` 共 31 passed；另 `cargo test -p ai-config-core -p ai-config-cli` 全绿。
- [x] **T007.9 Commit checkpoint**：source/renderer/transaction/CLI 分步 checkpoint 至 `eaeb880 feat(cli): 接入 MCP 源投影执行`；全程未切换 public lifecycle。

### T008 — Hook generated projection 与 canonical Prompt 入口

**Files:**

- Modify: `crates/ai-config-core/src/hook.rs`, `hook_adapter.rs`, `source.rs`, `projection/planner.rs`, `projection/executor.rs`
- Create/refactor focused files: `hook_adapter/{mod,cursor,codex,claude,hermes}.rs`
- Modify: `templates/project/AGENTS.md`, `CLAUDE.md`, `HERMES.md`
- Test: `crates/ai-config-core/tests/hook_projection.rs`
- Test: `crates/ai-config-core/tests/prompt_projection.rs`

**Produces:** Hook script/bundle direct link + platform config GeneratedMutation；`.ai-config/prompts/AGENTS.md` canonical、root `AGENTS.md` ProjectEntry link、Claude minimal wrapper。

**执行状态（2026-07-17，防偏航）**：Prompt canonical source、Hook JSON transaction 与
Hermes user 的跨域 coordinator 已完成独立 RED→GREEN checkpoint。Hook JSON 覆盖 Cursor、
Codex、Claude 的 foreign/unknown 保留、bundle direct-link、精确 retract、drift/ledger rollback
与 legacy import fail-close；Hermes user 的 MCP + external-directory + Hook 则由
`HermesUnifiedYaml` 一次 parse/render/swap，foreign/provider/model 保留且 ledger failure 回滚。
workspace/project Hermes 继续 report-only/零全局写。**尚未完成** 是该 coordinator 接入 T009
public lifecycle 以及 Hermes cross-domain retract/uninstall；在此之前，普通生命周期不得顺序应用
两份针对同一 `config.yaml` 的计划。

- [x] **T008.1 Write failing Hook tests**：scan zero-write；foreign bindings preserved across platforms；retract only owned binding/link；drift blocks retract；bundle single unit；apply failure restores config+link；project overlay；migration always backup；Hermes project Hook unsupported 且 global config 零变化。
- [x] **T008.2 Write failing Prompt tests**：project template seeds `.ai-config/prompts/AGENTS.md`；root AGENTS 是精确 link；四平台只产生一个 ProjectEntry action；Claude wrapper 只引用 AGENTS；现有普通 AGENTS/CLAUDE 或外部 link 为 Equivalent/Foreign/Conflict 且零覆盖；prompt overlay 与 repeated apply noop。
- [x] **T008.3 Write cross-domain container test**：Hermes user `config.yaml` 同时包含 MCP、`skills.external_dirs`、Hook 与 foreign provider/model 时，planner 只生成一个 target batch，container renderer 一次解析/替换并保留全部 foreign/unknown；workspace/project 请求为 Unsupported 且绝不触碰 global config。
- [x] **T008.4 Verify RED**：`hook_projection` 与 `prompt_projection` 均先实测 RED；Prompt 6/6、Hook JSON 11/11 已转 GREEN，保留 Hermes YAML 跨域 RED 给 T008.3。
- [ ] **T008.5 Split adapter responsibilities**：Hook/ExternalDirectory/PromptWrapper adapter 只产生 domain intents；每个目标配置只有一个 platform container renderer 负责跨 domain parse/render；ownership/planning/transaction 留 projection 模块；删除 reconcile side effect。
- [x] **T008.6 Link scripts/bundles**：能直接执行的 script/目录逐项 link；无法链接时显式 unsupported/copy fallback，不再默认复制。
- [x] **T008.7 Consolidate project entry prompts**：template 把完整正文 seed 到 `.ai-config/prompts/AGENTS.md`；root `AGENTS.md` 由 ProjectEntry link 产生；`CLAUDE.md` 仅 import/reference + Claude-specific short section；Hermes/Cursor/Codex 直接读 AGENTS，禁止复制完整正文。
- [x] **T008.8 Verify**：`hermes_cross_domain_projection` 5 passed、`hook_projection` 11 passed、`prompt_projection` 6 passed、`mcp_projection` 35 passed；并回归 `projection_plan` 20 passed、`projection_apply` 25 passed及 core strict clippy。
- [ ] **T008.9 Commit checkpoint**：`git commit -m "feat(hooks): 接入事务化生成投影与统一入口"`。

**T008.5 / T009 handoff（2026-07-17）**：已移除无生产调用的
`reconcile_orphan_hook_scripts` 隐式写源副作用（`5b99621`）。审计确认旧公开 lifecycle/GUI
仍可经 `hook_adapter::deploy`、`asset_ops`、`hermes_config::after_skill_deploy` 直接写平台
容器；这些调用必须在 T009 的同一次 public lifecycle 切换中替换为 plan/apply，不能在 T008
单独禁用而制造功能空窗。故 T008.5 与最终 T008.9 保持未完成，T009.1 必须先以默认零写和
显式 `--apply` 合同覆盖这些调用路径；T009 接通后回填 T008.5/9 验收。

**T009 回填（2026-07-17）**：CLI 与 MCP lifecycle 已统一切换到 projection，旧
`lifecycle.rs` 的 materialize/MCP/Hook/platform 写循环已删除；真实 MCP stdio 与 CLI 对同一
request 的 schema、digest、actions 完全一致。GUI 的 `command_bridge` / Tauri commands 仍可达
旧 `asset_ops`，其替换与 peer-copy 删除属于 T011.4/T011.6 的明确验收范围。为避免把最终
source-first 要求缩成 CLI-only，T008.5/T008.9 继续保持未完成，直到 T011 删除最后的 GUI
旁路；该跨任务门禁不重新打开已经完成的 T009 CLI/MCP 合同。

### T009 — CLI 与 ai-config MCP API 统一走 plan/apply

**Files:**

- Create: `crates/ai-config-cli/src/projection.rs`
- Create: `crates/ai-config-cli/tests/projection_lifecycle.rs`
- Modify: `crates/ai-config-cli/Cargo.toml`, `src/main.rs`, `lifecycle.rs`, `agent_api.rs`, `serve.rs`, `output.rs`
- Modify tests: `report_json.rs`, `agent_api_cli.rs`, `deploy_retract.rs`

**CLI contract:** `install/sync/uninstall` 默认 plan-only；`--apply` 才写入；`--workspace` 保留。MCP `ai_config_sync/deploy/retract` 增加 `apply: bool = false`。只有 T007 MCP renderer 与 T008 Hook/Prompt/container renderer 均完成后，才在本任务统一切换 public lifecycle，避免任何 kind 出现功能空窗。

**执行状态（2026-07-17，已完成）**：CLI `install/sync/uninstall` 默认 plan-only，显式
`--apply` 才创建持久 SQLite ledger 并执行 direct/MCP/Hook/Prompt；workspace members 共用一次
预检、lock、undo journal 与最终 ledger transaction。MCP `ai_config_sync` 复用同一 lifecycle，
默认零写；单项 deploy/retract 在安全计划尚未表达前 fail-closed。CLI 与真实 MCP stdio 对同一
request 的 `schema_version`、`plan_digest`、actions/member identity 完全一致；install 也有默认
零写与显式 apply 合同。运行期后段失败会把已恢复 action 报为 `rolled_back`、失败 action 报为
`failed`、未进入 action 报为 `not_applied`，并统一返回 exit 3；缺 MCP key 返回 exit 4 且只显示
key name；初始化/ledger 文件系统失败保留 exit 5。JSON 错误遵循既有 output contract 在 stdout
输出 redacted envelope，stderr 不混入第二份非结构化错误。旧 `lifecycle.rs` 直写循环已删除，
core+CLI 全量测试与严格 clippy 通过；GUI 旁路仍由 T011 单独收敛。

- [x] **T009.1 Write failing CLI contract tests**：default sync zero-write；`sync --apply` 对 direct/MCP/Hook/Prompt 创建 links/generated batches；second apply unchanged；JSON has schema_version/plan_digest/action/member summaries；blocking foreign conflict 以 exit 3 整 plan零写入；uninstall leaves source/foreign/generated container；MCP tool default no-write；CLI 与 MCP tool 对同一 request 产生相同 digest；install 默认 plan-only、显式 apply。
- [x] **T009.2 Verify RED**：`projection_lifecycle` 5 条合同先在无 `--apply`/plan 报告/统一编排的旧入口下实际失败，再转 GREEN。
- [x] **T009.3 Add contexts, clap flags and reports**：CLI/serve 调用层构造可序列化 `ProjectionRequest` 与注入式 `PlannerContext`/`ExecutorContext`；core 不打开 store；apply 只把 plan、context 和 options 交给 core。CLI/MCP 所有 kind 迁移后已删除 lifecycle 的 materialize/MCP/Hook platform loops；GUI 旧 commands 明确留给 T011.4 删除。
- [x] **T009.4 Define exit/report contract**：0=plan/apply success，3=blocking conflict 或已回滚 transaction failure，4=missing secret skipped，5=filesystem；输出 `failed/rolled_back/not_applied`；JSON error 继续使用 redacted envelope，并遵循既有 output contract 写 stdout。
- [x] **T009.5 Update MCP schemas**：tool output schema 使用 ProjectionPlan/ApplyReport；`apply` 默认 false；agent 无法绕过 conflict/adopt guard。`ai_config_sync` 已复用 lifecycle projection；单项 deploy/retract 在计划表达完成前 fail-closed，禁止旧直写旁路。
- [x] **T009.6 Verify**：指定四组 32 passed；另 `workspace_projection_lifecycle` 4 passed、`cargo test -p ai-config-core -p ai-config-cli` 全绿、core+CLI strict clippy 通过。
- [x] **T009.7 Commit checkpoint**：本任务以 `fix(cli): 完成投影生命周期事务合同` 建立独立 checkpoint；不包含用户 GUI/009 安全止血脏改。

### T010 — Explicit import 与 legacy/.cc-switch migration

**Files:**

- Create: `crates/ai-config-core/src/projection/migration.rs`
- Create: `crates/ai-config-cli/src/migration.rs`
- Create: `crates/ai-config-cli/tests/projection_migration.rs`
- Modify: `asset_ops.rs`, `platform_scan.rs`, `main.rs`, `serve.rs`

**CLI contract:**

```text
ai-config migrate inventory --json
ai-config migrate source-first --plan <file> --select <action-id>... [--extract-secrets] [--apply]
ai-config migrate rollback <transaction-id>
ai-config import <kind> <name> --from <platform> --to global|project [--replace] [--apply]
ai-config adopt --plan <file> --select <action-id>... --apply
```

**执行状态（2026-07-17，Skills inventory checkpoint）**：已新增纯 core migration
inventory 与 `migrate inventory` 只读 CLI，第一切片只处理 Skills，并严格扫描 canonical
`skills/`、current `.agents/skills` / `.claude/skills`、legacy `.cursor/skills` /
`.codex/skills`、external `.cc-switch/skills` 与 builtin `.codex/skills/.system`。未知链接
只 readlink + lexical compare，未命中 allowlist 时不 canonicalize/read/hash target；legacy marker
只产生 foreign candidate，不授予 ownership；`.cc-switch` / builtin 永不 selectable；报告与 digest
deterministic 且不创建 lock/backup/DB。CLI E2E 6 passed、core unit 3 passed、core+CLI strict clippy
通过。T010.4 仍未完成：Rules → MCP → Agents → Commands → Hooks 及 workspace/project scopes
必须继续逐类 RED→GREEN，不能把 Skills-only inventory 当成完整迁移盘点。

**执行状态（2026-07-17，Rules inventory checkpoint）**：在 Skills 基线之上补齐
global/project Rule 分层盘点。canonical `rules/*.mdc` 保留原始 source layer 记录，并按
`global → project` 选择 effective source；global 只盘点 Claude `.claude/rules/*.md`，project
盘点 Cursor/Hermes 共享 `.cursor/rules/*.mdc` 与 Claude `.claude/rules/*.md`。Codex
`.codex/rules/*.rules` 仅作为仍被消费的外部执行策略记录，绝不与 instruction Rule 互认或授予
ownership。项目盘点只读取项目 deploy root，不回退 HOME，也不扫描仅属于全局用户环境的
`.cc-switch` / builtin roots；legacy `.codex/skills` 只盘点、不标记为当前消费。Rules RED 为
2 tests，安全复审新增 2 个断言后再次得到 2 个 RED；最终 CLI E2E 8 passed、core unit 5 passed，
core+CLI 全量测试与 strict clippy 通过。T010.4 仍未完成：workspace scope 与 MCP → Agents →
Commands → Hooks 必须继续逐类 RED→GREEN。

**执行状态（2026-07-17，MCP inventory checkpoint）**：补齐 global/project MCP 逐 server
盘点。canonical 仅接受各 layer 的 direct regular `mcp/servers/<name>.json`，按 server name
整项 overlay，并验证 filename/name、source schema 与 secret references；报告只包含 secret key
名和 opaque fingerprint，不包含 config/body/literal value。global 盘点 Cursor JSON、Codex TOML、
Claude user JSON、Hermes YAML；project 只盘点 repo 内 Cursor/Codex/Claude 容器，Hermes 明确
`unsupported` 且不读取 HOME。无 ledger 的同名 generated entry 仍为 foreign/unowned/
unselectable，有 canonical 冲突时 blocking；Codex project 报 `trusted_project`。Codex、Claude、
Hermes 与 canonical monolith legacy 只作 inventory-only。平台与 canonical MCP 路径在读取前逐级
lstat 父组件，父目录/最终文件软链接均不可逃出 approved root；损坏、不可读、非普通、软链接、
filename/name mismatch 只产生不含原文的结构化 blocking issue，并继续盘点其他 roots。首轮 MCP
RED 2/2、安全 ownership RED 2/2、invalid/unsafe container RED 1/1、父目录逃逸与 legacy error
RED 1/1 均实际运行；最终 CLI E2E 12 passed、core unit 5 passed、strict clippy 通过。T010.4
仍未完成：workspace scope 与 Agents → Commands → Hooks 必须继续逐类 RED→GREEN。

**执行状态（2026-07-17，Agents inventory checkpoint）**：补齐 global/project Agent 分层盘点。
canonical 当前格式只接受 direct regular `agents/<name>.md`，并验证 UTF-8、YAML frontmatter
`name`/`description`、文件名与 name 一致及非空正文；无效 schema、软链接和不安全父路径只产生
不含正文的结构化 blocking issue。current native target 为 Cursor `.cursor/agents/*.md`、Codex
`.codex/agents/*.toml` 与 Claude `.claude/agents/*.md`；无 ledger 的生成文件保持 foreign/unowned/
unselectable，同名 canonical 存在时 blocking。旧 canonical 目录及 YAML/YML/JSON、Codex/Claude
`.codex|.claude/subagents` 的 direct regular/extensionless/directory 仅作 nonblocking legacy
inventory，保留 effective source 关联但不参与 case collision；隐藏项、README、系统/构建目录与
备份文件排除。Hermes static Agent 明确 `unsupported`，不猜测或扫描 `.hermes/agents`；project
scope 不读取 HOME 平台目录。报告只含 opaque digest/format/provenance/reason，不序列化 frontmatter、
body、tools、model 或外链内容。首轮 native RED 2/2、schema/symlink RED 1/1、legacy/name-parity
RED 1/1、current wrong-shape RED 1/1 均实际运行；同扩展目录/特殊文件现在 fail-closed，legacy
child symlink 只 lstat、不跟随。最终 CLI E2E 15 passed、core unit 5 passed、strict clippy 通过。
T010.4 仍未完成：workspace scope 与 Commands → Hooks 必须继续逐类 RED→GREEN。

**执行状态（2026-07-17，Commands inventory checkpoint）**：补齐 global/workspace/project
Command 分层盘点。canonical 仅接受 direct regular UTF-8 `commands/<name>.md`，按 whole-command
执行 `global → workspace → project` overlay；Cursor/Claude current target 分别为 scope deploy base
下 `.cursor/commands/*.md` 与 `.claude/commands/*.md`，正确 canonical link 为 ManagedLink，普通
同内容文件仅为 Equivalent，foreign/wrong-shape/unknown link 均 fail-closed。Codex/Hermes 对所有
effective Command 明确 `unsupported`，禁止扫描或创建 `.codex/commands`、`.hermes/commands`；仅
global 盘点 deprecated `.codex/prompts/*.md`，保持 consumed 但 foreign/unowned/unselectable/
nonblocking，并排除出 current/canonical case-collision 域。Workspace/Project 不扫描 HOME 平台
目录或本地 `.codex/prompts`。canonical/current/legacy 父路径与 direct child 均 lstat-only；报告只含
opaque digest/format/provenance/reason，不序列化正文、frontmatter、argument hint 或外链内容。
CLI global/project/security RED 3/3、Workspace core RED 1/1 均实际运行；最终 Commands CLI 3 passed、
完整 migration E2E 18 passed、core migration 6 passed、strict clippy 通过。T010.4 仍未完成：Hooks
inventory 必须继续 RED→GREEN；Workspace 的 Skills/Rules/MCP/Agents 统一补证仍是未完成边界。

**执行状态（2026-07-17，Hooks inventory checkpoint）**：补齐 global/workspace/project Hook
复合盘点。canonical Hook 只能由同一 source layer 的 `hooks.json#hooks.<name>` binding 与
`hooks/<name>` 单文件或 bundle 共同组成，按整项执行 `global → workspace → project` overlay；
binding-only、script-only 和跨 layer 拼接都 blocking，不能生成半投影。current target 固定为
Cursor `.cursor/hooks.json` + `.cursor/hooks/`、Codex `.codex/hooks.json` + `.codex/hooks/`、
Claude `.claude/settings.json` + `.claude/hooks/`，Hermes 仅 global 使用 `.hermes/config.yaml` +
`.hermes/hooks/`，workspace/project 明确 `unsupported`；Codex inline `config.toml#hooks` 只作
legacy inventory，永不授予 ownership/selectable。canonical lifecycle、matcher、argument 与平台
外层 matcher 都进入 opaque digest/plan digest；unknown lifecycle、缺失 bundle leaf、file-as-dir、
binding marker/command name mismatch、foreign half、父路径/后代 symlink 及 socket/FIFO/Other 节点
均 fail-closed。盘点只在 approved deploy root 内 lstat/read，报告不包含脚本正文、binding 原文或
secret value。CLI 首轮 2/2、Workspace core 1/1、安全复审 1/3 与 P1 复审 1/3 均取得真实 RED；
后续 catalog/native-shape/digest-isolation 复审 4/4、split cross-layer 复审 1/1 也取得真实 RED。
malformed document root 与跨平台脚本根复审 2/2 同样实际失败后修复。最终 Hooks CLI 3 passed、完整
migration E2E 21 passed；现有 adapter 合法 command 路径复审 1/1 真实失败后修复，最终 core
Claude-only shared Cursor root 复审 1/1 也先 RED 后 GREEN。最终 core Hooks/migration 19 passed、
strict clippy 与 core+CLI 全量测试通过；
生命周期直接引用唯一 Cursor catalog，平台 binding 按 Cursor/Hermes
direct、Codex/Claude grouped、legacy TOML 独立解析，具名 digest 不受 sibling/foreign 字段污染。
T010.4 仍未完成：Workspace 的 Skills/Rules/MCP/Agents 统一补证、CLI scope 接线与 lifecycle overlay
必须单独 RED→GREEN 后，才能进入 import/adopt/rollback。

**执行状态（2026-07-17，Workspace inventory/lifecycle checkpoint）**：补齐 Workspace
分层盘点与成员投影闭环。CLI 仅在显式 `--workspace --root <workspace-root>` 时构造
`DeploymentScope::Workspace` inventory，使用 global canonical + workspace canonical overlay，
并将 workspace root 作为 deploy base；缺少显式 root 时 fail-closed，禁止回落到 HOME 或当前目录。
core 对 Skills/Rules/MCP/Agents 补齐 `global → workspace` 整项覆盖，结合此前 Commands/Hooks
实现后六类资产均具备 Workspace inventory；canonical asset root、Skills/Rules approved root 与父路径
统一 lstat-only，root/parent symlink、非目录及不可读节点均产生结构化 blocking issue。Hermes 的
workspace Skills/MCP/Agents 与 Codex instruction Rules 维持 schema v1 精确 unsupported reason，
不扫描 HOME 平台目录。Workspace lifecycle 使用 `global → workspace → project` source overlay，
即使成员没有本地 `.ai-config` 也继承 workspace/global defaults；apply 仅写各成员 deploy target，
继续沿用统一预检、事务与回滚合同。CLI Workspace scope 首轮实际得到 `project` 而 RED；core overlay
首轮缺少 Workspace unsupported reason 而 RED；Skills/Rules parent 与 canonical root symlink 复审均
先 RED；lifecycle 空成员缺少 workspace default 也先 RED。最终 migration E2E 23 passed、Workspace
lifecycle 6 passed、core migration 定向 12 + Hooks/P1 11 passed、strict clippy 与 core+CLI 全量测试
通过（core 358 passed）。T010.4 完成；下一检查点进入 T010.2/T010.5–T010.7 的显式
import/adopt/rollback。

**执行状态（2026-07-17，显式 import 安全事务 checkpoint）**：新增独立 `migrate plan` /
`migrate source-first --plan ... --select ... [--apply]` 与顶层 `import` 入口；所有写入默认
plan-only，apply 必须精确匹配当前 plan digest 与 action ID。Equivalent adopt 已复用统一 projection
executor，且整份 plan 任一非 secret-skip blocking action 都在 writable ledger 打开前整批拒绝；对应
CLI E2E 先观察到错误写入 1 项，再修复为 8 passed。显式 import 当前覆盖 direct file/Skill、项目
Prompt 与 Cursor MCP JSON：create/replace 均冻结 apply-time source、保留平台原件，以 0700 transaction
root、排他锁、Prepared/Applying/Applied manifest、原子清单更新和关键目录 fsync 执行；任一 action、
postcondition 或后续 projection plan 失败均逆序恢复。rollback 只接受 caller-approved canonical root，
整批预检 before/after/backup fingerprint，漂移零目标写入；递归 credential-like 字段与未选 blocking
sibling 均会阻止整批。nested secret、stale lock、未选 sibling 与 crash-recovery 测试均先取得真实
RED；最终 core import 16 passed、projection executor 29 passed、CLI migration action 8 passed、
read-only Store 2 passed，core/store/CLI strict clippy 通过。plan/review 打开既有 SQLite ledger 使用
immutable read-only 模式，不建库、不迁移旧 schema、不跟随 ledger symlink；apply 只在全部 review
验证后打开 writable Store。T010.2/T010.5/T010.6 仍未完成：adopt 尚无统一 durable transaction ID
与人工 rollback；legacy `mcp migrate --extract-secrets --apply` 仍是待封堵写旁路；`.cc-switch`、
Workspace migration/import、Codex/Claude/Hermes MCP normalize、字段级 normalized diff、顶层 `adopt`
alias 与 MCP server surface 必须继续逐项 RED→GREEN，不能把当前安全切片视为完整 T010。

- [x] **T010.1 Write failing inventory tests**：legacy marker copy、unmarked equal copy、different copy、correct/wrong/broken symlink、`.cc-switch` external_owned、plugin/builtin、case-only collision、dotfiles in digest、unknown-root link 不跟随；另补未知不可读 target lstat-only、external platform link 不获 ownership、builtin 不重复盘点。
- [ ] **T010.2 Write failing migration/import/adopt E2E**：inventory no-write；plan deterministic；stale digest/action reject；未选择项零写入；不存在平台级/来源级 bulk takeover；Equivalent 逐 action-id 备份后 adopt；不同内容 foreign 直接 adopt 拒绝、必须先 import；`.cc-switch` 逐项选择；canonical-first then projection；transaction failure rollback；repeat plan noop；rollback refuses drifted post-state；import preview 显示 normalized diff、明确 destination source layer/absolute path、secret preflight 结果；foreign `AGENTS.md` 只有显式 import 后才进入 Prompt source 且平台原件不删除。
- [x] **T010.3 Verify RED**：首轮 4 tests 编译并实际运行，均因顶层 `migrate` 不存在而失败；安全复审扩为 6 tests 后由最小 Skills inventory 实现转 GREEN。
- [x] **T010.4 Implement allowlisted inventory**：只扫描 canonical/global/workspace/project、官方 current/legacy platform roots、`.cc-switch/skills` 和已知 plugin roots；每项记录 provenance/reason，不读取/输出 secret values。
- [ ] **T010.5 Implement explicit import**：只允许 create-only 或 replace-source-with-backup；预览必须包含 normalized diff、destination layer/path、敏感信息检查；平台原件不删除；source 校验通过后另建 projection plan。
- [ ] **T010.6 Implement migration/adopt/rollback**：默认 dry-run；legacy marker 只可生成建议，任何 apply 都需 plan digest + selected action IDs；equal ordinary copy 和 `.cc-switch` 必须逐项选择；transaction manifest 保留至少一个发布周期。
- [ ] **T010.7 Verify**：`cargo test -p ai-config-cli --test projection_migration` 与 `cargo test -p ai-config-cli --test projection_migration_actions`。
- [ ] **T010.8 Commit checkpoint**：`git commit -m "feat(migrate): 增加外部资产盘点与可回滚收敛"`。

### T011 — GUI/Tauri 从五平台对等改为 source-first

**Files:**

- Modify: `apps/ai-config-gui/src/command_bridge.rs`, `src/lib.rs`
- Modify: `src/types.ts`, `src/api/tauriAssets.ts`, `src/hooks/useAssetOperations.ts`
- Modify: `src/utils/entryPlatformToggle.ts`, `aggregatePlatformState.ts`, `entryUpdate.ts` and tests
- Modify: `src/components/assets/PlatformIconButtons.tsx`, `AssetRow.tsx`, `RowSyncActions.tsx`
- Replace/modify: `src/components/feedback/SyncConflictModal.tsx` → plan/diff modal
- Modify: zh-CN/en-US i18n

**Produces:** Tauri `projection_plan`、`projection_apply`、`projection_retract`、`import_to_source` commands；TS `ProjectionState` union 与 plan types。

- [ ] **T011.1 Write failing Rust bridge tests**：Tauri bridge 只调用 core；默认 plan-only；无 `materialize`/platform-to-platform implementation；planner ledger read error 安全降级为“不证明 ownership”，apply ledger write error 则中止并回滚；同一规范化 fixture 经 CLI、ai-config MCP API、Tauri bridge 的 schema/action ordering/plan digest 完全一致。
- [ ] **T011.2 Write failing frontend state tests**：Managed 才可 retract；Missing 可 create；Equivalent 需基于当前 plan digest + action_id adopt confirmation；不同内容 Foreign 不可直接 adopt，只可 import；Conflict/Drifted 只可 diff/import/repair；ai-config icon 不可 retract；无 peer/bulk takeover。
- [ ] **T011.3 Verify RED**：`cargo test -p ai-config-gui`；`cd apps/ai-config-gui && npm run test -- src/utils/entryPlatformToggle.test.ts src/components/assets/PlatformIconButtons.test.tsx`。
- [ ] **T011.4 Implement thin commands/types**：Rust bridge 构造 request、PlannerContext/ExecutorContext 后调用 core；TS 与 Rust serde names 一致；删除 `cmd_*_deploy_from_platform`、`cmd_apply_sync_choice` 等写路径。
- [ ] **T011.5 Implement plan modal**：展示 changed/unchanged/conflict/unsupported、source layer、target、backup policy 和 secret key names；普通 apply 传 plan digest，adopt/migration 另传逐项 selected action IDs，禁止“接管整个平台”按钮。
- [ ] **T011.6 Remove peer-copy UX**：平台视图仅 Import to ai-config；source view 才可投影；列表状态文案不再把 synced/linked 混用。
- [ ] **T011.7 Verify**：`cargo test -p ai-config-gui`；`cd apps/ai-config-gui && npm run test && npm run build`。
- [ ] **T011.8 Commit checkpoint**：`git commit -m "refactor(gui): 切换为单一源投影交互"`。

### T012 — 仓库 dogfood、旧实现清理与文档收敛

**Files:**

- Move canonical project assets into `.ai-config/{skills,rules,agents,commands,prompts,hooks,mcp}`
- Remove tracked generated copies under `.cursor/skills`, `.codex/skills`, `.claude/skills` and generated hook/MCP configs
- Modify: `.gitignore`, `AGENTS.md`, `CLAUDE.md`, `HERMES.md`
- Modify: `.specify/memory/constitution.md`, `.cursor/rules/00-ai-config-core.mdc`, `hooks-asset-layout.mdc`
- Modify: `README.md`, `docs/product/{PRD,ARCHITECTURE,DESIGN}.md`, `CHANGELOG.md`
- Create: `crates/ai-config-cli/tests/repository_hygiene.rs`
- Delete after zero references: `materialize.rs`, `sync_conflict.rs`, legacy MCP paths/functions, obsolete path-independence logic

- [ ] **T012.1 Write failing hygiene tests**：`git ls-files` fixture 禁止 platform skill/prompt regular copies、generated configs 和 machine absolute paths；canonical project assets 必须唯一；root AGENTS 只能是指向 `.ai-config/prompts/AGENTS.md` 的 tracked symlink（或经明确豁免的最小 bootstrap）；tracked MCP fixture 仅 placeholders。
- [ ] **T012.2 Verify RED**：`cargo test -p ai-config-cli --test repository_hygiene`，预期当前四套 tracked copies 与绝对路径导致失败。
- [ ] **T012.3 Dogfood source-first layout**：保留 `.ai-config` canonical；完整入口正文迁入 `.ai-config/prompts/AGENTS.md`，根 AGENTS 使用 ProjectEntry link；平台 projection 改为本地生成并 gitignored。
- [ ] **T012.4 Remove legacy production paths**：`rg` 确认无调用后删除 materialize、peer-copy、doctor materialize、旧 MCP whole-file deploy/retract；legacy inventory reader 保留一个发布周期且严格只读。
- [ ] **T012.5 Rewrite product truth**：PRD 明确唯一源/单向 projection；ARCHITECTURE 只保留 planner/executor；DESIGN 更新状态/按钮；constitution 把“直接覆盖不备份”改为“foreign fail-closed + generated transaction backup”。
- [ ] **T012.6 Verify no contradictory terms**：运行 `rg -n "五平台对等|独立硬拷贝|deploy_from_platform|materialize::deploy|\.codex/mcp\.json|\.claude/mcp\.json" README.md docs crates apps`；除 migration/history 章节外预期零命中。
- [ ] **T012.7 Verify hygiene**：`cargo test -p ai-config-cli --test repository_hygiene`。
- [ ] **T012.8 Commit checkpoint**：`git commit -m "refactor(core): 删除对等硬拷贝并收敛产品文档"`。

### T013 — 全量验证、canary 与发布门禁

**Files:**

- Update this plan’s checkbox states only after each command succeeds.
- No feature implementation beyond fixing verification failures attributable to this feature.

- [ ] **T013.1 Format/lint**：`cargo fmt --all --check`；`cargo clippy --workspace --all-targets -- -D warnings`；预期 exit 0。
- [ ] **T013.2 Core/CLI/store tests**：`cargo test -p ai-config-core -p ai-config-store -p ai-config-cli -- --test-threads=1`；预期 0 failed。
- [ ] **T013.3 Workspace tests**：`cargo test --workspace -- --test-threads=1`；预期 0 failed。
- [ ] **T013.4 GUI tests/build**：`cd apps/ai-config-gui && npm run test && npm run build`；随后仓库根 `cargo test -p ai-config-gui`；预期全绿。
- [ ] **T013.5 Temp-HOME acceptance corpora**：运行 deterministic fixture generator + migration/source-projection/MCP/Hook suites：至少 100 effective assets × 4 平台，逐项核对 target/strategy/unsupported；至少 50 foreign + 10 blocking conflicts，断言 sync/retract/uninstall 零修改；至少 200 legacy/foreign/conflict 混合项，断言分类 100%、未选项数据丢失 0、repeat migration action 0；同时覆盖 missing/correct/wrong/broken link、marker/copy/.cc-switch、四格式、global/workspace/project、stale plan、rollback。
- [ ] **T013.6 Sentinel secret scan**：以唯一哨兵运行所有 CLI JSON/MCP API/GUI bridge error paths；对 captured stdout/stderr/log/DB/plan/manifest/source 扫描，预期 0 命中。
- [ ] **T013.7 Real-HOME inventory-only**：`./target/debug/ai-config migrate inventory --json`、`sync --json`、`doctor --json`、`status --json`；执行前后 HOME allowlisted roots tree hash 必须完全一致。
- [ ] **T013.8 One-skill canary**：经用户再次确认后，只选择一个无 secret、非关键、无 conflict skill 执行 `--apply`；Cursor/Codex/Claude/Hermes 新会话确认发现；第二次 plan changed=0。
- [ ] **T013.9 Final review**：运行 requesting-code-review；逐项对照 42 条 FR 与 12 条 SC，确认 CLI/MCP/GUI digest contract、1,000-item 性能样本、platform contract snapshot、无 placeholder/接口漂移、rollback 证据齐全。
- [ ] **T013.10 Commit checkpoint**：`git commit -m "test(projection): 完成单一源迁移验证闭环"`。

## Post-release 本机迁移顺序

该段是发布后的运维 runbook，不在实现阶段自动执行：

1. 停止 ai-config daemon/watch 和任何会写 skills 的 `.cc-switch` 功能；保留 Provider 切换。
2. 对 `~/.ai-config`、平台目录、`.cc-switch/skills`、Codex/Claude 原生 MCP 配置做只读 inventory，保存 redacted plan。
3. 对已进入 Git 的 literal MCP env/header 按潜在泄露处理：人工确认、服务侧轮换；历史重写另行授权。
4. 先迁移 canonical MCP 到 per-server + secret references，验证 0600 与四平台 renderer；不删旧文件。
5. 对同名 assets 按 `conflict > external_owned > equivalent > legacy_managed` 顺序人工处理；不批量 take-over `.cc-switch`。
6. 选择一个普通 skill canary，备份后将平台旧副本替换为逐项 link；验证四平台。
7. 按 kind 分批迁移 Skills → Rules → MCP → Agents → Commands → Hooks；每批先留 RED 证据、再最小迁移并运行定向测试、second-plan-noop 和 uninstall-preserves-foreign；每个计划任务验证通过后提交 checkpoint。
8. 连续两个版本 inventory 不再发现 legacy managed copy 后，才清理备份和只读 legacy reader。
