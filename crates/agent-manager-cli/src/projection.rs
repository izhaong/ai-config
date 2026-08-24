//! Public install / sync / uninstall projection lifecycle.
//!
//! Planning is always read-only. The first persistent ownership ledger is created only after
//! the caller supplies --apply, so a default invocation cannot initialize a platform target.

use std::collections::{BTreeMap, BTreeSet};
use std::process::ExitCode;

use agent_manager_core::error::{exit_code, CoreError};
use agent_manager_core::model::{AssetKind, PlatformId};
use agent_manager_core::paths::{self, SyncRoots};
use agent_manager_core::projection::executor::{
    apply_projection_plans_transactionally, rollback_projection_transaction, ApplyOptions,
    ApplyReport, ExecutorContext, McpSecretProvider, ProjectionRollbackReport,
    ProjectionTransactionError,
};
use agent_manager_core::projection::ledger::{MemoryProjectionLedger, ProjectionLedger};
use agent_manager_core::projection::mcp::source::resolve_effective_mcp_definitions;
use agent_manager_core::projection::model::{
    DeploymentScope, LedgerMutation, ProjectionId, ProjectionRecord,
};
use agent_manager_core::projection::planner::{
    build_hermes_cross_domain_projection_plan, build_mcp_projection_plan, build_projection_plan,
    McpSecretAvailability, PlannerContext, ProjectionActionKind, ProjectionOperation,
    ProjectionPlan, ProjectionRequest, PROJECTION_PLAN_SCHEMA_VERSION,
};
use agent_manager_core::projection::source::{resolve_effective_assets, OverlayRoots};
use agent_manager_core::secrets;
use agent_manager_store::Store;
use camino::{Utf8Path, Utf8PathBuf};
use schemars::JsonSchema;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::output::{emit_error_envelope, emit_json, emit_line, OutputMode};

