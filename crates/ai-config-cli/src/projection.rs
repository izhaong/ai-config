//! Public install / sync / uninstall projection lifecycle.
//!
//! Planning is always read-only. The first persistent ownership ledger is created only after
//! the caller supplies --apply, so a default invocation cannot initialize a platform target.

use std::process::ExitCode;

use ai_config_core::error::{exit_code, CoreError};
use ai_config_core::model::{AssetKind, PlatformId};
use ai_config_core::paths::{self, SyncRoots};
use ai_config_core::projection::executor::{
    apply_projection_plans_transactionally, ApplyOptions, ExecutorContext, McpSecretProvider,
};
use ai_config_core::projection::ledger::{MemoryProjectionLedger, ProjectionLedger};
use ai_config_core::projection::mcp::source::resolve_effective_mcp_definitions;
use ai_config_core::projection::model::{
    DeploymentScope, LedgerMutation, ProjectionId, ProjectionRecord,
};
use ai_config_core::projection::planner::{
    build_hermes_cross_domain_projection_plan, build_mcp_projection_plan, build_projection_plan,
    PlannerContext, ProjectionActionKind, ProjectionOperation, ProjectionPlan,
    ProjectionRequest, PROJECTION_PLAN_SCHEMA_VERSION,
};
use ai_config_core::projection::source::{resolve_effective_assets, OverlayRoots};
use ai_config_store::Store;
use camino::Utf8Path;
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
}

#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct ApplySummary {
    pub changed: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub conflict: usize,
    pub failed: usize,
    pub rolled_back: usize,
    pub rollback_failed: usize,
    pub not_applied: usize,
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

struct EmptySecretProvider;

impl McpSecretProvider for EmptySecretProvider {
    fn resolve(&self, _key: &str) -> Result<Option<String>, CoreError> {
        Ok(None)
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
            "workspace projection lifecycle is not implemented yet; remove --workspace or wait for the workspace projection request adapter",
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
        .join(".ai-config/projection-ledger.sqlite");
    let bundle = build_with_existing_ledger(&roots, operation, &ledger_path)?;
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
    let (report, exit_code) = match apply_bundle(&bundle, &ledger, &roots) {
        Ok(summary) => (
            LifecycleReport {
                plan: bundle.public,
                apply: Some(summary),
                blocking_reason: None,
            },
            exit_code::SUCCESS,
        ),
        Err(error) => (
            LifecycleReport {
                plan: bundle.public,
                apply: None,
                blocking_reason: Some(error.to_string()),
            },
            error.exit_code(),
        ),
    };
    Ok(LifecycleExecution { report, exit_code })
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
) -> Result<PlanBundle, CoreError> {
    if ledger_path.is_file() {
        let store = Store::open_at(ledger_path.as_std_path())
            .map_err(|error| CoreError::ProjectionLedger(error.to_string()))?;
        build_bundle(roots, operation, &store.projections())
    } else {
        let ledger = MemoryProjectionLedger::default();
        build_bundle(roots, operation, &ledger)
    }
}

fn build_bundle(
    roots: &SyncRoots,
    operation: ProjectionOperation,
    ledger: &dyn ProjectionLedger,
) -> Result<PlanBundle, CoreError> {
    let scope = if paths::is_project_deploy_base(&roots.deploy_base) {
        DeploymentScope::Project
    } else {
        DeploymentScope::User
    };
    let overlay = overlay_roots(roots);
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
    let mcp_context = PlannerContext::new(&mcp_ledger);
    let context = PlannerContext::new(ledger);
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
    let core_actions = plans
        .iter()
        .flat_map(|plan| plan.actions.iter().cloned())
        .collect::<Vec<_>>();
    let encoded = serde_json::to_vec(&core_actions).map_err(CoreError::Json)?;
    let actions = serde_json::from_slice(&encoded).map_err(CoreError::Json)?;
    let public = PublicPlan {
        schema_version: PROJECTION_PLAN_SCHEMA_VERSION,
        plan_digest: hex::encode(Sha256::digest(encoded)),
        actions,
    };
    Ok(PlanBundle { plans, public })
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
            project: roots.asset_root.join(".ai-config-no-project-overlay"),
        }
    }
}

fn blocking_reason(bundle: &PlanBundle) -> Option<String> {
    bundle
        .plans
        .iter()
        .flat_map(|plan| plan.actions.iter())
        .find(|action| action.kind == ProjectionActionKind::ReportOnly)
        .map(|action| action.reason_code.clone())
}

fn apply_bundle(
    bundle: &PlanBundle,
    ledger: &dyn ProjectionLedger,
    roots: &SyncRoots,
) -> Result<ApplySummary, CoreError> {
    let mut summary = ApplySummary::default();
    let secrets = EmptySecretProvider;
    let backup_root = roots.deploy_base.join(".ai-config/projection-backups");
    let context = ExecutorContext::new(ledger, roots.deploy_base.clone(), backup_root)
        .with_mcp_secret_provider(&secrets);
    let reports = apply_projection_plans_transactionally(
        bundle
            .plans
            .iter()
            .filter(|plan| !plan.actions.is_empty())
            .map(|plan| (plan, ApplyOptions::for_plan(plan))),
        &context,
    )?;
    for report in reports {
        summary.changed += report.changed;
        summary.unchanged += report.unchanged;
        summary.skipped += report.skipped;
        summary.conflict += report.conflict;
        summary.failed += report.failed;
        summary.rolled_back += report.rolled_back;
        summary.rollback_failed += report.rollback_failed;
        summary.not_applied += report.not_applied;
    }
    Ok(summary)
}
