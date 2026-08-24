//! Shared read-only source-first projection planning facade.
//!
//! CLI, MCP, and Tauri provide their own persistence boundary, but must receive the same
//! reviewed actions and digest from core before any of them opens a writable ownership ledger.

use sha2::{Digest, Sha256};

use crate::error::CoreError;
use crate::model::{AssetKind, PlatformId};
use crate::paths::{self, SyncRoots};

use super::ledger::ProjectionLedger;
use super::mcp::source::resolve_effective_mcp_definitions;
use super::planner::{
    build_hermes_cross_domain_projection_plan, build_mcp_projection_plan, build_projection_plan,
    McpSecretAvailability, PlannerContext, ProjectionActionKind, ProjectionOperation,
    ProjectionPlan, ProjectionRequest, PROJECTION_PLAN_SCHEMA_VERSION,
};
use super::source::{resolve_effective_assets, OverlayRoots};

/// The review payload shared by every public transport. It never includes asset bodies or
/// secret values; callers must bind any later apply to `plan_digest`.
#[derive(Debug, Clone)]
pub struct ProjectionReview {
    pub schema_version: u16,
    pub plan_digest: String,
    pub actions: Vec<serde_json::Value>,
    pub plans: Vec<ProjectionPlan>,
    pub blocking_reason: Option<String>,
}

struct NonMcpLedger<'a> {
    inner: &'a dyn ProjectionLedger,
}

struct McpOnlyLedger<'a> {
    inner: &'a dyn ProjectionLedger,
}

impl ProjectionLedger for NonMcpLedger<'_> {
    fn get(
        &self,
        id: &super::model::ProjectionId,
    ) -> Result<Option<super::model::ProjectionRecord>, CoreError> {
        self.inner.get(id)
    }

    fn get_many(
        &self,
        ids: &[super::model::ProjectionId],
    ) -> Result<Vec<super::model::ProjectionRecord>, CoreError> {
        self.inner.get_many(ids)
    }

    fn list_scope(
        &self,
        scope_key: &str,
    ) -> Result<Vec<super::model::ProjectionRecord>, CoreError> {
        Ok(self
            .inner
            .list_scope(scope_key)?
            .into_iter()
            .filter(|record| record.id.kind != AssetKind::Mcp)
            .collect())
    }

    fn apply_batch(&self, mutations: &[super::model::LedgerMutation]) -> Result<(), CoreError> {
        self.inner.apply_batch(mutations)
    }
}

impl ProjectionLedger for McpOnlyLedger<'_> {
    fn get(
        &self,
        id: &super::model::ProjectionId,
    ) -> Result<Option<super::model::ProjectionRecord>, CoreError> {
        self.inner.get(id)
    }

    fn get_many(
        &self,
        ids: &[super::model::ProjectionId],
    ) -> Result<Vec<super::model::ProjectionRecord>, CoreError> {
        self.inner.get_many(ids)
    }

    fn list_scope(
        &self,
        _scope_key: &str,
    ) -> Result<Vec<super::model::ProjectionRecord>, CoreError> {
        Ok(Vec::new())
    }

    fn apply_batch(&self, mutations: &[super::model::LedgerMutation]) -> Result<(), CoreError> {
        self.inner.apply_batch(mutations)
    }
}

pub fn build_projection_review(
    roots: &SyncRoots,
    operation: ProjectionOperation,
    ledger: &dyn ProjectionLedger,
    secrets: &dyn McpSecretAvailability,
) -> Result<ProjectionReview, CoreError> {
    let scope = if paths::is_project_deploy_base(&roots.deploy_base) {
        super::model::DeploymentScope::Project
    } else {
        super::model::DeploymentScope::User
    };
    let overlay = if scope == super::model::DeploymentScope::Project {
        OverlayRoots {
            global: roots.global_default.clone(),
            workspace: None,
            project: roots.asset_root.clone(),
        }
    } else {
        OverlayRoots {
            global: roots.asset_root.clone(),
            workspace: None,
            project: roots.asset_root.join(".agents-manager-no-project-overlay"),
        }
    };
    let assets = resolve_effective_assets(&overlay)?;
    let definitions = resolve_effective_mcp_definitions(&overlay)?;
    let scope_key = match scope {
        super::model::DeploymentScope::User => format!("user:{}", roots.deploy_base),
        super::model::DeploymentScope::Workspace => format!("workspace:{}", roots.deploy_base),
        super::model::DeploymentScope::Project => format!("project:{}", roots.deploy_base),
    };
    let non_mcp = assets
        .into_iter()
        .filter(|asset| asset.kind != AssetKind::Mcp)
        .collect::<Vec<_>>();
    let request = ProjectionRequest {
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
    let mut plans = vec![
        build_projection_plan(&request, &PlannerContext::new(&non_mcp_ledger))?,
        build_mcp_projection_plan(
            &mcp_request,
            &definitions,
            &PlannerContext::new(&mcp_ledger).with_mcp_secret_availability(secrets),
        )?,
    ];
    if scope == super::model::DeploymentScope::User {
        let hermes_assets = non_mcp
            .into_iter()
            .filter(|asset| matches!(asset.kind, AssetKind::Skill | AssetKind::Hook))
            .collect();
        let request = ProjectionRequest {
            operation,
            scope_key,
            scope,
            deploy_base: roots.deploy_base.clone(),
            assets: hermes_assets,
            platforms: vec![PlatformId::Hermes],
        };
        plans.push(build_hermes_cross_domain_projection_plan(
            &request,
            &definitions,
            &PlannerContext::new(ledger).with_mcp_secret_availability(secrets),
        )?);
    }
    let mut actions = Vec::new();
    for plan in &plans {
        if plan.actions.len() != plan.action_ids.len() {
            return Err(CoreError::InvalidPath(
                "projection action IDs do not match actions".to_owned(),
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
            // A direct foreign target is never adopted by projection. Public callers must
            // route it to the explicit reviewed import workflow instead of presenting it as
            // a generic conflict.
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
    let plan_digest = hex::encode(Sha256::digest(
        serde_json::to_vec(&actions).map_err(CoreError::Json)?,
    ));
    let blocking_reason = plans
        .iter()
        .flat_map(|plan| plan.actions.iter())
        .find(|action| {
            action.kind == ProjectionActionKind::ReportOnly
                && action.state.as_deref() != Some("skipped")
        })
        .map(|action| action.reason_code.clone());
    Ok(ProjectionReview {
        schema_version: PROJECTION_PLAN_SCHEMA_VERSION,
        plan_digest,
        actions,
        plans,
        blocking_reason,
    })
}