#[derive(Debug, Serialize, JsonSchema)]
pub struct PublicPlan {
    pub schema_version: u16,
    pub plan_digest: String,
    /// The core action model is deliberately exposed as JSON so this public report can remain
    /// forward-compatible without giving the MCP bridge a second planner-owned schema.
    pub actions: Vec<serde_json::Value>,
    /// An unreadable existing ledger never becomes an implicit empty ledger. The plan remains
    /// inspectable, but review/apply must stop until ownership evidence is available again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ledger_status: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, JsonSchema)]
pub struct ApplySummary {
    pub changed: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub conflict: usize,
    pub failed: usize,
    pub rolled_back: usize,
    pub rollback_failed: usize,
    pub not_applied: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mcp_skipped_members: Vec<McpSkippedMemberSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MigrationAdoptionReport {
    pub transaction_id: String,
    #[serde(flatten)]
    pub apply: ApplySummary,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct McpSkippedMemberSummary {
    pub entry_key: String,
    pub missing_secret_keys: Vec<String>,
}

impl ApplySummary {
    fn has_missing_mcp_secrets(&self) -> bool {
        !self.mcp_skipped_members.is_empty()
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct LifecycleReport {
    #[schemars(schema_with = "public_plan_schema")]
    pub plan: PublicPlan,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply: Option<ApplySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocking_reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct WorkspaceMemberReport {
    member: String,
    plan: PublicPlan,
    #[serde(skip_serializing_if = "Option::is_none")]
    apply: Option<ApplySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    blocking_reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct WorkspaceLifecycleReport {
    members: Vec<WorkspaceMemberReport>,
}

fn public_plan_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    PublicPlan::json_schema(generator)
}

/// A report plus the CLI's process exit code. MCP returns the report directly so an agent can
/// inspect a foreign-target guard without triggering a side-effecting retry.
pub struct LifecycleExecution {
    pub report: LifecycleReport,
    pub exit_code: u8,
}

struct PlanBundle {
    plans: Vec<ProjectionPlan>,
    public: PublicPlan,
}

struct WorkspaceBundle {
    member: Utf8PathBuf,
    bundle: PlanBundle,
}

struct WorkspaceApplyFailure {
    error: CoreError,
    has_reports: bool,
    summaries: Vec<ApplySummary>,
}

struct LifecycleMcpSecrets {
    values: BTreeMap<String, String>,
}

impl LifecycleMcpSecrets {
    fn load() -> Result<Self, CoreError> {
        Ok(Self {
            values: secrets::load_from(&secrets::default_path())?
                .into_iter()
                .collect(),
        })
    }
}

impl McpSecretAvailability for LifecycleMcpSecrets {
    fn missing_secret_keys(&self, declared_keys: &[String]) -> Result<Vec<String>, CoreError> {
        Ok(declared_keys
            .iter()
            .filter(|key| !self.values.contains_key(key.as_str()))
            .cloned()
            .collect())
    }
}

impl McpSecretProvider for LifecycleMcpSecrets {
    fn resolve(&self, key: &str) -> Result<Option<String>, CoreError> {
        Ok(self.values.get(key).cloned())
    }
}

/// The generic planner receives no parsed MCP definitions, so its orphan scan must not see MCP
/// records. The MCP-only planner below receives the full ledger and remains authoritative there.
struct NonMcpLedger<'a> {
    inner: &'a dyn ProjectionLedger,
}

struct McpOnlyLedger<'a> {
    inner: &'a dyn ProjectionLedger,
}

impl ProjectionLedger for McpOnlyLedger<'_> {
    fn get(&self, id: &ProjectionId) -> Result<Option<ProjectionRecord>, CoreError> {
        self.inner.get(id)
    }

    fn get_many(&self, ids: &[ProjectionId]) -> Result<Vec<ProjectionRecord>, CoreError> {
        self.inner.get_many(ids)
    }

    fn list_scope(&self, scope_key: &str) -> Result<Vec<ProjectionRecord>, CoreError> {
        let _ = scope_key;
        // MCP has a dedicated source-first planner. Its generic orphan pass cannot express
        // named generated entries, so cleanup stays an explicit later operation.
        Ok(Vec::new())
    }

    fn apply_batch(&self, mutations: &[LedgerMutation]) -> Result<(), CoreError> {
        self.inner.apply_batch(mutations)
    }
}

impl ProjectionLedger for NonMcpLedger<'_> {
    fn get(&self, id: &ProjectionId) -> Result<Option<ProjectionRecord>, CoreError> {
        self.inner.get(id)
    }

    fn get_many(&self, ids: &[ProjectionId]) -> Result<Vec<ProjectionRecord>, CoreError> {
        self.inner.get_many(ids)
    }

    fn list_scope(&self, scope_key: &str) -> Result<Vec<ProjectionRecord>, CoreError> {
        Ok(self
            .inner
            .list_scope(scope_key)?
            .into_iter()
            .filter(|record| record.id.kind != AssetKind::Mcp)
            .collect())
    }

    fn apply_batch(&self, mutations: &[LedgerMutation]) -> Result<(), CoreError> {
        self.inner.apply_batch(mutations)
    }
}

/// Workspace remains a hard stop until its overlay request is available. It never falls back
/// to legacy loops.
pub fn run(
    default_root: &Utf8Path,
    workspace: bool,
    apply: bool,
    uninstall: bool,
    mode: OutputMode,
) -> ExitCode {
    if workspace {
        return match execute_workspace(default_root, apply, uninstall) {
            Ok((report, exit_code)) => {
                emit_workspace_report(mode, &report);
                ExitCode::from(exit_code)
            }
            Err(error) => {
                emit_error_envelope(mode, error.exit_code(), &error.to_string(), error.hint());
                ExitCode::from(error.exit_code())
            }
        };
    }
    match execute(default_root, workspace, apply, uninstall) {
        Ok(execution) => {
            emit_report(mode, &execution.report);
            ExitCode::from(execution.exit_code)
        }
        Err(error) => {
            emit_error_envelope(mode, error.exit_code(), &error.to_string(), error.hint());
            ExitCode::from(error.exit_code())
        }
    }
}

/// Builds the same source-first lifecycle report for CLI and MCP callers. This is the only
/// public write path for install/sync/uninstall projection operations.
pub fn execute(
    default_root: &Utf8Path,
    workspace: bool,
    apply: bool,
    uninstall: bool,
) -> Result<LifecycleExecution, CoreError> {
    if workspace {
        return Err(CoreError::NotImplemented(
            "workspace projections are exposed through the CLI workspace adapter",
        ));
    }

    let roots = paths::resolve_sync_roots(default_root);
    let operation = if uninstall {
        ProjectionOperation::Uninstall
    } else {
        ProjectionOperation::Sync
    };
    let ledger_path = roots
        .deploy_base
        .join(".agent-manager/projection-ledger.sqlite");
    let secrets = LifecycleMcpSecrets::load()?;
    let bundle = build_with_existing_ledger(&roots, operation, &ledger_path, &secrets)?;
    let blocking = blocking_reason(&bundle);
    if !apply {
        return Ok(LifecycleExecution {
            report: LifecycleReport {
                plan: bundle.public,
                apply: None,
                blocking_reason: blocking,
            },
            exit_code: exit_code::SUCCESS,
        });
    }
    if blocking.as_deref() == Some("ledger_unavailable") {
        return Err(CoreError::ProjectionLedger(
            "projection ownership ledger is unavailable".to_owned(),
        ));
    }
    if let Some(reason) = blocking {
        return Ok(LifecycleExecution {
            report: LifecycleReport {
                plan: bundle.public,
                apply: None,
                blocking_reason: Some(reason),
            },
            exit_code: exit_code::PARTIAL_FAILURE,
        });
    }

    let store = Store::open_at(ledger_path.as_std_path())
        .map_err(|error| CoreError::ProjectionLedger(error.to_string()))?;
    let ledger = store.projections();
    let (report, exit_code) = match apply_bundle(&bundle, &ledger, &roots, &secrets) {
        Ok(summary) => {
            let missing_mcp_secrets = summary.has_missing_mcp_secrets();
            (
                LifecycleReport {
                    plan: bundle.public,
                    apply: Some(summary),
                    blocking_reason: missing_mcp_secrets
                        .then_some("mcp_missing_secret_keys".to_owned()),
                },
                if missing_mcp_secrets {
                    exit_code::SECRETS_MISSING
                } else {
                    exit_code::SUCCESS
                },
            )
        }
        Err(error) => {
            if error.reports.is_empty() {
                return Err(error.error);
            }
            (
                LifecycleReport {
                    plan: bundle.public,
                    apply: Some(summarize_apply_reports(error.reports)),
                    // The core error can include a source path or parser context. Preserve only a
                    // stable failure category in the public lifecycle report so it never becomes a
                    // secret-bearing error channel.
                    blocking_reason: Some("transaction_apply_failed".to_owned()),
                },
                exit_code::PARTIAL_FAILURE,
            )
        }
    };
    Ok(LifecycleExecution { report, exit_code })
}

/// Build the read-only source-first migration plan. It intentionally uses the ordinary sync
/// planner so migration candidates keep the same preconditions and ownership classification as
/// a normal projection; only the public review representation differs.
pub(crate) fn build_migration_plan(default_root: &Utf8Path) -> Result<PublicPlan, CoreError> {
    let roots = paths::resolve_sync_roots(default_root);
    let ledger_path = roots
        .deploy_base
        .join(".agent-manager/projection-ledger.sqlite");
    let secrets = LifecycleMcpSecrets::load()?;
    Ok(
        build_with_existing_ledger(&roots, ProjectionOperation::Sync, &ledger_path, &secrets)?
            .public,
    )
}

/// Rebuild and validate a reviewed migration plan without opening a writable ledger. This is the
/// dry-run boundary used by `migrate source-first` when `--apply` is absent.
pub(crate) fn verify_migration_review(
    default_root: &Utf8Path,
    reviewed_digest: &str,
    selected: &BTreeSet<String>,
) -> Result<PublicPlan, CoreError> {
    let roots = paths::resolve_sync_roots(default_root);
    let ledger_path = roots
        .deploy_base
        .join(".agent-manager/projection-ledger.sqlite");
    let secrets = LifecycleMcpSecrets::load()?;
    let bundle =
        build_with_existing_ledger(&roots, ProjectionOperation::Sync, &ledger_path, &secrets)?;
    validate_migration_review(&bundle, reviewed_digest, selected)?;
    Ok(bundle.public)
}

/// The migration write path is deliberately a narrow adapter over the existing planner and
/// transactional executor. It never creates missing targets or executes unselected actions.
pub(crate) fn apply_reviewed_migration(
    default_root: &Utf8Path,
    reviewed_digest: &str,
    selected: &BTreeSet<String>,
) -> Result<MigrationAdoptionReport, CoreError> {
    let roots = paths::resolve_sync_roots(default_root);
    let ledger_path = roots
        .deploy_base
        .join(".agent-manager/projection-ledger.sqlite");
    let secrets = LifecycleMcpSecrets::load()?;
    let bundle =
        build_with_existing_ledger(&roots, ProjectionOperation::Sync, &ledger_path, &secrets)?;
    validate_migration_review(&bundle, reviewed_digest, selected)?;

    // Validation above happens before opening Store, so stale, foreign, or unknown selections
    // cannot create a SQLite file as an observable side effect.
    let store = Store::open_at(ledger_path.as_std_path())
        .map_err(|error| CoreError::ProjectionLedger(error.to_string()))?;
    apply_selected_adoptions(&bundle, &store.projections(), &roots, &secrets, selected)
        .map_err(|error| error.error)
}

pub(crate) fn rollback_reviewed_migration(
    default_root: &Utf8Path,
    transaction_id: &str,
) -> Result<ProjectionRollbackReport, CoreError> {
    let roots = paths::resolve_sync_roots(default_root);
    let ledger_path = roots
        .deploy_base
        .join(".agent-manager/projection-ledger.sqlite");
    let metadata = std::fs::symlink_metadata(ledger_path.as_std_path())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CoreError::ProjectionLedger(
            "durable adoption rollback requires a direct existing ownership ledger".to_owned(),
        ));
    }
    let store = Store::open_at(ledger_path.as_std_path())
        .map_err(|error| CoreError::ProjectionLedger(error.to_string()))?;
    let ledger = store.projections();
    let context = ExecutorContext::new(
        &ledger,
        roots.deploy_base.clone(),
        roots.deploy_base.join(".agent-manager/projection-backups"),
    );
    rollback_projection_transaction(&context, transaction_id)
}

