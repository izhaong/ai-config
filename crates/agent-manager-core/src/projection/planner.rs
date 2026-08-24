//! 只读 source-first 投影计划器。
//!
//! 本模块只读取 canonical source、目标路径与可选 ledger；它永不创建目录、链接、
//! 配置文件或 ledger 记录。写入由后续 executor 在已确认的 plan 上执行。

use std::collections::{BTreeMap, HashSet};
use std::fs;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::CoreError;
use crate::model::{AssetKind, PlatformId};

use super::fingerprint::path_fingerprint;
use super::ledger::ProjectionLedger;
use super::mcp::entry_fingerprint::{
    fingerprint_mcp_config, inspect_codex_mcp_entries, inspect_cursor_mcp_entries,
    inspect_hermes_mcp_entries, McpEntryFingerprint,
};
use super::mcp::source::{load_mcp_definition_at, EffectiveMcpDefinition};
use super::model::{
    DeploymentScope, EffectiveAsset, PathFingerprint, ProjectionId, ProjectionMode,
    ProjectionRecord, ProjectionState, ProjectionSurface, ProjectionTarget, SourceRef,
};
use super::ownership::{classify_projection, ProjectionExpectation, ProjectionObservation};
use super::platform_adapter::{
    capability_contract_for, hook_capability_contract_for, HookCapability, PlatformCapability,
    TargetContext, TrustRequirement,
};

pub const PROJECTION_PLAN_SCHEMA_VERSION: u16 = 1;

/// Sync 只描述 desired state；Retract/Uninstall 只允许移除已经证明托管的目标。
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionOperation {
    Sync,
    Retract,
    Uninstall,
    CleanupOrphans,
    Import,
    Migrate,
}

/// Planner 输入由 CLI / GUI / MCP bridge 构造；core 不依赖 store 或 HOME。
#[derive(Debug, Clone, Serialize)]
pub struct ProjectionRequest {
    pub operation: ProjectionOperation,
    pub scope_key: String,
    pub scope: DeploymentScope,
    pub deploy_base: Utf8PathBuf,
    pub assets: Vec<EffectiveAsset>,
    pub platforms: Vec<PlatformId>,
}

/// 注入式只读依赖。账本不可读时不推断任何 generated target 属于本工具。
pub struct PlannerContext<'a> {
    ledger: &'a dyn ProjectionLedger,
    link_availability: LinkAvailability,
    copy_fallback_policy: CopyFallbackPolicy,
    copy_fallback_authorization: CopyFallbackAuthorization,
    mcp_secret_availability: Option<&'a dyn McpSecretAvailability>,
}

/// Read-only secret preflight boundary for MCP planning. Implementations return only the names
/// of unavailable declared keys; the planner cannot receive or serialize secret values.
pub trait McpSecretAvailability {
    fn missing_secret_keys(&self, declared_keys: &[String]) -> Result<Vec<String>, CoreError>;
}

impl<'a> PlannerContext<'a> {
    pub fn new(ledger: &'a dyn ProjectionLedger) -> Self {
        Self {
            ledger,
            link_availability: LinkAvailability::Available,
            copy_fallback_policy: CopyFallbackPolicy::Never,
            copy_fallback_authorization: CopyFallbackAuthorization::Denied,
            mcp_secret_availability: None,
        }
    }

    /// Copy fallback is intentionally an injected planning condition, not an executor retry.
    /// Production callers may set `WindowsExplicit` only after the adapter has established that
    /// this target cannot create a link and the user has explicitly authorized a copied state.
    pub fn with_copy_fallback(
        ledger: &'a dyn ProjectionLedger,
        link_availability: LinkAvailability,
        copy_fallback_policy: CopyFallbackPolicy,
        copy_fallback_authorization: CopyFallbackAuthorization,
    ) -> Self {
        Self {
            ledger,
            link_availability,
            copy_fallback_policy,
            copy_fallback_authorization,
            mcp_secret_availability: None,
        }
    }

    /// Inject a caller-owned read-only secret availability probe. It may disclose only missing
    /// variable names, never values; callers without this capability retain the conservative
    /// source-only plan and apply-time check.
    pub fn with_mcp_secret_availability(
        mut self,
        availability: &'a dyn McpSecretAvailability,
    ) -> Self {
        self.mcp_secret_availability = Some(availability);
        self
    }

    fn copy_fallback_is_authorized_for(&self, consumers: &[PlatformId]) -> bool {
        self.link_availability == LinkAvailability::Unavailable
            && self.copy_fallback_authorization == CopyFallbackAuthorization::Granted
            && self.copy_fallback_policy.allows_any(consumers)
    }

    fn missing_mcp_secret_keys(&self, declared_keys: &[String]) -> Result<Vec<String>, CoreError> {
        let Some(availability) = self.mcp_secret_availability else {
            return Ok(Vec::new());
        };
        let mut missing = availability.missing_secret_keys(declared_keys)?;
        missing.sort();
        missing.dedup();
        if missing
            .iter()
            .any(|key| !declared_keys.iter().any(|declared| declared == key))
        {
            return Err(CoreError::InvalidPath(
                "MCP secret availability returned an undeclared key".to_owned(),
            ));
        }
        Ok(missing)
    }
}

/// Link availability is measured before planning and injected so dry-run remains read-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkAvailability {
    Available,
    Unavailable,
}

/// Copying is a Windows-only escape hatch. `Never` is the default and is the only policy used
/// unless a platform adapter explicitly opts into a Windows fallback request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyFallbackPolicy {
    Never,
    WindowsExplicit { platforms: Vec<PlatformId> },
}

impl CopyFallbackPolicy {
    pub fn windows_explicit_for(platforms: impl IntoIterator<Item = PlatformId>) -> Self {
        let mut platforms = platforms.into_iter().collect::<Vec<_>>();
        platforms.sort_by_key(platform_key);
        platforms.dedup();
        Self::WindowsExplicit { platforms }
    }

    fn allows_any(&self, consumers: &[PlatformId]) -> bool {
        match self {
            Self::Never => false,
            Self::WindowsExplicit { platforms } => consumers
                .iter()
                .any(|consumer| platforms.contains(consumer)),
        }
    }
}

/// A user confirmation is represented separately from platform policy, so policy alone can
/// never turn a normal sync into a copy operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyFallbackAuthorization {
    Denied,
    Granted,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionActionKind {
    CreateLink,
    RemoveManagedLink,
    /// Explicit Windows-only copied projection. It is never emitted as an automatic retry.
    CopyFallback,
    /// Removal is permitted only when the planned digest still matches the ledger proof.
    RemoveManagedCopy,
    RemoveGeneratedEntries,
    AdoptEquivalent,
    Noop,
    UpsertGeneratedBatch,
    CleanupOrphan,
    ReportOnly,
}

/// The one parser/renderer family permitted to own a generated container. This is explicit plan
/// metadata rather than a transient planner string, so apply can never select a renderer that was
/// not part of the reviewed plan digest.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GeneratedContainerRenderer {
    McpJson,
    McpToml,
    McpYaml,
    HookJson,
    HookToml,
    HookYaml,
    /// T008 cross-domain Hermes user configuration. This is the only renderer allowed to own
    /// MCP servers, `skills.external_dirs`, and Hook bindings in one YAML transaction.
    HermesUnifiedYaml,
    GenericJson,
    GenericToml,
    GenericYaml,
    Markdown,
    ExternalDirectory,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProjectionMember {
    pub id: ProjectionId,
    pub source: SourceRef,
    /// Renderer-owned key inside a generated container. Direct links use `None`.
    pub entry_key: Option<String>,
}

