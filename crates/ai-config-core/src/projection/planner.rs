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
}

impl<'a> PlannerContext<'a> {
    pub fn new(ledger: &'a dyn ProjectionLedger) -> Self {
        Self { ledger }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionActionKind {
    CreateLink,
    RemoveManagedLink,
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
        for platform in &platforms {
            let contract = capability_contract_for(*platform, asset, &target_context);
            match contract.capability {
                PlatformCapability::DirectLink {
                    target,
                    mode,
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
                consumers: batch.consumers,
                state: Some("conflict".to_owned()),
                reason_code: "generated_renderer_conflict".to_owned(),
                reason: "multiple generated renderers claim one normalized target".to_owned(),
            }
        } else {
            let ownership =
                generated_batch_ownership(&batch, &precondition, &mut warnings, context);
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
                    consumers: batch.consumers,
                    state: Some("drifted".to_owned()),
                    reason_code: "generated_target_drifted".to_owned(),
                    reason: "generated container changed after its recorded projection".to_owned(),
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
                        consumers: batch.consumers,
                        state: Some("managed_generated".to_owned()),
                        reason_code: "generated_source_changed".to_owned(),
                        reason: "canonical source changed since the recorded projection".to_owned(),
                    }
                }
                (ProjectionOperation::Sync, _, GeneratedOwnership::Drifted) => ProjectionAction {
                    kind: ProjectionActionKind::ReportOnly,
                    target: Some(batch.target),
                    precondition: Some(precondition),
                    ownership_fingerprint: None,
                    generated_renderer: Some(batch.renderer),
                    members: batch.members,
                    consumers: batch.consumers,
                    state: Some("drifted".to_owned()),
                    reason_code: "generated_target_drifted".to_owned(),
                    reason: "generated container changed after its recorded projection".to_owned(),
                },
                (ProjectionOperation::Sync, _, GeneratedOwnership::Foreign) => ProjectionAction {
                    kind: ProjectionActionKind::ReportOnly,
                    target: Some(batch.target),
                    precondition: Some(precondition),
                    ownership_fingerprint: None,
                    generated_renderer: Some(batch.renderer),
                    members: batch.members,
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
    renderer_conflict: bool,
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
                let (script_target, script_mode) = match script {
                    PlatformCapability::DirectLink { target, mode, .. } => (target, mode),
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
        .filter(|platform| *platform != PlatformId::AiConfig)
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
    let state = classify_projection(&expectation, &observation, record.as_ref());
    let (kind, reason_code, reason) = match operation {
        ProjectionOperation::Sync => match state {
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
            ProjectionState::Foreign | ProjectionState::Conflict | ProjectionState::Drifted => (
                ProjectionActionKind::ReportOnly,
                "direct_target_conflict",
                "target is not proven to be an owned direct link",
            ),
            ProjectionState::ManagedGenerated
            | ProjectionState::Copied
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
        ownership_fingerprint: None,
        generated_renderer: None,
        members: vec![ProjectionMember {
            id: intent.id,
            source: intent.source,
            entry_key: None,
        }],
        consumers: intent.consumers,
        state: Some(state_name(state).to_owned()),
        reason_code: reason_code.to_owned(),
        reason: reason.to_owned(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GeneratedOwnership {
    Managed,
    SourceChanged,
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
        .flat_map(|action| action.members.iter().map(|member| member.id.clone()))
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
    for member in &action.members {
        let platform = match &member.id.surface {
            ProjectionSurface::Platform(platform)
            | ProjectionSurface::PlatformBinding { platform, .. } => Some(*platform),
            ProjectionSurface::ProjectEntry | ProjectionSurface::SharedTarget { .. } => None,
        };
        let Some(platform) = platform else {
            continue;
        };
        if request.scope != DeploymentScope::User
            && platform == PlatformId::Codex
            && member.id.kind == AssetKind::Hook
        {
            return TrustRequirement::TrustedProjectWithIndependentReview;
        }
        if request.scope != DeploymentScope::User
            && platform == PlatformId::Codex
            && member.id.kind == AssetKind::Mcp
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
        PlatformId::AiConfig => 0,
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
        ProjectionActionKind::RemoveGeneratedEntries => 2,
        ProjectionActionKind::AdoptEquivalent => 3,
        ProjectionActionKind::UpsertGeneratedBatch => 4,
        ProjectionActionKind::Noop => 5,
        ProjectionActionKind::CleanupOrphan => 6,
        ProjectionActionKind::ReportOnly => 7,
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
}