fn execute_workspace(
    default_root: &Utf8Path,
    apply: bool,
    uninstall: bool,
) -> Result<(WorkspaceLifecycleReport, u8), CoreError> {
    let workspace_root = paths::resolve_project_roots(default_root).0;
    let operation = if uninstall {
        ProjectionOperation::Uninstall
    } else {
        ProjectionOperation::Sync
    };
    let ledger_path = workspace_root.join(".agent-manager/projection-ledger.sqlite");
    let secrets = LifecycleMcpSecrets::load()?;
    let bundles = if has_ledger_entry(&ledger_path) {
        match Store::open_read_only_at(ledger_path.as_std_path()) {
            Ok(store) => match build_workspace_bundles(
                &workspace_root,
                operation,
                &store.projections(),
                &secrets,
            ) {
                Ok(bundles) => bundles,
                Err(CoreError::ProjectionLedger(_)) => {
                    build_workspace_bundles_with_unavailable_ledger(
                        &workspace_root,
                        operation,
                        &secrets,
                    )?
                }
                Err(error) => return Err(error),
            },
            Err(_) => build_workspace_bundles_with_unavailable_ledger(
                &workspace_root,
                operation,
                &secrets,
            )?,
        }
    } else {
        let ledger = MemoryProjectionLedger::default();
        build_workspace_bundles(&workspace_root, operation, &ledger, &secrets)?
    };

    if !apply {
        return Ok((workspace_report(bundles, None, false), exit_code::SUCCESS));
    }
    if bundles
        .iter()
        .any(|bundle| bundle.bundle.public.ledger_status.as_deref() == Some("ledger_unavailable"))
    {
        return Err(CoreError::ProjectionLedger(
            "projection ownership ledger is unavailable".to_owned(),
        ));
    }
    if bundles
        .iter()
        .any(|bundle| blocking_reason(&bundle.bundle).is_some())
    {
        return Ok((
            workspace_report(bundles, None, false),
            exit_code::PARTIAL_FAILURE,
        ));
    }

    let store = Store::open_at(ledger_path.as_std_path())
        .map_err(|error| CoreError::ProjectionLedger(error.to_string()))?;
    let summaries =
        match apply_workspace_bundles(&bundles, &store.projections(), &workspace_root, &secrets) {
            Ok(summaries) => summaries,
            Err(failure) => {
                if !failure.has_reports {
                    return Err(failure.error);
                }
                return Ok((
                    workspace_report(bundles, Some(failure.summaries), true),
                    exit_code::PARTIAL_FAILURE,
                ));
            }
        };
    let missing_mcp_secrets = summaries.iter().any(ApplySummary::has_missing_mcp_secrets);
    Ok((
        workspace_report(bundles, Some(summaries), false),
        if missing_mcp_secrets {
            exit_code::SECRETS_MISSING
        } else {
            exit_code::SUCCESS
        },
    ))
}