/// A source-first MCP intent deliberately excludes the server configuration and secret values.
/// Apply can re-read `source` only after it has revalidated the plan-bound fingerprint.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct McpProjectionMember {
    pub id: ProjectionId,
    pub name: String,
    pub source: SourceRef,
    pub entry_key: String,
    /// Variable names only; values are resolved by a later executor through the secret provider.
    pub secret_keys: Vec<String>,
    /// Planner-known unavailable variables. Empty means apply may still resolve this member.
    pub missing_secret_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProjectionAction {
    pub kind: ProjectionActionKind,
    pub target: Option<ProjectionTarget>,
    pub precondition: Option<PathFingerprint>,
    /// Required for a cleanup candidate; copied from the ledger and included in its action ID.
    pub ownership_fingerprint: Option<String>,
    /// Present only for generated container actions. Direct links and report-only actions have no
    /// renderer and therefore cannot accidentally become generated writes at apply time.
    pub generated_renderer: Option<GeneratedContainerRenderer>,
    pub members: Vec<ProjectionMember>,
    /// MCP has source-first per-server semantics and must never be represented by generic asset
    /// members. Empty for all non-MCP actions.
    pub mcp_members: Vec<McpProjectionMember>,
    pub consumers: Vec<PlatformId>,
    /// Stable reason code; does not contain source body, rendered config or secret values.
    pub state: Option<String>,
    pub reason_code: String,
    /// Human-facing explanation. This is intentionally excluded from `plan_digest`.
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PlanWarning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProjectionPlan {
    pub schema_version: u16,
    pub actions: Vec<ProjectionAction>,
    /// 与 `actions` 同索引的稳定选择键；apply 对 adopt/cleanup 必须校验它和 plan digest。
    pub action_ids: Vec<String>,
    /// 与 `actions` 同索引的 apply 前 trust gate；None 表示不需要额外信任确认。
    pub trust_requirements: Vec<TrustRequirement>,
    pub warnings: Vec<PlanWarning>,
    pub plan_digest: String,
}

/// 从 effective assets 和平台纯契约构造 deterministic、零写入的计划。
pub fn build_projection_plan(
    request: &ProjectionRequest,
    context: &PlannerContext<'_>,
) -> Result<ProjectionPlan, CoreError> {
    let target_context = TargetContext {
        scope: request.scope,
        deploy_base: request.deploy_base.clone(),
    };
    let mut assets = request.assets.clone();
    assets.sort_by(|left, right| asset_key(left).cmp(&asset_key(right)));
    let mut platforms = request.platforms.clone();
    platforms.sort_by_key(platform_key);
    platforms.dedup();

    if matches!(
        request.operation,
        ProjectionOperation::Import | ProjectionOperation::Migrate
    ) {
        let actions = assets
            .iter()
            .flat_map(|asset| {
                platforms.iter().map(move |platform| ProjectionAction {
                    kind: ProjectionActionKind::ReportOnly,
                    target: None,
                    precondition: None,
                    ownership_fingerprint: None,
                    generated_renderer: None,
                    members: vec![ProjectionMember {
                        id: projection_id(request, asset, ProjectionSurface::Platform(*platform)),
                        source: asset.source_ref(),
                        entry_key: None,
                    }],
                    mcp_members: Vec::new(),
                    consumers: Vec::new(),
                    state: Some("unsupported".to_owned()),
                    reason_code: "operation_not_implemented".to_owned(),
                    reason:
                        "import/migrate execution is implemented by the dedicated migration task"
                            .to_owned(),
                })
            })
            .collect();
        return finish_plan(request, actions, Vec::new());
    }

    if request.operation == ProjectionOperation::CleanupOrphans {
        return build_orphan_cleanup_plan(request, context);
    }

    let mut direct = Vec::new();
    let mut generated: BTreeMap<String, GeneratedBatch> = BTreeMap::new();
    let mut report_only = Vec::new();
    let mut warnings = Vec::new();

    for asset in &assets {
        if asset.kind == AssetKind::Prompt {
            plan_project_entry_prompt(
                request,
                asset,
                &platforms,
                &mut direct,
                &mut report_only,
                &mut warnings,
                context,
            )?;
            continue;
        }
        if asset.kind == AssetKind::Hook {
            plan_hook_asset(
                request,
                asset,
                &platforms,
                &target_context,
                &mut direct,
                &mut generated,
                &mut report_only,
                &mut warnings,
                context,
            )?;
            continue;
        }
        if asset.kind == AssetKind::Mcp {
            for platform in &platforms {
                report_only.push(ProjectionAction {
                    kind: ProjectionActionKind::ReportOnly,
                    target: None,
                    precondition: None,
                    ownership_fingerprint: None,
                    generated_renderer: None,
                    members: Vec::new(),
                    mcp_members: vec![McpProjectionMember {
                        id: projection_id(request, asset, ProjectionSurface::Platform(*platform)),
                        name: asset.name.clone(),
                        source: asset.source_ref(),
                        entry_key: String::new(),
                        secret_keys: Vec::new(),
                        missing_secret_keys: Vec::new(),
                    }],
                    consumers: Vec::new(),
                    state: Some("unsupported".to_owned()),
                    reason_code: "mcp_requires_source_first_definition".to_owned(),
                    reason: "MCP projection requires parsed source-first per-server definitions"
                        .to_owned(),
                });
            }
            continue;
        }
        for platform in &platforms {
            let contract = capability_contract_for(*platform, asset, &target_context);
            match contract.capability {
                PlatformCapability::DirectLink {
                    target,
                    mode,
                    copy_fallback_allowed,
                    surface,
                } => {
                    let id = projection_id(request, asset, surface);
                    let action = plan_direct_link(
                        DirectLinkIntent {
                            id,
                            source: asset.source_ref(),
                            consumers: contract.consumers,
                            target,
                            mode,
                            adapter_allows_copy_fallback: copy_fallback_allowed,
                        },
                        request.operation,
                        &mut warnings,
                        context,
                    )?;
                    // Shared physical targets (Cursor/Codex skills, Cursor/Hermes project rules)
                    // must be a single action even if more than one consumer was requested.
                    if !direct.iter().any(|existing: &ProjectionAction| {
                        existing.target == action.target && existing.members == action.members
                    }) {
                        direct.push(action);
                    }
                }
                PlatformCapability::Generated {
                    target,
                    mode,
                    surface,
                }
                | PlatformCapability::ExternalDirectory {
                    target,
                    mode,
                    surface,
                } => {
                    let id = projection_id(request, asset, surface);
                    register_generated_member(
                        &mut generated,
                        target,
                        mode,
                        generated_renderer(asset.kind, mode),
                        contract.consumers,
                        ProjectionMember {
                            id,
                            source: asset.source_ref(),
                            entry_key: None,
                        },
                    );
                }
                PlatformCapability::Unsupported { reason } => report_only.push(ProjectionAction {
                    kind: ProjectionActionKind::ReportOnly,
                    target: None,
                    precondition: None,
                    ownership_fingerprint: None,
                    generated_renderer: None,
                    members: vec![ProjectionMember {
                        id: projection_id(request, asset, ProjectionSurface::Platform(*platform)),
                        source: asset.source_ref(),
                        entry_key: None,
                    }],
                    mcp_members: Vec::new(),
                    consumers: Vec::new(),
                    state: Some("unsupported".to_owned()),
                    reason_code: "unsupported_platform_contract".to_owned(),
                    reason,
                }),
            }
        }
    }

    let mut actions = direct;
    for (_, mut batch) in generated {
        batch
            .members
            .sort_by_key(|member| projection_id_key(&member.id));
        let precondition = path_fingerprint(&batch.target.path)?;
        let action = if batch.renderer_conflict {
            ProjectionAction {
                kind: ProjectionActionKind::ReportOnly,
                target: Some(batch.target),
                precondition: Some(precondition),
                ownership_fingerprint: None,
                generated_renderer: None,
                members: batch.members,
                mcp_members: batch.mcp_members,
                consumers: batch.consumers,
                state: Some("conflict".to_owned()),
                reason_code: "generated_renderer_conflict".to_owned(),
                reason: "multiple generated renderers claim one normalized target".to_owned(),
            }
        } else {
            let ownership = if matches!(
                batch.renderer,
                GeneratedContainerRenderer::HookJson
                    | GeneratedContainerRenderer::HookToml
                    | GeneratedContainerRenderer::HookYaml
            ) {
                hook_generated_batch_ownership(&batch, &precondition, &mut warnings, context)
            } else {
                generated_batch_ownership(&batch, &precondition, &mut warnings, context)
            };
            match (request.operation, precondition.entry_type, ownership) {
                (
                    ProjectionOperation::Retract | ProjectionOperation::Uninstall,
                    super::model::FingerprintType::Missing,
                    _,
                ) => ProjectionAction {
                    kind: ProjectionActionKind::Noop,
                    target: Some(batch.target),
                    precondition: Some(precondition),
                    ownership_fingerprint: None,
                    generated_renderer: Some(batch.renderer),
                    members: batch.members,
                    mcp_members: batch.mcp_members,
                    consumers: batch.consumers,
                    state: Some("missing".to_owned()),
                    reason_code: "generated_target_already_absent".to_owned(),
                    reason: "generated container is already absent".to_owned(),
                },
                (
                    ProjectionOperation::Retract | ProjectionOperation::Uninstall,
                    _,
                    GeneratedOwnership::Managed | GeneratedOwnership::SourceChanged,
                ) => ProjectionAction {
                    kind: ProjectionActionKind::RemoveGeneratedEntries,
                    target: Some(batch.target),
                    precondition: Some(precondition),
                    ownership_fingerprint: None,
                    generated_renderer: Some(batch.renderer),
                    members: batch.members,
                    mcp_members: batch.mcp_members,
                    consumers: batch.consumers,
                    state: Some("managed_generated".to_owned()),
                    reason_code: "managed_generated_retract".to_owned(),
                    reason: "only ledger-owned generated entries may be retracted".to_owned(),
                },
                (
                    ProjectionOperation::Retract | ProjectionOperation::Uninstall,
                    _,
                    GeneratedOwnership::Drifted,
                ) => ProjectionAction {
                    kind: ProjectionActionKind::ReportOnly,
                    target: Some(batch.target),
                    precondition: Some(precondition),
                    ownership_fingerprint: None,
                    generated_renderer: Some(batch.renderer),
                    members: batch.members,
                    mcp_members: batch.mcp_members,
                    consumers: batch.consumers,
                    state: Some("drifted".to_owned()),
                    reason_code: "generated_target_drifted".to_owned(),
                    reason: "generated container changed after its recorded projection".to_owned(),
                },
                (
                    ProjectionOperation::Retract | ProjectionOperation::Uninstall,
                    _,
                    GeneratedOwnership::Equivalent,
                ) => ProjectionAction {
                    kind: ProjectionActionKind::ReportOnly,
                    target: Some(batch.target),
                    precondition: Some(precondition),
                    ownership_fingerprint: None,
                    generated_renderer: Some(batch.renderer),
                    members: batch.members,
                    mcp_members: batch.mcp_members,
                    consumers: batch.consumers,
                    state: Some("foreign".to_owned()),
                    reason_code: "generated_retract_ownership_unproven".to_owned(),
                    reason: "generated retract requires matching ledger ownership".to_owned(),
                },
                (
                    ProjectionOperation::Retract | ProjectionOperation::Uninstall,
                    _,
                    GeneratedOwnership::Foreign,
                ) => ProjectionAction {
                    kind: ProjectionActionKind::ReportOnly,
                    target: Some(batch.target),
                    precondition: Some(precondition),
                    ownership_fingerprint: None,
                    generated_renderer: Some(batch.renderer),
                    members: batch.members,
                    mcp_members: batch.mcp_members,
                    consumers: batch.consumers,
                    state: Some("foreign".to_owned()),
                    reason_code: "generated_retract_ownership_unproven".to_owned(),
                    reason: "generated retract requires matching ledger ownership".to_owned(),
                },
                (ProjectionOperation::Sync, super::model::FingerprintType::Missing, _) => {
                    ProjectionAction {
                        kind: ProjectionActionKind::UpsertGeneratedBatch,
                        target: Some(batch.target),
                        precondition: Some(precondition),
                        ownership_fingerprint: None,
                        generated_renderer: Some(batch.renderer),
                        members: batch.members,
                        mcp_members: batch.mcp_members,
                        consumers: batch.consumers,
                        state: Some("missing".to_owned()),
                        reason_code: "generated_target_missing".to_owned(),
                        reason: "generated container is missing".to_owned(),
                    }
                }
                (ProjectionOperation::Sync, _, GeneratedOwnership::Managed) => ProjectionAction {
                    kind: ProjectionActionKind::Noop,
                    target: Some(batch.target),
                    precondition: Some(precondition),
                    ownership_fingerprint: None,
                    generated_renderer: Some(batch.renderer),
                    members: batch.members,
                    mcp_members: batch.mcp_members,
                    consumers: batch.consumers,
                    state: Some("managed_generated".to_owned()),
                    reason_code: "managed_generated_noop".to_owned(),
                    reason: "all generated members match ledger fingerprints".to_owned(),
                },
                (ProjectionOperation::Sync, _, GeneratedOwnership::SourceChanged) => {
                    ProjectionAction {
                        kind: ProjectionActionKind::UpsertGeneratedBatch,
                        target: Some(batch.target),
                        precondition: Some(precondition),
                        ownership_fingerprint: None,
                        generated_renderer: Some(batch.renderer),
                        members: batch.members,
                        mcp_members: batch.mcp_members,
                        consumers: batch.consumers,
                        state: Some("managed_generated".to_owned()),
                        reason_code: "generated_source_changed".to_owned(),
                        reason: "canonical source changed since the recorded projection".to_owned(),
                    }
                }
                (ProjectionOperation::Sync, _, GeneratedOwnership::Missing)
                    if matches!(batch.renderer, GeneratedContainerRenderer::HookJson) =>
                {
                    ProjectionAction {
                        kind: ProjectionActionKind::UpsertGeneratedBatch,
                        target: Some(batch.target),
                        precondition: Some(precondition),
                        ownership_fingerprint: None,
                        generated_renderer: Some(batch.renderer),
                        members: batch.members,
                        mcp_members: batch.mcp_members,
                        consumers: batch.consumers,
                        state: Some("missing".to_owned()),
                        reason_code: "hook_binding_missing".to_owned(),
                        reason: "the named Hook binding is absent; foreign container entries are preserved".to_owned(),
                    }
                }
                (_, _, GeneratedOwnership::Missing) => ProjectionAction {
                    kind: ProjectionActionKind::ReportOnly,
                    target: Some(batch.target),
                    precondition: Some(precondition),
                    ownership_fingerprint: None,
                    generated_renderer: Some(batch.renderer),
                    members: batch.members,
                    mcp_members: batch.mcp_members,
                    consumers: batch.consumers,
                    state: Some("unsupported".to_owned()),
                    reason_code: "generated_member_operations_not_implemented".to_owned(),
                    reason: "generic generated member reconciliation is not implemented".to_owned(),
                },
                (ProjectionOperation::Sync, _, GeneratedOwnership::Drifted) => ProjectionAction {
                    kind: ProjectionActionKind::ReportOnly,
                    target: Some(batch.target),
                    precondition: Some(precondition),
                    ownership_fingerprint: None,
                    generated_renderer: Some(batch.renderer),
                    members: batch.members,
                    mcp_members: batch.mcp_members,
                    consumers: batch.consumers,
                    state: Some("drifted".to_owned()),
                    reason_code: "generated_target_drifted".to_owned(),
                    reason: "generated container changed after its recorded projection".to_owned(),
                },
                (
                    ProjectionOperation::Sync,
                    _,
                    GeneratedOwnership::Foreign | GeneratedOwnership::Equivalent,
                ) => ProjectionAction {
                    kind: ProjectionActionKind::ReportOnly,
                    target: Some(batch.target),
                    precondition: Some(precondition),
                    ownership_fingerprint: None,
                    generated_renderer: Some(batch.renderer),
                    members: batch.members,
                    mcp_members: batch.mcp_members,
                    consumers: batch.consumers,
                    state: Some("foreign".to_owned()),
                    reason_code: "generated_ownership_unproven".to_owned(),
                    reason: "generated container has no matching ownership record".to_owned(),
                },
                (
                    ProjectionOperation::CleanupOrphans
                    | ProjectionOperation::Import
                    | ProjectionOperation::Migrate,
                    _,
                    _,
                ) => {
                    unreachable!("cleanup/import/migrate return before generated batching")
                }
            }
        };
        actions.push(action);
    }
    actions.extend(report_only);
    if request.operation == ProjectionOperation::Sync {
        append_orphan_candidates(request, context, &mut warnings, &mut actions)?;
    }
    finish_plan(request, actions, warnings)
}

/// Build MCP actions exclusively from parsed, canonical per-server sources. The general asset
/// planner intentionally has no access to MCP `enabled`, `targets`, or secret-key metadata, so
/// it must not be used to create MCP container writes.
pub fn build_mcp_projection_plan(
    request: &ProjectionRequest,
    definitions: &[EffectiveMcpDefinition],
    context: &PlannerContext<'_>,
) -> Result<ProjectionPlan, CoreError> {
    let target_context = TargetContext {
        scope: request.scope,
        deploy_base: request.deploy_base.clone(),
    };
    let mut platforms = request.platforms.clone();
    platforms.sort_by_key(platform_key);
    platforms.dedup();
    let mut definitions = definitions.to_vec();
    definitions.sort_by(|left, right| {
        left.definition
            .server
            .name
            .cmp(&right.definition.server.name)
    });

    if matches!(
        request.operation,
        ProjectionOperation::Import | ProjectionOperation::Migrate
    ) {
        let actions = definitions
            .iter()
            .flat_map(|definition| {
                platforms
                    .iter()
                    .filter(move |platform| definition.definition.enabled_for(**platform))
                    .map(move |platform| {
                        mcp_report_only(
                            request,
                            &definition.source,
                            &definition.definition.server.name,
                            &definition.definition.server.secret_keys,
                            *platform,
                            "operation_not_implemented",
                            "MCP import/migrate execution is not implemented",
                        )
                    })
            })
            .collect();
        return finish_plan(request, actions, Vec::new());
    }

    let mut generated = BTreeMap::new();
    let mut actions = Vec::new();
    let mut warnings = Vec::new();
    for definition in &definitions {
        let server = &definition.definition.server;
        // Retraction is deliberately secret-free. A disabled server or an unavailable resolver
        // must not prevent removal of an already ledger-proven entry.
        let missing_secret_keys = if request.operation == ProjectionOperation::Sync {
            context.missing_mcp_secret_keys(&server.secret_keys)?
        } else {
            Vec::new()
        };
        for platform in &platforms {
            let active_for_platform = definition.definition.enabled_for(*platform);
            if !active_for_platform
                && !matches!(
                    request.operation,
                    ProjectionOperation::Retract | ProjectionOperation::Uninstall
                )
            {
                continue;
            }
            let asset = EffectiveAsset {
                kind: AssetKind::Mcp,
                name: server.name.clone(),
                source_path: definition.source.absolute_path.clone(),
                layer: definition.source.layer,
                fingerprint: definition.source.fingerprint.clone(),
            };
            let contract = capability_contract_for(*platform, &asset, &target_context);
            match contract.capability {
                PlatformCapability::Generated {
                    target,
                    mode,
                    surface,
                } => {
                    let Some(entry_key) = target.entry_key.clone() else {
                        actions.push(mcp_report_only(
                            request,
                            &definition.source,
                            &server.name,
                            &server.secret_keys,
                            *platform,
                            "mcp_entry_key_missing",
                            "MCP platform contract must provide a named container entry",
                        ));
                        continue;
                    };
                    let id = projection_id(request, &asset, surface);
                    if !active_for_platform
                        && !mcp_retract_has_ledger_candidate(context, &id, &mut warnings)
                    {
                        // A currently disabled/non-targeted source must not manufacture a
                        // retract candidate. Only its existing ledger identity can make it
                        // addressable for an explicit retract/uninstall operation.
                        continue;
                    }
                    if request.operation == ProjectionOperation::Sync
                        && !missing_secret_keys.is_empty()
                    {
                        actions.push(mcp_missing_secret_report_only(
                            request,
                            &definition.source,
                            &server.name,
                            &server.secret_keys,
                            &missing_secret_keys,
                            &entry_key,
                            *platform,
                        ));
                        continue;
                    }
                    let mut secret_keys = server.secret_keys.clone();
                    secret_keys.sort();
                    secret_keys.dedup();
                    register_mcp_generated_member(
                        &mut generated,
                        target,
                        mode,
                        generated_renderer(AssetKind::Mcp, mode),
                        contract.consumers,
                        McpProjectionMember {
                            id,
                            name: server.name.clone(),
                            source: definition.source.clone(),
                            entry_key,
                            secret_keys,
                            missing_secret_keys: Vec::new(),
                        },
                    );
                }
                PlatformCapability::Unsupported { reason } => actions.push(mcp_report_only(
                    request,
                    &definition.source,
                    &server.name,
                    &server.secret_keys,
                    *platform,
                    "unsupported_platform_contract",
                    &reason,
                )),
                PlatformCapability::DirectLink { .. }
                | PlatformCapability::ExternalDirectory { .. } => actions.push(mcp_report_only(
                    request,
                    &definition.source,
                    &server.name,
                    &server.secret_keys,
                    *platform,
                    "mcp_requires_generated_container",
                    "MCP platform contracts must use a named generated container",
                )),
            }
        }
    }

    for (_, mut batch) in generated {
        batch
            .mcp_members
            .sort_by_key(|member| projection_id_key(&member.id));
        if request.operation == ProjectionOperation::Sync {
            let mut remaining = batch.clone_without_members();
            for member in &batch.mcp_members {
                let candidate = batch.clone_with_members(vec![member.clone()]);
                if mcp_generated_batch_ownership(
                    &candidate,
                    &path_fingerprint(&candidate.target.path)?,
                    &mut warnings,
                    context,
                ) == GeneratedOwnership::Equivalent
                {
                    actions.push(plan_mcp_generated_batch(
                        request,
                        &candidate,
                        &mut warnings,
                        context,
                    )?);
                } else {
                    remaining.mcp_members.push(member.clone());
                }
            }
            if !remaining.mcp_members.is_empty() {
                actions.push(plan_mcp_generated_batch(
                    request,
                    &remaining,
                    &mut warnings,
                    context,
                )?);
            }
        } else {
            actions.push(plan_mcp_generated_batch(
                request,
                &batch,
                &mut warnings,
                context,
            )?);
        }
    }
    finish_plan(request, actions, warnings)
}

/// Build the T008 user-scoped Hermes cross-domain slice as one plan. The ordinary planner and
/// the MCP-only planner intentionally remain separate until this explicit coordinator is used;
/// callers must never sequence their two plans against the same `config.yaml`.
pub fn build_hermes_cross_domain_projection_plan(
    request: &ProjectionRequest,
    definitions: &[EffectiveMcpDefinition],
    context: &PlannerContext<'_>,
) -> Result<ProjectionPlan, CoreError> {
    if request.scope != DeploymentScope::User || !request.platforms.contains(&PlatformId::Hermes) {
        let mut actions = Vec::new();
        for asset in &request.assets {
            actions.push(ProjectionAction {
                kind: ProjectionActionKind::ReportOnly,
                target: None,
                precondition: None,
                ownership_fingerprint: None,
                generated_renderer: None,
                members: vec![ProjectionMember {
                    id: projection_id(request, asset, ProjectionSurface::Platform(PlatformId::Hermes)),
                    source: asset.source_ref(),
                    entry_key: None,
                }],
                mcp_members: Vec::new(),
                consumers: Vec::new(),
                state: Some("unsupported".to_owned()),
                reason_code: "hermes_cross_domain_requires_user_scope".to_owned(),
                reason: "Hermes cross-domain configuration is user-scoped; project/workspace must not write global config.yaml".to_owned(),
            });
        }
        for definition in definitions {
            actions.push(mcp_report_only(
                request,
                &definition.source,
                &definition.definition.server.name,
                &definition.definition.server.secret_keys,
                PlatformId::Hermes,
                "unsupported_platform_contract",
                "Hermes project/workspace MCP is unsupported; do not modify global config.yaml",
            ));
        }
        return finish_plan(request, actions, Vec::new());
    }

    let target = ProjectionTarget {
        path: normalized_target_path(&request.deploy_base.join(".hermes/config.yaml")),
        entry_key: None,
    };
    let target_context = TargetContext {
        scope: request.scope,
        deploy_base: request.deploy_base.clone(),
    };
    let mut direct = Vec::new();
    let mut members = Vec::new();
    let mut warnings = Vec::new();

    for asset in &request.assets {
        match asset.kind {
            AssetKind::Skill => {
                let contract = capability_contract_for(PlatformId::Hermes, asset, &target_context);
                let PlatformCapability::ExternalDirectory {
                    target: external, ..
                } = contract.capability
                else {
                    return Err(CoreError::InvalidPath(
                        "Hermes skill contract must be external_dirs".to_owned(),
                    ));
                };
                if normalized_target_path(&external.path) != target.path {
                    return Err(CoreError::InvalidPath(
                        "Hermes external_dirs target is outside shared config.yaml".to_owned(),
                    ));
                }
                members.push(ProjectionMember {
                    id: projection_id(
                        request,
                        asset,
                        ProjectionSurface::Platform(PlatformId::Hermes),
                    ),
                    source: asset.source_ref(),
                    entry_key: external.entry_key,
                });
            }
            AssetKind::Hook => {
                let contract =
                    hook_capability_contract_for(PlatformId::Hermes, asset, &target_context);
                let HookCapability::Supported { binding, script } = contract.capability else {
                    return Err(CoreError::InvalidPath(
                        "Hermes user Hook contract is unexpectedly unsupported".to_owned(),
                    ));
                };
                let PlatformCapability::Generated {
                    target: binding_target,
                    ..
                } = binding
                else {
                    return Err(CoreError::InvalidPath(
                        "Hermes Hook binding must be generated".to_owned(),
                    ));
                };
                if normalized_target_path(&binding_target.path) != target.path {
                    return Err(CoreError::InvalidPath(
                        "Hermes Hook binding is outside shared config.yaml".to_owned(),
                    ));
                }
                let PlatformCapability::DirectLink {
                    target: script_target,
                    mode,
                    copy_fallback_allowed,
                    ..
                } = script
                else {
                    return Err(CoreError::InvalidPath(
                        "Hermes Hook script must be direct-linked".to_owned(),
                    ));
                };
                direct.push(plan_direct_link(
                    DirectLinkIntent {
                        id: projection_id(
                            request,
                            asset,
                            ProjectionSurface::PlatformBinding {
                                platform: PlatformId::Hermes,
                                binding: "hook_script".to_owned(),
                            },
                        ),
                        source: asset.source_ref(),
                        consumers: vec![PlatformId::Hermes],
                        target: script_target,
                        mode,
                        adapter_allows_copy_fallback: copy_fallback_allowed,
                    },
                    request.operation,
                    &mut warnings,
                    context,
                )?);
                members.push(ProjectionMember {
                    id: projection_id(
                        request,
                        asset,
                        ProjectionSurface::PlatformBinding {
                            platform: PlatformId::Hermes,
                            binding: "hook_binding".to_owned(),
                        },
                    ),
                    source: asset.source_ref(),
                    entry_key: binding_target.entry_key,
                });
            }
            _ => {}
        }
    }

    let mut mcp_members = Vec::new();
    for definition in definitions {
        if !definition.definition.enabled_for(PlatformId::Hermes) {
            continue;
        }
        let asset = EffectiveAsset {
            kind: AssetKind::Mcp,
            name: definition.definition.server.name.clone(),
            source_path: definition.source.absolute_path.clone(),
            layer: definition.source.layer,
            fingerprint: definition.source.fingerprint.clone(),
        };
        let contract = capability_contract_for(PlatformId::Hermes, &asset, &target_context);
        let PlatformCapability::Generated {
            target: mcp_target,
            mode,
            surface,
        } = contract.capability
        else {
            return Err(CoreError::InvalidPath(
                "Hermes user MCP must be generated".to_owned(),
            ));
        };
        if mode != ProjectionMode::GeneratedYaml
            || normalized_target_path(&mcp_target.path) != target.path
        {
            return Err(CoreError::InvalidPath(
                "Hermes MCP is outside shared config.yaml".to_owned(),
            ));
        }
        let mut secret_keys = definition.definition.server.secret_keys.clone();
        secret_keys.sort();
        secret_keys.dedup();
        mcp_members.push(McpProjectionMember {
            id: projection_id(request, &asset, surface),
            name: definition.definition.server.name.clone(),
            source: definition.source.clone(),
            entry_key: mcp_target
                .entry_key
                .unwrap_or_else(|| format!("mcp_servers.{}", asset.name)),
            secret_keys,
            missing_secret_keys: Vec::new(),
        });
    }

    let mut actions = direct;
    if !members.is_empty() || !mcp_members.is_empty() {
        let precondition = path_fingerprint(&target.path)?;
        let retracting = matches!(
            request.operation,
            ProjectionOperation::Retract | ProjectionOperation::Uninstall
        );
        let ownership_conflict = if retracting {
            hermes_cross_domain_retract_reason(
                &target,
                &members,
                &mcp_members,
                context,
                &mut warnings,
            )?
        } else {
            hermes_cross_domain_foreign_entry_reason(
                &target,
                &members,
                &mcp_members,
                context,
                &mut warnings,
            )?
        };
        actions.push(ProjectionAction {
            kind: if ownership_conflict.is_some() {
                ProjectionActionKind::ReportOnly
            } else if retracting {
                ProjectionActionKind::RemoveGeneratedEntries
            } else {
                ProjectionActionKind::UpsertGeneratedBatch
            },
            target: Some(target),
            precondition: Some(precondition),
            ownership_fingerprint: None,
            generated_renderer: Some(GeneratedContainerRenderer::HermesUnifiedYaml),
            members,
            mcp_members,
            consumers: vec![PlatformId::Hermes],
            state: Some(
                if ownership_conflict.is_some() {
                    "foreign"
                } else if retracting {
                    "managed_generated"
                } else {
                    "missing"
                }
                .to_owned(),
            ),
            reason_code: if ownership_conflict.is_some() {
                "hermes_cross_domain_ownership_unproven".to_owned()
            } else if retracting {
                "hermes_cross_domain_retract".to_owned()
            } else {
                "hermes_cross_domain_batch".to_owned()
            },
            reason: ownership_conflict.unwrap_or_else(|| {
                if retracting {
                    "only unchanged ledger-proven Hermes MCP, external_dirs and Hook bindings may be retracted".to_owned()
                } else {
                    "MCP, external skill directories and Hook bindings share one Hermes YAML transaction".to_owned()
                }
            }),
        });
    }
    finish_plan(request, actions, warnings)
}

/// Retraction is stricter than upsert: every desired cross-domain entry must still be present,
/// marked where applicable, and covered by an unchanged ledger record before any YAML or link is
/// removed. A missing ledger or any drift blocks the whole transaction.
fn hermes_cross_domain_retract_reason(
    target: &ProjectionTarget,
    members: &[ProjectionMember],
    mcp_members: &[McpProjectionMember],
    context: &PlannerContext<'_>,
    warnings: &mut Vec<PlanWarning>,
) -> Result<Option<String>, CoreError> {
    if !target.path.is_file() {
        return Ok(Some(
            "Hermes config.yaml is absent; unified ownership is not proven".to_owned(),
        ));
    }
    let current = path_fingerprint(&target.path)?;
    let Some(digest) = current.digest.as_deref() else {
        return Ok(Some(
            "Hermes config.yaml has no stable ownership digest".to_owned(),
        ));
    };
    let document =
        serde_yaml::from_str::<serde_yaml::Value>(&fs::read_to_string(target.path.as_std_path())?)
            .map_err(|_| {
                CoreError::InvalidPath("Hermes config.yaml cannot be parsed".to_owned())
            })?;
    let Some(root) = document.as_mapping() else {
        return Ok(Some(
            "Hermes config.yaml root is not a mapping; ownership is not proven".to_owned(),
        ));
    };
    let key = |name: &str| serde_yaml::Value::String(name.to_owned());
    let mut record_is_current = |id: &ProjectionId, entry_key: Option<&str>| {
        ledger_record(context, id, warnings).is_some_and(|record| {
            record.mode == ProjectionMode::GeneratedYaml
                && record.target_path == target.path
                && record.entry_key.as_deref() == entry_key
                && record.target_fingerprint == digest
        })
    };
    let servers = root
        .get(key("mcp_servers"))
        .and_then(serde_yaml::Value::as_mapping);
    for member in mcp_members {
        if !servers.is_some_and(|servers| servers.contains_key(key(&member.name)))
            || !record_is_current(&member.id, Some(&member.entry_key))
        {
            return Ok(Some(
                "Hermes MCP retraction requires an unchanged ledger-proven named server".to_owned(),
            ));
        }
    }
    let external_dirs = root
        .get(key("skills"))
        .and_then(serde_yaml::Value::as_mapping)
        .and_then(|skills| skills.get(key("external_dirs")))
        .and_then(serde_yaml::Value::as_sequence);
    let hooks = root
        .get(key("hooks"))
        .and_then(serde_yaml::Value::as_mapping);
    for member in members {
        let present = match member.id.kind {
            AssetKind::Skill => external_dirs.is_some_and(|dirs| {
                dirs.iter()
                    .any(|entry| entry.as_str() == Some(member.source.absolute_path.as_str()))
            }),
            AssetKind::Hook => hooks.is_some_and(|hooks| {
                hooks.values().any(|entries| {
                    entries.as_sequence().is_some_and(|entries| {
                        entries.iter().any(|entry| {
                            entry.as_mapping().is_some_and(|entry| {
                                entry
                                    .get(key("managedBy"))
                                    .and_then(serde_yaml::Value::as_str)
                                    == Some("agents-manager")
                                    && entry.get(key("hook")).and_then(serde_yaml::Value::as_str)
                                        == Some(member.id.name.as_str())
                            })
                        })
                    })
                })
            }),
            _ => false,
        };
        if !present || !record_is_current(&member.id, member.entry_key.as_deref()) {
            return Ok(Some(
                "Hermes generated retraction requires unchanged ledger-proven entries".to_owned(),
            ));
        }
    }
    Ok(None)
}

/// The unified renderer may append a new named entry to a foreign YAML container, but may never
/// replace a same-name MCP server or a marked Hook binding unless the ledger already proves it.
fn hermes_cross_domain_foreign_entry_reason(
    target: &ProjectionTarget,
    members: &[ProjectionMember],
    mcp_members: &[McpProjectionMember],
    context: &PlannerContext<'_>,
    warnings: &mut Vec<PlanWarning>,
) -> Result<Option<String>, CoreError> {
    if !target.path.is_file() {
        return Ok(None);
    }
    let content = fs::read_to_string(target.path.as_std_path())?;
    let current_digest = path_fingerprint(&target.path)?.digest;
    let document = match serde_yaml::from_str::<serde_yaml::Value>(&content) {
        Ok(document) => document,
        Err(_) => {
            return Ok(Some(
                "Hermes config.yaml cannot be parsed; ownership is not proven".to_owned(),
            ))
        }
    };
    let Some(root) = document.as_mapping() else {
        return Ok(Some(
            "Hermes config.yaml root is not a mapping; ownership is not proven".to_owned(),
        ));
    };
    let key = |name: &str| serde_yaml::Value::String(name.to_owned());
    if let Some(servers) = root
        .get(key("mcp_servers"))
        .and_then(serde_yaml::Value::as_mapping)
    {
        for member in mcp_members {
            if !servers.contains_key(key(&member.name)) {
                continue;
            }
            let Some(record) = ledger_record(context, &member.id, warnings) else {
                return Ok(Some(
                    "a same-name Hermes MCP server exists without ledger ownership".to_owned(),
                ));
            };
            if record.mode != ProjectionMode::GeneratedYaml
                || record.target_path != target.path
                || record.entry_key.as_deref() != Some(member.entry_key.as_str())
                || record.target_fingerprint != current_digest.clone().unwrap_or_default()
            {
                return Ok(Some(
                    "a same-name Hermes MCP server is not ledger-proven for this target".to_owned(),
                ));
            }
        }
    }
    if let Some(hooks) = root
        .get(key("hooks"))
        .and_then(serde_yaml::Value::as_mapping)
    {
        for member in members
            .iter()
            .filter(|member| member.id.kind == AssetKind::Hook)
        {
            let marked_binding_exists = hooks.values().any(|entries| {
                entries.as_sequence().is_some_and(|entries| {
                    entries.iter().any(|entry| {
                        entry.as_mapping().is_some_and(|entry| {
                            entry
                                .get(key("managedBy"))
                                .and_then(serde_yaml::Value::as_str)
                                == Some("agents-manager")
                                && entry.get(key("hook")).and_then(serde_yaml::Value::as_str)
                                    == Some(member.id.name.as_str())
                        })
                    })
                })
            });
            if !marked_binding_exists {
                continue;
            }
            let Some(record) = ledger_record(context, &member.id, warnings) else {
                return Ok(Some(
                    "a marked Hermes Hook binding exists without ledger ownership".to_owned(),
                ));
            };
            if record.mode != ProjectionMode::GeneratedYaml
                || record.target_path != target.path
                || record.target_fingerprint != current_digest.clone().unwrap_or_default()
            {
                return Ok(Some(
                    "a marked Hermes Hook binding is not ledger-proven for this target".to_owned(),
                ));
            }
        }
    }
    Ok(None)
}

fn mcp_retract_has_ledger_candidate(
    context: &PlannerContext<'_>,
    id: &ProjectionId,
    warnings: &mut Vec<PlanWarning>,
) -> bool {
    match context.ledger.get(id) {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(_) => {
            if !warnings
                .iter()
                .any(|warning| warning.code == "ledger_unavailable")
            {
                warnings.push(PlanWarning {
                    code: "ledger_unavailable".to_owned(),
                    message: "projection ledger is unavailable; ownership is not proven".to_owned(),
                });
            }
            // Continue to the regular entry-level ownership check, which becomes report-only
            // when the ledger cannot provide proof. This is safer than silently hiding a
            // requested retract candidate.
            true
        }
    }
}

fn mcp_report_only(
    request: &ProjectionRequest,
    source: &SourceRef,
    name: &str,
    secret_keys: &[String],
    platform: PlatformId,
    reason_code: &str,
    reason: &str,
) -> ProjectionAction {
    let mut secret_keys = secret_keys.to_vec();
    secret_keys.sort();
    secret_keys.dedup();
    ProjectionAction {
        kind: ProjectionActionKind::ReportOnly,
        target: None,
        precondition: None,
        ownership_fingerprint: None,
        generated_renderer: None,
        members: Vec::new(),
        mcp_members: vec![McpProjectionMember {
            id: ProjectionId {
                scope_key: request.scope_key.clone(),
                kind: AssetKind::Mcp,
                name: name.to_owned(),
                surface: ProjectionSurface::Platform(platform),
            },
            name: name.to_owned(),
            source: source.clone(),
            entry_key: String::new(),
            secret_keys,
            missing_secret_keys: Vec::new(),
        }],
        consumers: Vec::new(),
        state: Some("unsupported".to_owned()),
        reason_code: reason_code.to_owned(),
        reason: reason.to_owned(),
    }
}

fn mcp_missing_secret_report_only(
    request: &ProjectionRequest,
    source: &SourceRef,
    name: &str,
    secret_keys: &[String],
    missing_secret_keys: &[String],
    entry_key: &str,
    platform: PlatformId,
) -> ProjectionAction {
    let mut action = mcp_report_only(
        request,
        source,
        name,
        secret_keys,
        platform,
        "mcp_missing_secret_keys",
        "MCP server is skipped because declared secret keys are unavailable",
    );
    let member = action
        .mcp_members
        .first_mut()
        .expect("MCP report action has one member");
    member.entry_key = entry_key.to_owned();
    member.missing_secret_keys = missing_secret_keys.to_vec();
    member.missing_secret_keys.sort();
    member.missing_secret_keys.dedup();
    action.state = Some("skipped".to_owned());
    action
}

fn plan_mcp_generated_batch(
    request: &ProjectionRequest,
    batch: &GeneratedBatch,
    warnings: &mut Vec<PlanWarning>,
    context: &PlannerContext<'_>,
) -> Result<ProjectionAction, CoreError> {
    let precondition = path_fingerprint(&batch.target.path)?;
    if batch.renderer_conflict {
        return Ok(mcp_batch_action(
            batch,
            ProjectionActionKind::ReportOnly,
            precondition,
            Some("conflict"),
            "generated_renderer_conflict",
            "multiple generated renderers claim one normalized target",
        ));
    }
    let ownership = mcp_generated_batch_ownership(batch, &precondition, warnings, context);
    let action = match (request.operation, precondition.entry_type, ownership) {
        (
            ProjectionOperation::Retract | ProjectionOperation::Uninstall,
            super::model::FingerprintType::Missing,
            _,
        ) => (
            ProjectionActionKind::Noop,
            Some("missing"),
            "generated_target_already_absent",
            "generated container is already absent",
        ),
        (
            ProjectionOperation::Retract | ProjectionOperation::Uninstall,
            _,
            GeneratedOwnership::Managed | GeneratedOwnership::SourceChanged,
        ) => (
            ProjectionActionKind::RemoveGeneratedEntries,
            Some("managed_generated"),
            "managed_generated_retract",
            "only ledger-owned generated entries may be retracted",
        ),
        (ProjectionOperation::Sync, super::model::FingerprintType::Missing, _) => (
            ProjectionActionKind::UpsertGeneratedBatch,
            Some("missing"),
            "generated_target_missing",
            "generated container is missing",
        ),
        (ProjectionOperation::Sync, _, GeneratedOwnership::Managed) => (
            ProjectionActionKind::Noop,
            Some("managed_generated"),
            "managed_generated_noop",
            "all generated members match ledger entry fingerprints",
        ),
        (ProjectionOperation::Sync, _, GeneratedOwnership::SourceChanged) => (
            ProjectionActionKind::UpsertGeneratedBatch,
            Some("managed_generated"),
            "generated_source_changed",
            "canonical source changed since the recorded projection",
        ),
        (ProjectionOperation::Sync, _, GeneratedOwnership::Equivalent) => (
            ProjectionActionKind::AdoptEquivalent,
            Some("equivalent"),
            "equivalent_mcp_entries_unmanaged",
            "all unmanaged MCP entries are semantically equivalent to canonical sources",
        ),
        (ProjectionOperation::Sync, _, GeneratedOwnership::Missing) => (
            ProjectionActionKind::UpsertGeneratedBatch,
            Some("missing"),
            "generated_entries_missing",
            "one or more desired MCP entries are absent; unrelated container entries are preserved",
        ),
        (
            ProjectionOperation::CleanupOrphans
            | ProjectionOperation::Import
            | ProjectionOperation::Migrate,
            _,
            _,
        ) => (
            ProjectionActionKind::ReportOnly,
            Some("unsupported"),
            "operation_not_implemented",
            "MCP cleanup/import/migrate execution is not implemented",
        ),
        (_, _, GeneratedOwnership::Drifted) => (
            ProjectionActionKind::ReportOnly,
            Some("drifted"),
            "generated_target_drifted",
            "a managed MCP entry changed after its recorded projection",
        ),
        (_, _, GeneratedOwnership::Foreign) => (
            ProjectionActionKind::ReportOnly,
            Some("foreign"),
            "generated_ownership_unproven",
            "MCP generated entries require matching entry-level ledger proof",
        ),
        (_, _, GeneratedOwnership::Equivalent) => (
            ProjectionActionKind::ReportOnly,
            Some("foreign"),
            "generated_ownership_unproven",
            "equivalent MCP entries require explicit adoption before non-sync operations",
        ),
        (_, _, GeneratedOwnership::Missing) => (
            ProjectionActionKind::ReportOnly,
            Some("unsupported"),
            "mcp_member_operations_not_implemented",
            "MCP non-sync member reconciliation is not implemented",
        ),
    };
    Ok(mcp_batch_action(
        batch,
        action.0,
        precondition,
        action.1,
        action.2,
        action.3,
    ))
}

fn mcp_batch_action(
    batch: &GeneratedBatch,
    kind: ProjectionActionKind,
    precondition: PathFingerprint,
    state: Option<&str>,
    reason_code: &str,
    reason: &str,
) -> ProjectionAction {
    ProjectionAction {
        kind,
        target: Some(batch.target.clone()),
        precondition: Some(precondition),
        ownership_fingerprint: None,
        generated_renderer: Some(batch.renderer),
        members: Vec::new(),
        mcp_members: batch.mcp_members.clone(),
        consumers: batch.consumers.clone(),
        state: state.map(str::to_owned),
        reason_code: reason_code.to_owned(),
        reason: reason.to_owned(),
    }
}

fn finish_plan(
    request: &ProjectionRequest,
    mut actions: Vec<ProjectionAction>,
    warnings: Vec<PlanWarning>,
) -> Result<ProjectionPlan, CoreError> {
    actions.sort_by_key(action_key);
    let digest_actions = actions
        .iter()
        .map(PlanDigestAction::from_action)
        .collect::<Vec<_>>();
    let action_ids = digest_actions
        .iter()
        .map(stable_action_id)
        .collect::<Vec<_>>();
    let trust_requirements = actions
        .iter()
        .map(|action| trust_requirement_for_action(request, action))
        .collect::<Vec<_>>();
    let digest_input = PlanDigestInput {
        schema_version: PROJECTION_PLAN_SCHEMA_VERSION,
        actions: digest_actions,
    };
    let encoded = serde_json::to_vec(&digest_input)
        .map_err(|error| CoreError::InvalidPath(format!("serialize projection plan: {error}")))?;
    let plan_digest = hex::encode(Sha256::digest(encoded));
    Ok(ProjectionPlan {
        schema_version: PROJECTION_PLAN_SCHEMA_VERSION,
        actions,
        action_ids,
        trust_requirements,
        warnings,
        plan_digest,
    })
}

#[derive(Debug)]
struct GeneratedBatch {
    target: ProjectionTarget,
    mode: ProjectionMode,
    renderer: GeneratedContainerRenderer,
    consumers: Vec<PlatformId>,
    members: Vec<ProjectionMember>,
    mcp_members: Vec<McpProjectionMember>,
    renderer_conflict: bool,
}

impl GeneratedBatch {
    fn clone_without_members(&self) -> Self {
        Self {
            target: self.target.clone(),
            mode: self.mode,
            renderer: self.renderer,
            consumers: self.consumers.clone(),
            members: Vec::new(),
            mcp_members: Vec::new(),
            renderer_conflict: self.renderer_conflict,
        }
    }

    fn clone_with_members(&self, mcp_members: Vec<McpProjectionMember>) -> Self {
        Self {
            mcp_members,
            ..self.clone_without_members()
        }
    }
}

fn register_generated_member(
    generated: &mut BTreeMap<String, GeneratedBatch>,
    mut target: ProjectionTarget,
    mode: ProjectionMode,
    renderer: GeneratedContainerRenderer,
    consumers: Vec<PlatformId>,
    mut member: ProjectionMember,
) {
    target.path = normalized_target_path(&target.path);
    let key = target.path.as_str().to_owned();
    member.entry_key = target.entry_key.clone();
    let batch = generated.entry(key).or_insert_with(|| GeneratedBatch {
        target: ProjectionTarget {
            path: target.path.clone(),
            entry_key: None,
        },
        mode,
        renderer,
        consumers: consumers.clone(),
        members: Vec::new(),
        mcp_members: Vec::new(),
        renderer_conflict: false,
    });
    if batch.mode != mode || batch.renderer != renderer {
        batch.renderer_conflict = true;
    }
    merge_consumers(&mut batch.consumers, &consumers);
    if !batch
        .members
        .iter()
        .any(|existing| existing.id == member.id)
    {
        batch.members.push(member);
    }
}

fn register_mcp_generated_member(
    generated: &mut BTreeMap<String, GeneratedBatch>,
    mut target: ProjectionTarget,
    mode: ProjectionMode,
    renderer: GeneratedContainerRenderer,
    consumers: Vec<PlatformId>,
    member: McpProjectionMember,
) {
    target.path = normalized_target_path(&target.path);
    let key = target.path.as_str().to_owned();
    let batch = generated.entry(key).or_insert_with(|| GeneratedBatch {
        target: ProjectionTarget {
            path: target.path.clone(),
            entry_key: None,
        },
        mode,
        renderer,
        consumers: consumers.clone(),
        members: Vec::new(),
        mcp_members: Vec::new(),
        renderer_conflict: false,
    });
    if batch.mode != mode || batch.renderer != renderer || !batch.members.is_empty() {
        batch.renderer_conflict = true;
    }
    merge_consumers(&mut batch.consumers, &consumers);
    if !batch
        .mcp_members
        .iter()
        .any(|existing| existing.id == member.id)
    {
        batch.mcp_members.push(member);
    }
}

/// T007 keeps MCP and Hook renderers separate even where they happen to target the same format.
/// T008 may explicitly introduce a shared cross-domain adapter only after it can preserve both
/// schemas in one parse/render transaction.
fn generated_renderer(kind: AssetKind, mode: ProjectionMode) -> GeneratedContainerRenderer {
    match (kind, mode) {
        (AssetKind::Mcp, ProjectionMode::GeneratedJson) => GeneratedContainerRenderer::McpJson,
        (AssetKind::Mcp, ProjectionMode::GeneratedToml) => GeneratedContainerRenderer::McpToml,
        (AssetKind::Mcp, ProjectionMode::GeneratedYaml) => GeneratedContainerRenderer::McpYaml,
        (AssetKind::Hook, ProjectionMode::GeneratedJson) => GeneratedContainerRenderer::HookJson,
        (AssetKind::Hook, ProjectionMode::GeneratedToml) => GeneratedContainerRenderer::HookToml,
        (AssetKind::Hook, ProjectionMode::GeneratedYaml) => GeneratedContainerRenderer::HookYaml,
        (_, ProjectionMode::GeneratedJson) => GeneratedContainerRenderer::GenericJson,
        (_, ProjectionMode::GeneratedToml) => GeneratedContainerRenderer::GenericToml,
        (_, ProjectionMode::GeneratedYaml) => GeneratedContainerRenderer::GenericYaml,
        (_, ProjectionMode::GeneratedMarkdown) => GeneratedContainerRenderer::Markdown,
        (_, ProjectionMode::ExternalDirectory) => GeneratedContainerRenderer::ExternalDirectory,
        (_, ProjectionMode::DirectLink | ProjectionMode::CopyFallback) => {
            unreachable!("direct/copy modes are not generated containers")
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn plan_hook_asset(
    request: &ProjectionRequest,
    asset: &EffectiveAsset,
    platforms: &[PlatformId],
    target_context: &TargetContext,
    direct: &mut Vec<ProjectionAction>,
    generated: &mut BTreeMap<String, GeneratedBatch>,
    report_only: &mut Vec<ProjectionAction>,
    warnings: &mut Vec<PlanWarning>,
    context: &PlannerContext<'_>,
) -> Result<(), CoreError> {
    for platform in platforms {
        let contract = hook_capability_contract_for(*platform, asset, target_context);
        match contract.capability {
            HookCapability::Supported { binding, script } => {
                let (binding_target, binding_mode) = match binding {
                    PlatformCapability::Generated { target, mode, .. } => (target, mode),
                    _ => {
                        report_only.push(hook_report_only(
                            request,
                            asset,
                            *platform,
                            "hook_binding_contract_invalid",
                            "hook binding must be a generated named entry",
                        ));
                        continue;
                    }
                };
                let (script_target, script_mode, script_copy_fallback_allowed) = match script {
                    PlatformCapability::DirectLink {
                        target,
                        mode,
                        copy_fallback_allowed,
                        ..
                    } => (target, mode, copy_fallback_allowed),
                    _ => {
                        report_only.push(hook_report_only(
                            request,
                            asset,
                            *platform,
                            "hook_script_contract_invalid",
                            "hook script must be a direct-link unit",
                        ));
                        continue;
                    }
                };
                register_generated_member(
                    generated,
                    binding_target,
                    binding_mode,
                    generated_renderer(AssetKind::Hook, binding_mode),
                    contract.consumers.clone(),
                    ProjectionMember {
                        id: projection_id(
                            request,
                            asset,
                            ProjectionSurface::PlatformBinding {
                                platform: *platform,
                                binding: "hook_binding".to_owned(),
                            },
                        ),
                        source: asset.source_ref(),
                        entry_key: None,
                    },
                );
                direct.push(plan_direct_link(
                    DirectLinkIntent {
                        id: projection_id(
                            request,
                            asset,
                            ProjectionSurface::PlatformBinding {
                                platform: *platform,
                                binding: "hook_script".to_owned(),
                            },
                        ),
                        source: asset.source_ref(),
                        consumers: contract.consumers,
                        target: script_target,
                        mode: script_mode,
                        adapter_allows_copy_fallback: script_copy_fallback_allowed,
                    },
                    request.operation,
                    warnings,
                    context,
                )?);
            }
            HookCapability::Unsupported { reason } => report_only.push(ProjectionAction {
                kind: ProjectionActionKind::ReportOnly,
                target: None,
                precondition: None,
                ownership_fingerprint: None,
                generated_renderer: None,
                members: vec![ProjectionMember {
                    id: projection_id(request, asset, ProjectionSurface::Platform(*platform)),
                    source: asset.source_ref(),
                    entry_key: None,
                }],
                mcp_members: Vec::new(),
                consumers: Vec::new(),
                state: Some("unsupported".to_owned()),
                reason_code: "unsupported_hook_contract".to_owned(),
                reason,
            }),
        }
    }
    Ok(())
}

fn hook_report_only(
    request: &ProjectionRequest,
    asset: &EffectiveAsset,
    platform: PlatformId,
    reason_code: &str,
    reason: &str,
) -> ProjectionAction {
    ProjectionAction {
        kind: ProjectionActionKind::ReportOnly,
        target: None,
        precondition: None,
        ownership_fingerprint: None,
        generated_renderer: None,
        members: vec![ProjectionMember {
            id: projection_id(request, asset, ProjectionSurface::Platform(platform)),
            source: asset.source_ref(),
            entry_key: None,
        }],
        mcp_members: Vec::new(),
        consumers: Vec::new(),
        state: Some("conflict".to_owned()),
        reason_code: reason_code.to_owned(),
        reason: reason.to_owned(),
    }
}

struct DirectLinkIntent {
    id: ProjectionId,
    source: SourceRef,
    consumers: Vec<PlatformId>,
    target: ProjectionTarget,
    mode: ProjectionMode,
    /// The adapter returned a dedicated direct-link target; only that explicit contract may be
    /// considered for a policy-authorized fallback.
    adapter_allows_copy_fallback: bool,
}

fn plan_project_entry_prompt(
    request: &ProjectionRequest,
    asset: &EffectiveAsset,
    platforms: &[PlatformId],
    direct: &mut Vec<ProjectionAction>,
    report_only: &mut Vec<ProjectionAction>,
    warnings: &mut Vec<PlanWarning>,
    context: &PlannerContext<'_>,
) -> Result<(), CoreError> {
    if request.scope != DeploymentScope::Project {
        report_only.push(ProjectionAction {
            kind: ProjectionActionKind::ReportOnly,
            target: None,
            precondition: None,
            ownership_fingerprint: None,
            generated_renderer: None,
            members: vec![ProjectionMember {
                id: projection_id(request, asset, ProjectionSurface::ProjectEntry),
                source: asset.source_ref(),
                entry_key: None,
            }],
            mcp_members: Vec::new(),
            consumers: Vec::new(),
            state: Some("unsupported".to_owned()),
            reason_code: "prompt_requires_project_scope".to_owned(),
            reason: "canonical prompt entry is only projected at project scope".to_owned(),
        });
        return Ok(());
    }
    let consumers = platforms
        .iter()
        .copied()
        .filter(|platform| *platform != PlatformId::AgentManager)
        .collect();
    let action = plan_direct_link(
        DirectLinkIntent {
            id: projection_id(request, asset, ProjectionSurface::ProjectEntry),
            source: asset.source_ref(),
            consumers,
            target: ProjectionTarget {
                path: request.deploy_base.join("AGENTS.md"),
                entry_key: None,
            },
            mode: ProjectionMode::DirectLink,
            adapter_allows_copy_fallback: false,
        },
        request.operation,
        warnings,
        context,
    )?;
    direct.push(action);
    Ok(())
}

fn plan_direct_link(
    mut intent: DirectLinkIntent,
    operation: ProjectionOperation,
    warnings: &mut Vec<PlanWarning>,
    context: &PlannerContext<'_>,
) -> Result<ProjectionAction, CoreError> {
    intent.target.path = normalized_target_path(&intent.target.path);
    let precondition = path_fingerprint(&intent.target.path)?;
    let source_path = canonical_source_path(&intent.source.absolute_path)?;
    let observation = observe_target(&intent.target.path, &precondition);
    let expectation = ProjectionExpectation {
        id: intent.id.clone(),
        mode: intent.mode,
        canonical_source: source_path,
        source_content_digest: intent.source.fingerprint.clone(),
    };
    let record = ledger_record(context, &intent.id, warnings);
    let copy_ownership = copied_ownership(&intent, &precondition, record.as_ref());
    let state = match copy_ownership {
        CopiedOwnership::Current | CopiedOwnership::SourceChanged => ProjectionState::Copied,
        CopiedOwnership::Drifted => ProjectionState::Drifted,
        CopiedOwnership::NotCopied => {
            classify_projection(&expectation, &observation, record.as_ref())
        }
    };
    let copy_fallback_allowed = intent.adapter_allows_copy_fallback
        && context.copy_fallback_is_authorized_for(&intent.consumers);
    let (kind, reason_code, reason) = match operation {
        ProjectionOperation::Sync => match state {
            ProjectionState::Missing if copy_fallback_allowed => (
                ProjectionActionKind::CopyFallback,
                "copy_fallback_target_missing",
                "an explicitly authorized Windows copy fallback will materialize the missing target",
            ),
            ProjectionState::Missing => (
                ProjectionActionKind::CreateLink,
                "direct_target_missing",
                "target is missing",
            ),
            ProjectionState::ManagedLink => (
                ProjectionActionKind::Noop,
                "managed_link_noop",
                "exact canonical link already exists",
            ),
            ProjectionState::Equivalent => (
                ProjectionActionKind::AdoptEquivalent,
                "equivalent_requires_explicit_adoption",
                "equivalent unmanaged content requires explicit adoption",
            ),
            ProjectionState::Copied if copy_ownership == CopiedOwnership::Current => (
                ProjectionActionKind::Noop,
                "managed_copy_noop",
                "ledger-proven copied target still matches its recorded digest",
            ),
            ProjectionState::Copied if copy_fallback_allowed => (
                ProjectionActionKind::CopyFallback,
                "managed_copy_source_changed",
                "an explicitly authorized Windows copy fallback will refresh the ledger-proven copy",
            ),
            ProjectionState::Copied => (
                ProjectionActionKind::ReportOnly,
                "copy_refresh_requires_explicit_authorization",
                "a copied target may be refreshed only by a newly explicit Windows fallback plan",
            ),
            ProjectionState::Foreign | ProjectionState::Conflict | ProjectionState::Drifted => (
                ProjectionActionKind::ReportOnly,
                "direct_target_conflict",
                "target is not proven to be an owned direct link or unchanged copied projection",
            ),
            ProjectionState::ManagedGenerated
            | ProjectionState::Unsupported => (
                ProjectionActionKind::ReportOnly,
                "direct_target_incompatible_state",
                "direct-link planner received an incompatible ownership state",
            ),
        },
        ProjectionOperation::Retract | ProjectionOperation::Uninstall => match state {
            ProjectionState::ManagedLink => (
                ProjectionActionKind::RemoveManagedLink,
                "managed_link_retract",
                "exact canonical link is safe to retract",
            ),
            ProjectionState::Copied if copy_ownership == CopiedOwnership::Current => (
                ProjectionActionKind::RemoveManagedCopy,
                "managed_copy_retract",
                "ledger-proven copied target still matches its recorded digest",
            ),
            ProjectionState::Missing => (
                ProjectionActionKind::Noop,
                "target_already_absent",
                "target is already absent",
            ),
            _ => (
                ProjectionActionKind::ReportOnly,
                "retract_requires_managed_link",
                "retract requires an exact managed link",
            ),
        },
        ProjectionOperation::CleanupOrphans
        | ProjectionOperation::Import
        | ProjectionOperation::Migrate => {
            unreachable!("cleanup/import/migrate return before direct-link planning")
        }
    };
    Ok(ProjectionAction {
        kind,
        target: Some(intent.target),
        precondition: Some(precondition),
        ownership_fingerprint: (matches!(kind, ProjectionActionKind::RemoveManagedCopy)
            || (kind == ProjectionActionKind::CopyFallback
                && copy_ownership == CopiedOwnership::SourceChanged))
            .then(|| {
                record
                    .as_ref()
                    .map(|record| record.target_fingerprint.clone())
            })
            .flatten(),
        generated_renderer: None,
        members: vec![ProjectionMember {
            id: intent.id,
            source: intent.source,
            entry_key: None,
        }],
        mcp_members: Vec::new(),
        consumers: intent.consumers,
        state: Some(
            if kind == ProjectionActionKind::CopyFallback {
                "copied"
            } else {
                state_name(state)
            }
            .to_owned(),
        ),
        reason_code: reason_code.to_owned(),
        reason: reason.to_owned(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopiedOwnership {
    NotCopied,
    Current,
    SourceChanged,
    Drifted,
}

/// A copied projection has no link target to prove ownership. Its proof is therefore stricter:
/// a matching ledger ID/mode/source path plus the unchanged recorded target digest. A missing
/// ledger entry or any target drift is intentionally never removable.
fn copied_ownership(
    intent: &DirectLinkIntent,
    precondition: &PathFingerprint,
    record: Option<&ProjectionRecord>,
) -> CopiedOwnership {
    let Some(record) = record else {
        return CopiedOwnership::NotCopied;
    };
    if record.id != intent.id || record.mode != ProjectionMode::CopyFallback {
        return CopiedOwnership::NotCopied;
    }
    if precondition.entry_type == super::model::FingerprintType::Missing {
        return CopiedOwnership::NotCopied;
    }
    if !matches!(
        precondition.entry_type,
        super::model::FingerprintType::File | super::model::FingerprintType::Directory
    ) || precondition.digest.as_deref() != Some(record.target_fingerprint.as_str())
        || record.source_path != intent.source.absolute_path
    {
        return CopiedOwnership::Drifted;
    }
    if record.source_fingerprint == intent.source.fingerprint {
        CopiedOwnership::Current
    } else {
        CopiedOwnership::SourceChanged
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GeneratedOwnership {
    Managed,
    SourceChanged,
    Equivalent,
    Missing,
    Drifted,
    Foreign,
}

fn generated_batch_ownership(
    batch: &GeneratedBatch,
    precondition: &PathFingerprint,
    warnings: &mut Vec<PlanWarning>,
    context: &PlannerContext<'_>,
) -> GeneratedOwnership {
    let Some(digest) = precondition.digest.as_deref() else {
        return GeneratedOwnership::Foreign;
    };
    let mut source_changed = false;
    for member in &batch.members {
        let Some(record) = ledger_record(context, &member.id, warnings) else {
            return GeneratedOwnership::Foreign;
        };
        if record.mode != batch.mode
            || record.target_path != batch.target.path
            || record.entry_key != member.entry_key
        {
            return GeneratedOwnership::Foreign;
        }
        if record.target_fingerprint != digest {
            return GeneratedOwnership::Drifted;
        }
        if record.source_path != member.source.absolute_path
            || record.source_fingerprint != member.source.fingerprint
        {
            source_changed = true;
        }
    }
    if source_changed {
        GeneratedOwnership::SourceChanged
    } else {
        GeneratedOwnership::Managed
    }
}

/// Hook ownership is per managed binding, while container drift remains deliberately strict.
/// A foreign hooks.json may therefore receive a missing named binding, but retract still needs
/// both the managed binding and an unchanged container digest recorded in the ledger.
fn hook_generated_batch_ownership(
    batch: &GeneratedBatch,
    precondition: &PathFingerprint,
    warnings: &mut Vec<PlanWarning>,
    context: &PlannerContext<'_>,
) -> GeneratedOwnership {
    if precondition.entry_type == super::model::FingerprintType::Missing {
        return GeneratedOwnership::Foreign;
    }
    if precondition.entry_type != super::model::FingerprintType::File
        || batch.renderer != GeneratedContainerRenderer::HookJson
    {
        return GeneratedOwnership::Foreign;
    }
    let Some(digest) = precondition.digest.as_deref() else {
        return GeneratedOwnership::Foreign;
    };
    let document = match fs::read_to_string(batch.target.path.as_std_path())
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
    {
        Some(document) => document,
        None => {
            warnings.push(PlanWarning {
                code: "hook_target_uninspectable".to_owned(),
                message: "Hook target cannot be read; ownership is not proven".to_owned(),
            });
            return GeneratedOwnership::Drifted;
        }
    };
    let mut source_changed = false;
    let mut missing = false;
    for member in &batch.members {
        if !hook_document_contains_managed_binding(&document, &member.id.name) {
            if let Some(record) = ledger_record(context, &member.id, warnings) {
                if record.target_fingerprint != digest {
                    return GeneratedOwnership::Drifted;
                }
            }
            missing = true;
            continue;
        }
        let Some(record) = ledger_record(context, &member.id, warnings) else {
            return GeneratedOwnership::Foreign;
        };
        if record.mode != batch.mode
            || record.target_path != batch.target.path
            || record.entry_key != member.entry_key
            || record.target_fingerprint != digest
        {
            return GeneratedOwnership::Drifted;
        }
        if record.source_path != member.source.absolute_path
            || record.source_fingerprint != member.source.fingerprint
        {
            source_changed = true;
        }
    }
    if missing {
        GeneratedOwnership::Missing
    } else if source_changed {
        GeneratedOwnership::SourceChanged
    } else {
        GeneratedOwnership::Managed
    }
}

fn hook_document_contains_managed_binding(document: &serde_json::Value, name: &str) -> bool {
    document
        .get("hooks")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flat_map(|hooks| hooks.values())
        .filter_map(serde_json::Value::as_array)
        .flatten()
        .any(|entry| {
            entry.get("hook").and_then(serde_json::Value::as_str) == Some(name)
                && entry.get("managedBy").and_then(serde_json::Value::as_str) == Some("agents-manager")
                || entry
                    .get("hooks")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|nested| {
                        nested.iter().any(|nested| {
                            nested.get("hook").and_then(serde_json::Value::as_str) == Some(name)
                                && nested.get("managedBy").and_then(serde_json::Value::as_str)
                                    == Some("agents-manager")
                        })
                    })
        })
}

/// MCP generated ownership is per named server entry, not per container. In particular, the
/// full-file digest is intentionally ignored here: users may edit unrelated MCP servers or
/// platform settings without invalidating an otherwise unchanged managed server.
fn mcp_generated_batch_ownership(
    batch: &GeneratedBatch,
    precondition: &PathFingerprint,
    warnings: &mut Vec<PlanWarning>,
    context: &PlannerContext<'_>,
) -> GeneratedOwnership {
    if precondition.entry_type == super::model::FingerprintType::Missing {
        return GeneratedOwnership::Foreign;
    }
    let Some(entries) = inspect_mcp_target_entries(batch, warnings) else {
        return GeneratedOwnership::Drifted;
    };
    let mut source_changed = false;
    let mut entry_missing = false;
    let mut unowned_equivalent = false;
    for member in &batch.mcp_members {
        let Some(entry) = entries.iter().find(|entry| entry.name == member.name) else {
            entry_missing = true;
            continue;
        };
        let Some(record) = ledger_record(context, &member.id, warnings) else {
            let expected = load_mcp_definition_at(&member.source.absolute_path)
                .and_then(|definition| fingerprint_mcp_config(definition.server.config));
            match expected {
                Ok(expected) if expected == entry.digest => {
                    unowned_equivalent = true;
                    continue;
                }
                _ => return GeneratedOwnership::Foreign,
            }
        };
        if record.mode != batch.mode
            || record.target_path != batch.target.path
            || record.entry_key.as_deref() != Some(member.entry_key.as_str())
            || record.source_path != member.source.absolute_path
        {
            return GeneratedOwnership::Foreign;
        }
        let Some(recorded_entry_fingerprint) = record.entry_fingerprint.as_deref() else {
            return GeneratedOwnership::Foreign;
        };
        if entry.digest != recorded_entry_fingerprint {
            return GeneratedOwnership::Drifted;
        }
        if record.source_fingerprint != member.source.fingerprint {
            source_changed = true;
        }
    }
    if source_changed {
        GeneratedOwnership::SourceChanged
    } else if entry_missing {
        GeneratedOwnership::Missing
    } else if unowned_equivalent {
        GeneratedOwnership::Equivalent
    } else {
        GeneratedOwnership::Managed
    }
}

fn inspect_mcp_target_entries(
    batch: &GeneratedBatch,
    warnings: &mut Vec<PlanWarning>,
) -> Option<Vec<McpEntryFingerprint>> {
    let existing = match fs::read_to_string(batch.target.path.as_std_path()) {
        Ok(existing) => existing,
        Err(_) => {
            warnings.push(PlanWarning {
                code: "mcp_target_uninspectable".to_owned(),
                message: "MCP target cannot be read; entry ownership is not proven".to_owned(),
            });
            return None;
        }
    };
    let inspected = match batch.renderer {
        GeneratedContainerRenderer::McpJson => inspect_cursor_mcp_entries(&existing),
        GeneratedContainerRenderer::McpToml => inspect_codex_mcp_entries(&existing),
        GeneratedContainerRenderer::McpYaml => inspect_hermes_mcp_entries(&existing),
        _ => unreachable!("MCP intent batches only use MCP renderers"),
    };
    match inspected {
        Ok(entries) => Some(entries),
        Err(_) => {
            warnings.push(PlanWarning {
                code: "mcp_target_uninspectable".to_owned(),
                message: "MCP target syntax is invalid; entry ownership is not proven".to_owned(),
            });
            None
        }
    }
}

fn ledger_record(
    context: &PlannerContext<'_>,
    id: &ProjectionId,
    warnings: &mut Vec<PlanWarning>,
) -> Option<super::model::ProjectionRecord> {
    match context.ledger.get(id) {
        Ok(record) => record,
        Err(_) => {
            if !warnings
                .iter()
                .any(|warning| warning.code == "ledger_unavailable")
            {
                warnings.push(PlanWarning {
                    code: "ledger_unavailable".to_owned(),
                    message: "projection ledger is unreadable; generated ownership is not proven"
                        .to_owned(),
                });
            }
            None
        }
    }
}

fn append_orphan_candidates(
    request: &ProjectionRequest,
    context: &PlannerContext<'_>,
    warnings: &mut Vec<PlanWarning>,
    actions: &mut Vec<ProjectionAction>,
) -> Result<(), CoreError> {
    let planned_ids = actions
        .iter()
        .flat_map(|action| {
            action
                .members
                .iter()
                .map(|member| member.id.clone())
                .chain(action.mcp_members.iter().map(|member| member.id.clone()))
        })
        .collect::<HashSet<_>>();
    let records = match context.ledger.list_scope(&request.scope_key) {
        Ok(records) => records,
        Err(_) => {
            if !warnings
                .iter()
                .any(|warning| warning.code == "ledger_unavailable")
            {
                warnings.push(PlanWarning {
                    code: "ledger_unavailable".to_owned(),
                    message: "projection ledger is unreadable; orphan ownership is not proven"
                        .to_owned(),
                });
            }
            return Ok(());
        }
    };
    for record in records {
        if planned_ids.contains(&record.id) {
            continue;
        }
        actions.push(ProjectionAction {
            kind: ProjectionActionKind::ReportOnly,
            target: Some(ProjectionTarget {
                path: record.target_path.clone(),
                entry_key: record.entry_key.clone(),
            }),
            precondition: Some(path_fingerprint(&record.target_path)?),
            ownership_fingerprint: None,
            generated_renderer: None,
            members: Vec::new(),
            mcp_members: Vec::new(),
            consumers: Vec::new(),
            state: Some("orphan_candidate".to_owned()),
            reason_code: "ledger_orphan_candidate".to_owned(),
            reason: "ledger record has no effective source asset in this scope".to_owned(),
        });
    }
    Ok(())
}

/// Cleanup is a separate explicit request. It never infers ownership from an absent or
/// modified target: the current fingerprint must still equal the ledger record.
fn build_orphan_cleanup_plan(
    request: &ProjectionRequest,
    context: &PlannerContext<'_>,
) -> Result<ProjectionPlan, CoreError> {
    let mut sync_request = request.clone();
    sync_request.operation = ProjectionOperation::Sync;
    let sync_plan = build_projection_plan(&sync_request, context)?;
    let planned_ids = sync_plan
        .actions
        .iter()
        .filter(|action| action.reason_code != "ledger_orphan_candidate")
        .flat_map(|action| action.members.iter().map(|member| member.id.clone()))
        .collect::<HashSet<_>>();
    let records = match context.ledger.list_scope(&request.scope_key) {
        Ok(records) => records,
        Err(_) => {
            return finish_plan(
                request,
                Vec::new(),
                vec![PlanWarning {
                    code: "ledger_unavailable".to_owned(),
                    message: "projection ledger is unreadable; orphan cleanup is not proven"
                        .to_owned(),
                }],
            )
        }
    };
    let mut actions = Vec::new();
    for record in records {
        if planned_ids.contains(&record.id) {
            continue;
        }
        let target = ProjectionTarget {
            path: record.target_path.clone(),
            entry_key: record.entry_key.clone(),
        };
        match path_fingerprint(&record.target_path) {
            Ok(precondition) if orphan_cleanup_is_proven(&record, &precondition) => {
                actions.push(ProjectionAction {
                    kind: ProjectionActionKind::CleanupOrphan,
                    target: Some(target),
                    precondition: Some(precondition),
                    ownership_fingerprint: Some(orphan_ownership_fingerprint(&record)),
                    generated_renderer: None,
                    members: Vec::new(),
                    mcp_members: Vec::new(),
                    consumers: Vec::new(),
                    state: Some("managed_orphan".to_owned()),
                    reason_code: "cleanup_orphan_requires_confirmation".to_owned(),
                    reason: "unchanged ledger-owned orphan requires explicit cleanup confirmation"
                        .to_owned(),
                });
            }
            Ok(precondition) => actions.push(ProjectionAction {
                kind: ProjectionActionKind::ReportOnly,
                target: Some(target),
                precondition: Some(precondition),
                ownership_fingerprint: None,
                generated_renderer: None,
                members: Vec::new(),
                mcp_members: Vec::new(),
                consumers: Vec::new(),
                state: Some("drifted".to_owned()),
                reason_code: "orphan_cleanup_ownership_drifted".to_owned(),
                reason: "orphan target no longer matches its ledger ownership fingerprint"
                    .to_owned(),
            }),
            Err(error) => actions.push(ProjectionAction {
                kind: ProjectionActionKind::ReportOnly,
                target: Some(target),
                precondition: None,
                ownership_fingerprint: None,
                generated_renderer: None,
                members: Vec::new(),
                mcp_members: Vec::new(),
                consumers: Vec::new(),
                state: Some("conflict".to_owned()),
                reason_code: "orphan_cleanup_target_unreadable".to_owned(),
                reason: format!("orphan target cannot be fingerprinted: {error}"),
            }),
        }
    }
    finish_plan(request, actions, Vec::new())
}

/// A cleanup candidate needs stronger proof than an unchanged path digest alone.
/// Direct links must still point at the exact ledger source. Generated entries retain their
/// named entry key and unchanged container digest; copy fallback is never cleanup-eligible.
fn orphan_cleanup_is_proven(record: &ProjectionRecord, precondition: &PathFingerprint) -> bool {
    if precondition.digest.as_deref() != Some(record.target_fingerprint.as_str()) {
        return false;
    }
    match record.mode {
        ProjectionMode::DirectLink => {
            precondition.entry_type == super::model::FingerprintType::Symlink
                && precondition.link_target.as_ref() == Some(&record.source_path)
        }
        ProjectionMode::GeneratedJson
        | ProjectionMode::GeneratedToml
        | ProjectionMode::GeneratedYaml
        | ProjectionMode::ExternalDirectory => record.entry_key.is_some(),
        ProjectionMode::GeneratedMarkdown | ProjectionMode::CopyFallback => false,
    }
}

#[derive(Serialize)]
struct OrphanOwnership<'a> {
    id: &'a ProjectionId,
    mode: ProjectionMode,
    source_path: &'a Utf8Path,
    target_path: &'a Utf8Path,
    entry_key: &'a Option<String>,
    target_fingerprint: &'a str,
}

fn orphan_ownership_fingerprint(record: &ProjectionRecord) -> String {
    let ownership = OrphanOwnership {
        id: &record.id,
        mode: record.mode,
        source_path: &record.source_path,
        target_path: &record.target_path,
        entry_key: &record.entry_key,
        target_fingerprint: &record.target_fingerprint,
    };
    let bytes = serde_json::to_vec(&ownership).expect("orphan ownership metadata serializes");
    hex::encode(Sha256::digest(bytes))
}

fn observe_target(path: &Utf8Path, fingerprint: &PathFingerprint) -> ProjectionObservation {
    use super::model::FingerprintType;

    match fingerprint.entry_type {
        FingerprintType::Missing => ProjectionObservation::Missing,
        FingerprintType::Symlink => fs::canonicalize(path.as_std_path())
            .ok()
            .and_then(|target| Utf8PathBuf::from_path_buf(target).ok())
            .map(|canonical_target| ProjectionObservation::Link { canonical_target })
            .unwrap_or(ProjectionObservation::Unreadable),
        FingerprintType::File | FingerprintType::Directory => fingerprint
            .digest
            .clone()
            .map(|content_digest| ProjectionObservation::Regular { content_digest })
            .unwrap_or(ProjectionObservation::Unreadable),
        FingerprintType::Other => ProjectionObservation::Unreadable,
    }
}

fn canonical_source_path(path: &Utf8Path) -> Result<Utf8PathBuf, CoreError> {
    let canonical = fs::canonicalize(path.as_std_path())?;
    Utf8PathBuf::from_path_buf(canonical)
        .map_err(|non_utf8| CoreError::InvalidPath(non_utf8.to_string_lossy().into_owned()))
}

/// Normalize only separator and current-directory spelling for deterministic target grouping.
/// Parent components are deliberately retained because collapsing them can cross a symlink.
fn normalized_target_path(path: &Utf8Path) -> Utf8PathBuf {
    let mut normalized = Utf8PathBuf::new();
    for component in path.components() {
        normalized.push(component.as_str());
    }
    normalized
}

fn projection_id(
    request: &ProjectionRequest,
    asset: &EffectiveAsset,
    surface: ProjectionSurface,
) -> ProjectionId {
    ProjectionId {
        scope_key: request.scope_key.clone(),
        kind: asset.kind,
        name: asset.name.clone(),
        surface,
    }
}

fn merge_consumers(existing: &mut Vec<PlatformId>, incoming: &[PlatformId]) {
    existing.extend(incoming.iter().copied());
    existing.sort_by_key(platform_key);
    existing.dedup();
}

fn asset_key(asset: &EffectiveAsset) -> (u8, &str) {
    (asset_kind_key(asset.kind), asset.name.as_str())
}

fn action_key(action: &ProjectionAction) -> (String, u8, String) {
    let target = action
        .target
        .as_ref()
        .map(|target| target.path.as_str().to_owned())
        .unwrap_or_default();
    let member = action
        .members
        .first()
        .map(|member| projection_id_key(&member.id))
        .unwrap_or_default();
    (target, action_kind_key(action.kind), member)
}

fn trust_requirement_for_action(
    request: &ProjectionRequest,
    action: &ProjectionAction,
) -> TrustRequirement {
    for id in action
        .members
        .iter()
        .map(|member| &member.id)
        .chain(action.mcp_members.iter().map(|member| &member.id))
    {
        let platform = match &id.surface {
            ProjectionSurface::Platform(platform)
            | ProjectionSurface::PlatformBinding { platform, .. } => Some(*platform),
            ProjectionSurface::ProjectEntry | ProjectionSurface::SharedTarget { .. } => None,
        };
        let Some(platform) = platform else {
            continue;
        };
        if request.scope != DeploymentScope::User
            && platform == PlatformId::Codex
            && id.kind == AssetKind::Hook
        {
            return TrustRequirement::TrustedProjectWithIndependentReview;
        }
        if request.scope != DeploymentScope::User
            && platform == PlatformId::Codex
            && id.kind == AssetKind::Mcp
        {
            return TrustRequirement::TrustedProject;
        }
    }
    TrustRequirement::None
}

fn projection_id_key(id: &ProjectionId) -> String {
    format!(
        "{}:{}:{}:{:?}",
        id.scope_key,
        asset_kind_key(id.kind),
        id.name,
        id.surface
    )
}

fn asset_kind_key(kind: AssetKind) -> u8 {
    match kind {
        AssetKind::Skill => 0,
        AssetKind::Rule => 1,
        AssetKind::Mcp => 2,
        AssetKind::Agent => 3,
        AssetKind::Command => 4,
        AssetKind::Prompt => 5,
        AssetKind::Hook => 6,
    }
}

fn platform_key(platform: &PlatformId) -> u8 {
    match platform {
        PlatformId::AgentManager => 0,
        PlatformId::Cursor => 1,
        PlatformId::Codex => 2,
        PlatformId::Claude => 3,
        PlatformId::Hermes => 4,
    }
}

fn action_kind_key(kind: ProjectionActionKind) -> u8 {
    match kind {
        ProjectionActionKind::CreateLink => 0,
        ProjectionActionKind::RemoveManagedLink => 1,
        ProjectionActionKind::CopyFallback => 2,
        ProjectionActionKind::RemoveManagedCopy => 3,
        ProjectionActionKind::RemoveGeneratedEntries => 4,
        ProjectionActionKind::AdoptEquivalent => 5,
        ProjectionActionKind::UpsertGeneratedBatch => 6,
        ProjectionActionKind::Noop => 7,
        ProjectionActionKind::CleanupOrphan => 8,
        ProjectionActionKind::ReportOnly => 9,
    }
}

fn state_name(state: ProjectionState) -> &'static str {
    match state {
        ProjectionState::Missing => "missing",
        ProjectionState::ManagedLink => "managed_link",
        ProjectionState::ManagedGenerated => "managed_generated",
        ProjectionState::Copied => "copied",
        ProjectionState::Equivalent => "equivalent",
        ProjectionState::Foreign => "foreign",
        ProjectionState::Conflict => "conflict",
        ProjectionState::Drifted => "drifted",
        ProjectionState::Unsupported => "unsupported",
    }
}

#[derive(Serialize)]
struct PlanDigestInput<'a> {
    schema_version: u16,
    actions: Vec<PlanDigestAction<'a>>,
}

#[derive(Serialize)]
struct PlanDigestAction<'a> {
    kind: ProjectionActionKind,
    target: &'a Option<ProjectionTarget>,
    precondition: &'a Option<PathFingerprint>,
    ownership_fingerprint: &'a Option<String>,
    generated_renderer: &'a Option<GeneratedContainerRenderer>,
    members: &'a [ProjectionMember],
    mcp_members: &'a [McpProjectionMember],
    consumers: &'a [PlatformId],
    state: &'a Option<String>,
    reason_code: &'a str,
}

impl<'a> PlanDigestAction<'a> {
    fn from_action(action: &'a ProjectionAction) -> Self {
        Self {
            kind: action.kind,
            target: &action.target,
            precondition: &action.precondition,
            ownership_fingerprint: &action.ownership_fingerprint,
            generated_renderer: &action.generated_renderer,
            members: &action.members,
            mcp_members: &action.mcp_members,
            consumers: &action.consumers,
            state: &action.state,
            reason_code: &action.reason_code,
        }
    }
}

fn stable_action_id(action: &PlanDigestAction<'_>) -> String {
    let bytes = serde_json::to_vec(action).expect("projection action metadata serializes");
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use camino::{Utf8Path, Utf8PathBuf};

    use crate::model::{AssetKind, PlatformId};
    use crate::projection::model::{
        ProjectionId, ProjectionMode, ProjectionSurface, ProjectionTarget, SourceLayer, SourceRef,
    };

    use super::{
        normalized_target_path, register_generated_member, GeneratedBatch,
        GeneratedContainerRenderer, ProjectionMember,
    };

    #[test]
    fn target_normalization_collapses_separator_and_current_directory_only() {
        assert_eq!(
            normalized_target_path(Utf8Path::new("/tmp//deploy/./.cursor/mcp.json")),
            Utf8Path::new("/tmp/deploy/.cursor/mcp.json")
        );
        assert_eq!(
            normalized_target_path(Utf8Path::new("/tmp/deploy/../linked/.cursor/mcp.json")),
            Utf8Path::new("/tmp/deploy/../linked/.cursor/mcp.json"),
            "parent components may cross symlinks and must not be collapsed lexically"
        );
    }

    #[test]
    fn normalized_generated_targets_share_one_batch_and_keep_entry_keys() {
        let mut batches: BTreeMap<String, GeneratedBatch> = BTreeMap::new();
        let member = |name: &str| ProjectionMember {
            id: ProjectionId {
                scope_key: "project:/fixture".to_owned(),
                kind: AssetKind::Mcp,
                name: name.to_owned(),
                surface: ProjectionSurface::Platform(PlatformId::Cursor),
            },
            source: SourceRef {
                layer: SourceLayer::Project,
                absolute_path: Utf8PathBuf::from(format!("/source/{name}.json")),
                fingerprint: name.to_owned(),
            },
            entry_key: None,
        };

        register_generated_member(
            &mut batches,
            ProjectionTarget {
                path: Utf8PathBuf::from("/tmp//deploy/./.cursor/mcp.json"),
                entry_key: Some("mcpServers.catalog".to_owned()),
            },
            ProjectionMode::GeneratedJson,
            GeneratedContainerRenderer::McpJson,
            vec![PlatformId::Cursor],
            member("catalog"),
        );
        register_generated_member(
            &mut batches,
            ProjectionTarget {
                path: Utf8PathBuf::from("/tmp/deploy/.cursor/mcp.json"),
                entry_key: Some("mcpServers.search".to_owned()),
            },
            ProjectionMode::GeneratedJson,
            GeneratedContainerRenderer::McpJson,
            vec![PlatformId::Cursor],
            member("search"),
        );

        assert_eq!(batches.len(), 1);
        let batch = batches.values().next().unwrap();
        assert_eq!(
            batch.target.path,
            Utf8PathBuf::from("/tmp/deploy/.cursor/mcp.json")
        );
        assert_eq!(batch.target.entry_key, None);
        assert_eq!(
            batch
                .members
                .iter()
                .map(|member| member.entry_key.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("mcpServers.catalog"), Some("mcpServers.search")]
        );
    }

    #[test]
    fn normalized_target_with_disagreeing_renderer_is_marked_blocking_conflict() {
        let mut batches: BTreeMap<String, GeneratedBatch> = BTreeMap::new();
        let member = |name: &str| ProjectionMember {
            id: ProjectionId {
                scope_key: "project:/fixture".to_owned(),
                kind: AssetKind::Hook,
                name: name.to_owned(),
                surface: ProjectionSurface::Platform(PlatformId::Cursor),
            },
            source: SourceRef {
                layer: SourceLayer::Project,
                absolute_path: Utf8PathBuf::from(format!("/source/{name}.json")),
                fingerprint: name.to_owned(),
            },
            entry_key: None,
        };

        register_generated_member(
            &mut batches,
            ProjectionTarget {
                path: Utf8PathBuf::from("/tmp/deploy/.cursor/container"),
                entry_key: Some("one".to_owned()),
            },
            ProjectionMode::GeneratedJson,
            GeneratedContainerRenderer::HookJson,
            vec![PlatformId::Cursor],
            member("one"),
        );
        register_generated_member(
            &mut batches,
            ProjectionTarget {
                path: Utf8PathBuf::from("/tmp//deploy/./.cursor/container"),
                entry_key: Some("two".to_owned()),
            },
            ProjectionMode::GeneratedJson,
            GeneratedContainerRenderer::GenericJson,
            vec![PlatformId::Cursor],
            member("two"),
        );

        assert_eq!(batches.len(), 1);
        assert!(batches.values().next().unwrap().renderer_conflict);
    }
}