fn build_workspace_bundles(
    workspace_root: &Utf8Path,
    operation: ProjectionOperation,
    ledger: &dyn ProjectionLedger,
    secrets: &LifecycleMcpSecrets,
) -> Result<Vec<WorkspaceBundle>, CoreError> {
    let global_default = paths::discover_global_asset_root_read_only();
    let workspace_assets = paths::project_asset_root(workspace_root);
    agent_manager_core::workspace::discover_members(workspace_root)?
        .into_iter()
        .filter(|member| member != workspace_root)
        .map(|member| {
            let asset_root = paths::project_asset_root(&member);
            (member, asset_root)
        })
        .map(|(member, asset_root)| {
            let roots = SyncRoots {
                repo_root: member.clone(),
                asset_root,
                global_default: global_default.clone(),
                deploy_base: member.clone(),
            };
            let overlay = OverlayRoots {
                global: global_default.clone(),
                workspace: Some(workspace_assets.clone()),
                project: roots.asset_root.clone(),
            };
            Ok(WorkspaceBundle {
                member,
                bundle: build_bundle_with_overlay(
                    &roots,
                    overlay,
                    DeploymentScope::Project,
                    operation,
                    ledger,
                    secrets,
                )?,
            })
        })
        .collect()
}

fn build_workspace_bundles_with_unavailable_ledger(
    workspace_root: &Utf8Path,
    operation: ProjectionOperation,
    secrets: &LifecycleMcpSecrets,
) -> Result<Vec<WorkspaceBundle>, CoreError> {
    let ledger = MemoryProjectionLedger::default();
    let mut bundles = build_workspace_bundles(workspace_root, operation, &ledger, secrets)?;
    for bundle in &mut bundles {
        mark_ledger_unavailable(&mut bundle.bundle);
    }
    Ok(bundles)
}

fn workspace_report(
    bundles: Vec<WorkspaceBundle>,
    summaries: Option<Vec<ApplySummary>>,
    transaction_failed: bool,
) -> WorkspaceLifecycleReport {
    let members = bundles
        .into_iter()
        .enumerate()
        .map(|(index, bundle)| {
            let blocking_reason = summaries
                .as_ref()
                .and_then(|summaries| summaries.get(index))
                .filter(|summary| summary.has_missing_mcp_secrets())
                .map(|_| "mcp_missing_secret_keys".to_owned())
                .or_else(|| transaction_failed.then_some("transaction_apply_failed".to_owned()))
                .or_else(|| blocking_reason(&bundle.bundle));
            WorkspaceMemberReport {
                member: bundle.member.to_string(),
                plan: bundle.bundle.public,
                apply: summaries
                    .as_ref()
                    .and_then(|summaries| summaries.get(index).cloned()),
                blocking_reason,
            }
        })
        .collect();
    WorkspaceLifecycleReport { members }
}

fn apply_workspace_bundles(
    bundles: &[WorkspaceBundle],
    ledger: &dyn ProjectionLedger,
    workspace_root: &Utf8Path,
    secrets: &LifecycleMcpSecrets,
) -> Result<Vec<ApplySummary>, Box<WorkspaceApplyFailure>> {
    let plan_members = bundles
        .iter()
        .enumerate()
        .flat_map(|(member_index, bundle)| {
            bundle
                .bundle
                .plans
                .iter()
                .filter(|plan| !plan.actions.is_empty())
                .map(move |plan| (member_index, plan))
        })
        .collect::<Vec<_>>();
    let backup_root = workspace_root.join(".agent-manager/projection-backups");
    let context = ExecutorContext::new(ledger, workspace_root.to_path_buf(), backup_root)
        .with_mcp_secret_provider(secrets);
    let reports = match apply_projection_plans_transactionally(
        plan_members
            .iter()
            .map(|(_, plan)| (*plan, ApplyOptions::for_plan(plan))),
        &context,
    ) {
        Ok(reports) => reports,
        Err(error) => {
            let ProjectionTransactionError { error, reports } = error;
            let has_reports = !reports.is_empty();
            let mut summaries = vec![ApplySummary::default(); bundles.len()];
            for ((member_index, _), report) in plan_members.into_iter().zip(reports) {
                add_apply_report(&mut summaries[member_index], report);
            }
            return Err(Box::new(WorkspaceApplyFailure {
                error,
                has_reports,
                summaries,
            }));
        }
    };
    let mut summaries = vec![ApplySummary::default(); bundles.len()];
    for ((member_index, _), report) in plan_members.into_iter().zip(reports) {
        add_apply_report(&mut summaries[member_index], report);
    }
    Ok(summaries)
}

fn emit_workspace_report(mode: OutputMode, report: &WorkspaceLifecycleReport) {
    if mode.is_json() {
        emit_json(mode, report);
    } else if !mode.is_quiet() {
        emit_line(
            mode,
            format!("workspace projection: {} members", report.members.len()),
        );
    }
}

fn emit_report(mode: OutputMode, report: &LifecycleReport) {
    if mode.is_json() {
        emit_json(mode, report);
    } else if !mode.is_quiet() {
        let action_count = report.plan.actions.len();
        let suffix = report
            .apply
            .as_ref()
            .map(|apply| format!(", changed={}, unchanged={}", apply.changed, apply.unchanged))
            .unwrap_or_else(|| ", plan-only".to_owned());
        emit_line(
            mode,
            format!("projection plan: {action_count} actions{suffix}"),
        );
    }
}

fn build_with_existing_ledger(
    roots: &SyncRoots,
    operation: ProjectionOperation,
    ledger_path: &Utf8Path,
    secrets: &LifecycleMcpSecrets,
) -> Result<PlanBundle, CoreError> {
    if has_ledger_entry(ledger_path) {
        match Store::open_read_only_at(ledger_path.as_std_path()) {
            Ok(store) => match build_bundle(roots, operation, &store.projections(), secrets) {
                Ok(bundle) => Ok(bundle),
                Err(CoreError::ProjectionLedger(_)) => {
                    build_bundle_with_unavailable_ledger(roots, operation, secrets)
                }
                Err(error) => Err(error),
            },
            Err(_) => build_bundle_with_unavailable_ledger(roots, operation, secrets),
        }
    } else {
        let ledger = MemoryProjectionLedger::default();
        build_bundle(roots, operation, &ledger, secrets)
    }
}

fn build_bundle_with_unavailable_ledger(
    roots: &SyncRoots,
    operation: ProjectionOperation,
    secrets: &LifecycleMcpSecrets,
) -> Result<PlanBundle, CoreError> {
    let ledger = MemoryProjectionLedger::default();
    let mut bundle = build_bundle(roots, operation, &ledger, secrets)?;
    mark_ledger_unavailable(&mut bundle);
    Ok(bundle)
}

fn build_bundle(
    roots: &SyncRoots,
    operation: ProjectionOperation,
    ledger: &dyn ProjectionLedger,
    secrets: &LifecycleMcpSecrets,
) -> Result<PlanBundle, CoreError> {
    let scope = if paths::is_project_deploy_base(&roots.deploy_base) {
        DeploymentScope::Project
    } else {
        DeploymentScope::User
    };
    let overlay = overlay_roots(roots);
    build_bundle_with_overlay(roots, overlay, scope, operation, ledger, secrets)
}

fn build_bundle_with_overlay(
    roots: &SyncRoots,
    overlay: OverlayRoots,
    scope: DeploymentScope,
    operation: ProjectionOperation,
    ledger: &dyn ProjectionLedger,
    secrets: &LifecycleMcpSecrets,
) -> Result<PlanBundle, CoreError> {
    let assets = resolve_effective_assets(&overlay)?;
    let definitions = resolve_effective_mcp_definitions(&overlay)?;
    let scope_key = match scope {
        DeploymentScope::User => format!("user:{}", roots.deploy_base),
        DeploymentScope::Workspace => format!("workspace:{}", roots.deploy_base),
        DeploymentScope::Project => format!("project:{}", roots.deploy_base),
    };
    let non_mcp = assets
        .iter()
        .filter(|asset| asset.kind != AssetKind::Mcp)
        .cloned()
        .collect::<Vec<_>>();
    let normal_request = ProjectionRequest {
        operation,
        scope_key: scope_key.clone(),
        scope,
        deploy_base: roots.deploy_base.clone(),
        assets: non_mcp.clone(),
        platforms: vec![PlatformId::Cursor, PlatformId::Codex, PlatformId::Claude],
    };
    let mcp_request = ProjectionRequest {
        operation,
        scope_key: scope_key.clone(),
        scope,
        deploy_base: roots.deploy_base.clone(),
        assets: Vec::new(),
        platforms: vec![PlatformId::Cursor, PlatformId::Codex, PlatformId::Claude],
    };
    let non_mcp_ledger = NonMcpLedger { inner: ledger };
    let mcp_ledger = McpOnlyLedger { inner: ledger };
    let normal_context = PlannerContext::new(&non_mcp_ledger);
    let mcp_context = PlannerContext::new(&mcp_ledger).with_mcp_secret_availability(secrets);
    let context = PlannerContext::new(ledger).with_mcp_secret_availability(secrets);
    let mut plans = vec![
        build_projection_plan(&normal_request, &normal_context)?,
        build_mcp_projection_plan(&mcp_request, &definitions, &mcp_context)?,
    ];
    // Hermes project scope is explicitly unsupported and must never touch user config.yaml.
    // At user scope its MCP, external skill directory and Hook intents are planned together.
    if scope == DeploymentScope::User {
        let hermes_assets = non_mcp
            .into_iter()
            .filter(|asset| matches!(asset.kind, AssetKind::Skill | AssetKind::Hook))
            .collect();
        let hermes_request = ProjectionRequest {
            operation,
            scope_key,
            scope,
            deploy_base: roots.deploy_base.clone(),
            assets: hermes_assets,
            platforms: vec![PlatformId::Hermes],
        };
        plans.push(build_hermes_cross_domain_projection_plan(
            &hermes_request,
            &definitions,
            &context,
        )?);
    }
    let actions = public_actions_with_ids(&plans)?;
    // The reviewed public digest deliberately includes stable action IDs. A selection therefore
    // cannot be replayed against an action list whose ordering or identities have changed.
    let encoded = serde_json::to_vec(&actions).map_err(CoreError::Json)?;
    let public = PublicPlan {
        schema_version: PROJECTION_PLAN_SCHEMA_VERSION,
        plan_digest: hex::encode(Sha256::digest(encoded)),
        actions,
        ledger_status: None,
    };
    Ok(PlanBundle { plans, public })
}

fn public_actions_with_ids(plans: &[ProjectionPlan]) -> Result<Vec<serde_json::Value>, CoreError> {
    let mut actions = Vec::new();
    for plan in plans {
        if plan.actions.len() != plan.action_ids.len() {
            return Err(CoreError::InvalidPath(
                "projection plan action IDs do not match its actions".to_owned(),
            ));
        }
        for (action, action_id) in plan.actions.iter().zip(&plan.action_ids) {
            let mut value = serde_json::to_value(action).map_err(CoreError::Json)?;
            let object = value.as_object_mut().ok_or_else(|| {
                CoreError::InvalidPath(
                    "projection action did not serialize as an object".to_owned(),
                )
            })?;
            object.insert(
                "action_id".to_owned(),
                serde_json::Value::String(action_id.clone()),
            );
            // Keep the CLI's workspace-overlay planner aligned with the core facade's public
            // contract: a foreign direct target must be reviewed through import, never adopted.
            if action.kind == ProjectionActionKind::ReportOnly
                && action.state.as_deref() == Some("foreign")
                && !action.members.is_empty()
                && action.mcp_members.is_empty()
            {
                object.insert(
                    "reason_code".to_owned(),
                    serde_json::Value::String("import_required".to_owned()),
                );
            }
            actions.push(value);
        }
    }
    Ok(actions)
}

fn overlay_roots(roots: &SyncRoots) -> OverlayRoots {
    if paths::is_project_deploy_base(&roots.deploy_base) {
        OverlayRoots {
            global: roots.global_default.clone(),
            workspace: None,
            project: roots.asset_root.clone(),
        }
    } else {
        OverlayRoots {
            global: roots.asset_root.clone(),
            workspace: None,
            project: roots.asset_root.join(".agent-manager-no-project-overlay"),
        }
    }
}

fn blocking_reason(bundle: &PlanBundle) -> Option<String> {
    if let Some(status) = &bundle.public.ledger_status {
        return Some(status.clone());
    }
    bundle
        .plans
        .iter()
        .flat_map(|plan| plan.actions.iter())
        .find(|action| {
            action.kind == ProjectionActionKind::ReportOnly && !is_missing_secret_skip(action)
        })
        .map(|action| action.reason_code.clone())
}

fn mark_ledger_unavailable(bundle: &mut PlanBundle) {
    bundle.public.ledger_status = Some("ledger_unavailable".to_owned());
}

fn has_ledger_entry(path: &Utf8Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

fn is_missing_secret_skip(action: &agent_manager_core::projection::planner::ProjectionAction) -> bool {
    action.state.as_deref() == Some("skipped")
        && action.reason_code == "mcp_missing_secret_keys"
        && !action.mcp_members.is_empty()
        && action
            .mcp_members
            .iter()
            .all(|member| !member.missing_secret_keys.is_empty())
}

fn apply_bundle(
    bundle: &PlanBundle,
    ledger: &dyn ProjectionLedger,
    roots: &SyncRoots,
    secrets: &LifecycleMcpSecrets,
) -> Result<ApplySummary, ProjectionTransactionError> {
    let mut summary = ApplySummary::default();
    let backup_root = roots.deploy_base.join(".agent-manager/projection-backups");
    let context = ExecutorContext::new(ledger, roots.deploy_base.clone(), backup_root)
        .with_mcp_secret_provider(secrets);
    let reports = apply_projection_plans_transactionally(
        bundle
            .plans
            .iter()
            .filter(|plan| !plan.actions.is_empty())
            .map(|plan| (plan, ApplyOptions::for_plan(plan))),
        &context,
    )?;
    for report in reports {
        add_apply_report(&mut summary, report);
    }
    Ok(summary)
}

fn validate_migration_review(
    bundle: &PlanBundle,
    reviewed_digest: &str,
    selected: &BTreeSet<String>,
) -> Result<(), CoreError> {
    if bundle.public.ledger_status.as_deref() == Some("ledger_unavailable") {
        return Err(CoreError::InvalidPath(
            "source-first migration cannot apply while the ownership ledger is unavailable"
                .to_owned(),
        ));
    }
    if reviewed_digest != bundle.public.plan_digest {
        return Err(CoreError::InvalidPath(
            "reviewed migration plan digest is stale or does not match current source-first plan"
                .to_owned(),
        ));
    }
    if selected.is_empty() {
        return Err(CoreError::InvalidPath(
            "source-first migration requires at least one selected action ID".to_owned(),
        ));
    }
    for selected_id in selected {
        let candidate = bundle.plans.iter().find_map(|plan| {
            plan.action_ids
                .iter()
                .position(|action_id| action_id == selected_id)
                .map(|index| &plan.actions[index])
        });
        let Some(action) = candidate else {
            return Err(CoreError::InvalidPath(
                "selected migration action ID is not part of the current plan".to_owned(),
            ));
        };
        if action.kind != ProjectionActionKind::AdoptEquivalent {
            return Err(CoreError::InvalidPath(
                "only an AdoptEquivalent migration action may be selected".to_owned(),
            ));
        }
    }
    Ok(())
}

fn apply_selected_adoptions(
    bundle: &PlanBundle,
    ledger: &dyn ProjectionLedger,
    roots: &SyncRoots,
    secrets: &LifecycleMcpSecrets,
    selected: &BTreeSet<String>,
) -> Result<MigrationAdoptionReport, ProjectionTransactionError> {
    let plans = bundle
        .plans
        .iter()
        .filter_map(|plan| {
            let indexes = plan
                .action_ids
                .iter()
                .enumerate()
                .filter_map(|(index, action_id)| selected.contains(action_id).then_some(index))
                .collect::<Vec<_>>();
            (!indexes.is_empty()).then(|| ProjectionPlan {
                schema_version: plan.schema_version,
                actions: indexes
                    .iter()
                    .map(|index| plan.actions[*index].clone())
                    .collect(),
                action_ids: indexes
                    .iter()
                    .map(|index| plan.action_ids[*index].clone())
                    .collect(),
                trust_requirements: indexes
                    .iter()
                    .map(|index| plan.trust_requirements[*index])
                    .collect(),
                warnings: Vec::new(),
                // The executor's internal digest authorization only needs to bind this freshly
                // rebuilt core slice; the public reviewed digest was verified above.
                plan_digest: plan.plan_digest.clone(),
            })
        })
        .collect::<Vec<_>>();
    let backup_root = roots.deploy_base.join(".agent-manager/projection-backups");
    let context = ExecutorContext::new(ledger, roots.deploy_base.clone(), backup_root)
        .with_mcp_secret_provider(secrets);
    let reports = apply_projection_plans_transactionally(
        plans.iter().map(|plan| {
            (
                plan,
                ApplyOptions::with_selected_action_ids(plan, plan.action_ids.clone()),
            )
        }),
        &context,
    )?;
    let transaction_ids = reports
        .iter()
        .filter_map(|report| report.transaction_id.clone())
        .collect::<BTreeSet<_>>();
    let transaction_id = if transaction_ids.len() == 1 {
        transaction_ids.into_iter().next().expect("checked length")
    } else {
        return Err(ProjectionTransactionError {
            error: CoreError::ProjectionLedger(
                "selected adoption did not produce one durable transaction ID".to_owned(),
            ),
            reports,
        });
    };
    Ok(MigrationAdoptionReport {
        transaction_id,
        apply: summarize_apply_reports(reports),
    })
}

fn summarize_apply_reports(reports: Vec<ApplyReport>) -> ApplySummary {
    let mut summary = ApplySummary::default();
    for report in reports {
        add_apply_report(&mut summary, report);
    }
    summary
}

fn add_apply_report(summary: &mut ApplySummary, report: ApplyReport) {
    summary.changed += report.changed;
    summary.unchanged += report.unchanged;
    summary.skipped += report.skipped;
    summary.conflict += report.conflict;
    summary.failed += report.failed;
    summary.rolled_back += report.rolled_back;
    summary.rollback_failed += report.rollback_failed;
    summary.not_applied += report.not_applied;
    summary
        .mcp_skipped_members
        .extend(
            report
                .mcp_skipped_members
                .into_iter()
                .map(|member| McpSkippedMemberSummary {
                    entry_key: member.entry_key,
                    missing_secret_keys: member.missing_secret_keys,
                }),
        );
}
