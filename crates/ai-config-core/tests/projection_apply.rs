use std::fs;

use ai_config_core::error::CoreError;
use ai_config_core::model::{AssetKind, PlatformId};
use ai_config_core::projection::executor::{
    apply_projection_plan, apply_projection_plans_transactionally, rollback_projection_transaction,
    ApplyActionStatus, ApplyOptions, ExecutorContext,
};
use ai_config_core::projection::fingerprint::path_content_digest;
use ai_config_core::projection::ledger::MemoryProjectionLedger;
use ai_config_core::projection::ledger::ProjectionLedger;
use ai_config_core::projection::model::{
    DeploymentScope, EffectiveAsset, LedgerMutation, ProjectionId, ProjectionMode,
    ProjectionRecord, SourceLayer,
};
use ai_config_core::projection::planner::{
    build_projection_plan, CopyFallbackAuthorization, CopyFallbackPolicy, LinkAvailability,
    PlannerContext, ProjectionActionKind, ProjectionOperation, ProjectionRequest,
};
use camino::Utf8Path;
use tempfile::TempDir;

struct FailingLedger;

impl ProjectionLedger for FailingLedger {
    fn get(&self, _id: &ProjectionId) -> Result<Option<ProjectionRecord>, CoreError> {
        Ok(None)
    }

    fn get_many(&self, _ids: &[ProjectionId]) -> Result<Vec<ProjectionRecord>, CoreError> {
        Ok(Vec::new())
    }

    fn list_scope(&self, _scope_key: &str) -> Result<Vec<ProjectionRecord>, CoreError> {
        Ok(Vec::new())
    }

    fn apply_batch(&self, _mutations: &[LedgerMutation]) -> Result<(), CoreError> {
        Err(CoreError::ProjectionLedger(
            "test ledger write failure".to_owned(),
        ))
    }
}

struct FailingLedgerWithRecord {
    record: ProjectionRecord,
}

impl ProjectionLedger for FailingLedgerWithRecord {
    fn get(&self, id: &ProjectionId) -> Result<Option<ProjectionRecord>, CoreError> {
        Ok((id == &self.record.id).then(|| self.record.clone()))
    }

    fn get_many(&self, ids: &[ProjectionId]) -> Result<Vec<ProjectionRecord>, CoreError> {
        Ok(if ids.iter().any(|id| id == &self.record.id) {
            vec![self.record.clone()]
        } else {
            Vec::new()
        })
    }

    fn list_scope(&self, scope_key: &str) -> Result<Vec<ProjectionRecord>, CoreError> {
        Ok(if scope_key == self.record.id.scope_key {
            vec![self.record.clone()]
        } else {
            Vec::new()
        })
    }

    fn apply_batch(&self, _mutations: &[LedgerMutation]) -> Result<(), CoreError> {
        Err(CoreError::ProjectionLedger(
            "test ledger write failure".to_owned(),
        ))
    }
}

fn skill(root: &Utf8Path, name: &str) -> EffectiveAsset {
    let source_path = root.join("source/skills").join(name);
    fs::create_dir_all(source_path.as_std_path()).unwrap();
    fs::write(
        source_path.join("SKILL.md").as_std_path(),
        "canonical skill",
    )
    .unwrap();
    EffectiveAsset {
        kind: AssetKind::Skill,
        name: name.to_owned(),
        fingerprint: path_content_digest(&source_path).unwrap(),
        source_path,
        layer: SourceLayer::Project,
    }
}

fn rule(root: &Utf8Path, name: &str) -> EffectiveAsset {
    let source_path = root.join("source/rules").join(format!("{name}.mdc"));
    fs::create_dir_all(source_path.parent().unwrap().as_std_path()).unwrap();
    fs::write(source_path.as_std_path(), "---\ndescription: review\n---\n").unwrap();
    EffectiveAsset {
        kind: AssetKind::Rule,
        name: name.to_owned(),
        fingerprint: path_content_digest(&source_path).unwrap(),
        source_path,
        layer: SourceLayer::Project,
    }
}

#[cfg(unix)]
#[test]
fn apply_creates_an_exact_link_for_a_missing_direct_target() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset.clone()],
        platforms: vec![PlatformId::Cursor],
    };
    fs::create_dir_all(request.deploy_base.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    let target = request.deploy_base.join(".agents/skills/review");

    let report = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    )
    .unwrap();

    assert_eq!(report.changed, 1);
    assert_eq!(
        fs::read_link(target.as_std_path()).unwrap(),
        asset.source_path
    );
}

#[cfg(unix)]
#[test]
fn transactional_apply_failure_preserves_failed_rolled_back_and_not_applied_reports() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let deploy_base = root.join("deploy");
    let ledger = MemoryProjectionLedger::default();
    let plans = ["first", "second", "third"]
        .into_iter()
        .map(|name| {
            let asset = skill(root, name);
            let request = ProjectionRequest {
                operation: ProjectionOperation::Sync,
                scope_key: "project:/fixture".to_owned(),
                scope: DeploymentScope::Project,
                deploy_base: deploy_base.clone(),
                assets: vec![asset],
                platforms: vec![PlatformId::Cursor],
            };
            build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap()
        })
        .collect::<Vec<_>>();
    fs::remove_dir_all(root.join("source/skills/second").as_std_path()).unwrap();

    let error = apply_projection_plans_transactionally(
        plans
            .iter()
            .map(|plan| (plan, ApplyOptions::for_plan(plan))),
        &ExecutorContext::new(&ledger, deploy_base.clone(), root.join("backups")),
    )
    .unwrap_err();

    assert_eq!(error.reports.len(), 3);
    assert_eq!(error.reports[0].rolled_back, 1);
    assert_eq!(error.reports[1].failed, 1);
    assert_eq!(error.reports[2].not_applied, 1);
    assert!(fs::symlink_metadata(deploy_base.join(".agents/skills/first").as_std_path()).is_err());
}

#[cfg(unix)]
#[test]
fn direct_link_reads_canonical_edits_without_another_sync() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset.clone()],
        platforms: vec![PlatformId::Cursor],
    };
    fs::create_dir_all(request.deploy_base.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    let target = request.deploy_base.join(".agents/skills/review/SKILL.md");

    apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    )
    .unwrap();
    fs::write(
        asset.source_path.join("SKILL.md").as_std_path(),
        "updated canonical skill",
    )
    .unwrap();

    assert_eq!(
        fs::read_to_string(target.as_std_path()).unwrap(),
        "updated canonical skill"
    );
}

#[cfg(unix)]
#[test]
fn apply_records_direct_link_ownership_after_the_link_is_written() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset.clone()],
        platforms: vec![PlatformId::Cursor],
    };
    fs::create_dir_all(request.deploy_base.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    let id = plan.actions[0].members[0].id.clone();

    apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    )
    .unwrap();

    let record = ledger.get(&id).unwrap().unwrap();
    assert_eq!(record.mode, ProjectionMode::DirectLink);
    assert_eq!(record.source_path, asset.source_path);
    assert_eq!(
        record.target_path,
        request.deploy_base.join(".agents/skills/review")
    );
}

#[test]
fn blocking_conflict_keeps_every_target_unchanged() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset],
        platforms: vec![PlatformId::Cursor],
    };
    let target = request.deploy_base.join(".agents/skills/review");
    fs::create_dir_all(target.as_std_path()).unwrap();
    fs::write(target.join("SKILL.md").as_std_path(), "foreign skill").unwrap();
    let before = path_content_digest(&target).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    let result = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    );

    let failure = result.unwrap_err();
    assert_eq!(failure.report.changed, 0);
    assert_eq!(failure.report.conflict, 1);
    assert_eq!(failure.report.not_applied, 0);
    assert_eq!(
        failure.report.actions[0].status,
        ApplyActionStatus::Conflict
    );
    assert_eq!(path_content_digest(&target).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn apply_keeps_an_exact_existing_link_unchanged() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset.clone()],
        platforms: vec![PlatformId::Cursor],
    };
    let target = request.deploy_base.join(".agents/skills/review");
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    std::os::unix::fs::symlink(asset.source_path.as_std_path(), target.as_std_path()).unwrap();
    let before = fs::read_link(target.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    let report = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    )
    .unwrap();

    assert_eq!(report.changed, 0);
    assert_eq!(report.unchanged, 1);
    assert_eq!(fs::read_link(target.as_std_path()).unwrap(), before);
}

#[test]
fn apply_rejects_a_target_changed_after_planning() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset],
        platforms: vec![PlatformId::Cursor],
    };
    fs::create_dir_all(request.deploy_base.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    let target = request.deploy_base.join(".agents/skills/review");
    fs::create_dir_all(target.as_std_path()).unwrap();
    fs::write(target.join("SKILL.md").as_std_path(), "late foreign write").unwrap();

    let result = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    );

    assert!(result.is_err());
    assert!(target.join("SKILL.md").exists());
}

#[test]
fn apply_rejects_a_source_removed_after_planning_without_creating_a_target() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset.clone()],
        platforms: vec![PlatformId::Cursor],
    };
    fs::create_dir_all(request.deploy_base.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    fs::remove_dir_all(asset.source_path.as_std_path()).unwrap();
    let target = request.deploy_base.join(".agents/skills/review");

    let result = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    );

    assert!(result.is_err());
    assert!(fs::symlink_metadata(target.as_std_path()).is_err());
}

#[test]
fn apply_rejects_a_source_changed_after_planning_without_creating_a_target() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset.clone()],
        platforms: vec![PlatformId::Cursor],
    };
    fs::create_dir_all(request.deploy_base.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    fs::write(
        asset.source_path.join("SKILL.md").as_std_path(),
        "changed after plan",
    )
    .unwrap();
    let target = request.deploy_base.join(".agents/skills/review");

    let result = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    );

    assert!(result.is_err());
    assert!(fs::symlink_metadata(target.as_std_path()).is_err());
}

#[cfg(unix)]
#[test]
fn apply_rejects_a_symlinked_parent_that_escapes_the_deploy_root() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset],
        platforms: vec![PlatformId::Cursor],
    };
    fs::create_dir_all(request.deploy_base.as_std_path()).unwrap();
    let outside = root.join("outside");
    fs::create_dir_all(outside.as_std_path()).unwrap();
    std::os::unix::fs::symlink(
        outside.as_std_path(),
        request.deploy_base.join(".agents").as_std_path(),
    )
    .unwrap();
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    let result = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    );

    assert!(result.is_err());
    assert!(!outside.join("skills/review").exists());
}

#[cfg(unix)]
#[test]
fn apply_retract_removes_only_the_exact_managed_link() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Retract,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset.clone()],
        platforms: vec![PlatformId::Cursor],
    };
    let target = request.deploy_base.join(".agents/skills/review");
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    std::os::unix::fs::symlink(asset.source_path.as_std_path(), target.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    let report = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    )
    .unwrap();

    assert_eq!(report.changed, 1);
    assert!(fs::symlink_metadata(target.as_std_path()).is_err());
}

#[cfg(unix)]
#[test]
fn retract_removes_the_matching_ledger_record_with_the_link() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let deploy_base = root.join("deploy");
    fs::create_dir_all(deploy_base.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let sync_request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: deploy_base.clone(),
        assets: vec![asset.clone()],
        platforms: vec![PlatformId::Cursor],
    };
    let sync_plan = build_projection_plan(&sync_request, &PlannerContext::new(&ledger)).unwrap();
    let id = sync_plan.actions[0].members[0].id.clone();
    apply_projection_plan(
        &sync_plan,
        &ExecutorContext::new(&ledger, deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&sync_plan),
    )
    .unwrap();
    assert!(ledger.get(&id).unwrap().is_some());

    let retract_request = ProjectionRequest {
        operation: ProjectionOperation::Retract,
        ..sync_request
    };
    let retract_plan =
        build_projection_plan(&retract_request, &PlannerContext::new(&ledger)).unwrap();
    apply_projection_plan(
        &retract_plan,
        &ExecutorContext::new(&ledger, deploy_base, root.join("backups")),
        ApplyOptions::for_plan(&retract_plan),
    )
    .unwrap();

    assert!(ledger.get(&id).unwrap().is_none());
}

#[cfg(unix)]
#[test]
fn ledger_failure_rolls_back_a_created_link() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset],
        platforms: vec![PlatformId::Cursor],
    };
    fs::create_dir_all(request.deploy_base.as_std_path()).unwrap();
    let ledger = FailingLedger;
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    let target = request.deploy_base.join(".agents/skills/review");

    let result = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    );

    assert!(result.is_err());
    assert!(fs::symlink_metadata(target.as_std_path()).is_err());
}

#[test]
fn later_action_failure_removes_prior_links_and_their_new_empty_parents() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let alpha = skill(root, "alpha");
    let bravo = skill(root, "bravo");
    let charlie = skill(root, "charlie");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![alpha.clone(), bravo.clone(), charlie],
        platforms: vec![PlatformId::Cursor],
    };
    fs::create_dir_all(request.deploy_base.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    fs::remove_dir_all(bravo.source_path.as_std_path()).unwrap();

    let result = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    );

    let failure = result.unwrap_err();
    assert_eq!(failure.report.changed, 0);
    assert_eq!(failure.report.rolled_back, 1);
    assert_eq!(failure.report.failed, 1);
    assert_eq!(failure.report.not_applied, 1);
    assert_eq!(
        failure.report.actions[0].status,
        ApplyActionStatus::RolledBack
    );
    assert_eq!(failure.report.actions[1].status, ApplyActionStatus::Failed);
    assert_eq!(
        failure.report.actions[2].status,
        ApplyActionStatus::NotApplied
    );
    assert!(fs::symlink_metadata(request.deploy_base.join(".agents").as_std_path()).is_err());
}

#[cfg(unix)]
#[test]
fn ledger_failure_restores_a_retracted_link() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Retract,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset.clone()],
        platforms: vec![PlatformId::Cursor],
    };
    let target = request.deploy_base.join(".agents/skills/review");
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    std::os::unix::fs::symlink(asset.source_path.as_std_path(), target.as_std_path()).unwrap();
    let ledger = FailingLedger;
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    let result = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    );

    assert!(result.is_err());
    assert_eq!(
        fs::read_link(target.as_std_path()).unwrap(),
        asset.source_path
    );
}

#[cfg(unix)]
#[test]
fn apply_refuses_to_run_while_another_projection_lock_exists() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset],
        platforms: vec![PlatformId::Cursor],
    };
    fs::create_dir_all(request.deploy_base.as_std_path()).unwrap();
    fs::write(
        request
            .deploy_base
            .join(".agent-manager-projection.lock")
            .as_std_path(),
        "held",
    )
    .unwrap();
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    let target = request.deploy_base.join(".agents/skills/review");

    let result = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    );

    assert!(result.is_err());
    assert!(fs::symlink_metadata(target.as_std_path()).is_err());
}

#[test]
fn apply_rejects_a_selected_action_id_that_is_not_in_the_plan() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset],
        platforms: vec![PlatformId::Cursor],
    };
    fs::create_dir_all(request.deploy_base.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    let result = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::with_selected_action_ids(&plan, ["unknown-action".to_owned()]),
    );

    assert!(result.is_err());
}

#[cfg(unix)]
#[test]
fn selected_equivalent_adoption_moves_the_old_target_to_backup_before_linking() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset.clone()],
        platforms: vec![PlatformId::Cursor],
    };
    let target = request.deploy_base.join(".agents/skills/review");
    fs::create_dir_all(target.as_std_path()).unwrap();
    fs::write(target.join("SKILL.md").as_std_path(), "canonical skill").unwrap();
    let backup_root = root.join("backups");
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    assert_eq!(
        plan.actions[0].kind,
        ai_config_core::projection::planner::ProjectionActionKind::AdoptEquivalent
    );

    let report = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), backup_root.clone()),
        ApplyOptions::with_selected_action_ids(&plan, [plan.action_ids[0].clone()]),
    )
    .unwrap();

    assert_eq!(report.changed, 1);
    let transaction_id = report
        .transaction_id
        .as_deref()
        .expect("the public single-plan API must not bypass durable adoption");
    assert!(
        backup_root.join(format!("{transaction_id}.json")).is_file(),
        "selected adoption must always leave one durable transaction manifest"
    );
    assert_eq!(
        fs::read_link(target.as_std_path()).unwrap(),
        asset.source_path
    );
    assert!(backup_root.exists());
    assert!(backup_root.read_dir().unwrap().next().is_some());
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(backup_root.as_std_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

#[cfg(unix)]
#[test]
fn selected_equivalent_adoption_rejects_a_symlinked_backup_root_without_writing_outside() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset],
        platforms: vec![PlatformId::Cursor],
    };
    let target = request.deploy_base.join(".agents/skills/review");
    fs::create_dir_all(target.as_std_path()).unwrap();
    fs::write(target.join("SKILL.md").as_std_path(), "canonical skill").unwrap();
    let outside = root.join("outside");
    fs::create_dir_all(outside.as_std_path()).unwrap();
    fs::write(outside.join("sentinel").as_std_path(), "outside data").unwrap();
    let backup_root = root.join("backups");
    std::os::unix::fs::symlink(outside.as_std_path(), backup_root.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    let result = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), backup_root),
        ApplyOptions::with_selected_action_ids(&plan, [plan.action_ids[0].clone()]),
    );

    assert_eq!(
        fs::read_dir(outside.as_std_path())
            .unwrap()
            .map(Result::unwrap)
            .map(|entry| entry.file_name())
            .collect::<Vec<_>>(),
        vec![std::ffi::OsString::from("sentinel")]
    );
    assert!(fs::symlink_metadata(target.as_std_path())
        .unwrap()
        .file_type()
        .is_dir());
    assert_eq!(
        fs::read_to_string(target.join("SKILL.md").as_std_path()).unwrap(),
        "canonical skill"
    );
    assert!(result.is_err());
}

#[cfg(unix)]
#[test]
fn selected_equivalent_adoption_ignores_preexisting_legacy_backup_slots() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let backup_root = root.join("backups");
    fs::create_dir_all(backup_root.as_std_path()).unwrap();
    let first_deploy_base = root.join("first-deploy");
    let first_target = first_deploy_base.join(".agents/skills/review");
    fs::create_dir_all(first_target.as_std_path()).unwrap();
    fs::write(
        first_target.join("SKILL.md").as_std_path(),
        "canonical skill",
    )
    .unwrap();
    let ledger = MemoryProjectionLedger::default();
    let first_request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/first".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: first_deploy_base.clone(),
        assets: vec![asset.clone()],
        platforms: vec![PlatformId::Cursor],
    };
    let first_plan = build_projection_plan(&first_request, &PlannerContext::new(&ledger)).unwrap();
    apply_projection_plan(
        &first_plan,
        &ExecutorContext::new(&ledger, first_deploy_base, backup_root.clone()),
        ApplyOptions::with_selected_action_ids(&first_plan, [first_plan.action_ids[0].clone()]),
    )
    .unwrap();

    let mut preserved = Vec::new();
    for sequence in 0..1_024 {
        let backup = backup_root.join(format!("{sequence}-review"));
        let manifest = backup.with_extension("manifest.json");
        let backup_contents = format!("backup-{sequence}");
        let manifest_contents = format!("manifest-{sequence}");
        fs::write(backup.as_std_path(), &backup_contents).unwrap();
        fs::write(manifest.as_std_path(), &manifest_contents).unwrap();
        preserved.push((backup, backup_contents, manifest, manifest_contents));
    }

    let second_deploy_base = root.join("second-deploy");
    let second_target = second_deploy_base.join(".agents/skills/review");
    fs::create_dir_all(second_target.as_std_path()).unwrap();
    fs::write(
        second_target.join("SKILL.md").as_std_path(),
        "canonical skill",
    )
    .unwrap();
    let second_request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/second".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: second_deploy_base.clone(),
        assets: vec![asset],
        platforms: vec![PlatformId::Cursor],
    };
    let second_plan =
        build_projection_plan(&second_request, &PlannerContext::new(&ledger)).unwrap();
    let result = apply_projection_plan(
        &second_plan,
        &ExecutorContext::new(&ledger, second_deploy_base, backup_root),
        ApplyOptions::with_selected_action_ids(&second_plan, [second_plan.action_ids[0].clone()]),
    );

    for (backup, backup_contents, manifest, manifest_contents) in preserved {
        assert_eq!(
            fs::read_to_string(backup.as_std_path()).unwrap(),
            backup_contents
        );
        assert_eq!(
            fs::read_to_string(manifest.as_std_path()).unwrap(),
            manifest_contents
        );
    }
    result.unwrap();
    assert!(fs::read_link(second_target.as_std_path()).is_ok());
}

#[cfg(unix)]
#[test]
fn selected_equivalent_adoption_ignores_unrelated_legacy_manifest_temp_symlinks() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let backup_root = root.join("backups");
    fs::create_dir_all(backup_root.as_std_path()).unwrap();
    let outside = root.join("outside-sentinel");
    fs::write(outside.as_std_path(), "outside data").unwrap();
    for sequence in 0..1_024 {
        let temporary = backup_root
            .join(format!("{sequence}-review"))
            .with_extension("manifest.tmp");
        std::os::unix::fs::symlink(outside.as_std_path(), temporary.as_std_path()).unwrap();
    }
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset],
        platforms: vec![PlatformId::Cursor],
    };
    let target = request.deploy_base.join(".agents/skills/review");
    fs::create_dir_all(target.as_std_path()).unwrap();
    fs::write(target.join("SKILL.md").as_std_path(), "canonical skill").unwrap();
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    let result = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base, backup_root),
        ApplyOptions::with_selected_action_ids(&plan, [plan.action_ids[0].clone()]),
    );

    assert_eq!(
        fs::read_to_string(outside.as_std_path()).unwrap(),
        "outside data"
    );
    result.unwrap();
    assert!(fs::read_link(target.as_std_path()).is_ok());
}

#[cfg(unix)]
#[test]
fn ledger_failure_restores_an_adopted_equivalent_target() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset],
        platforms: vec![PlatformId::Cursor],
    };
    let target = request.deploy_base.join(".agents/skills/review");
    fs::create_dir_all(target.as_std_path()).unwrap();
    fs::write(target.join("SKILL.md").as_std_path(), "canonical skill").unwrap();
    let ledger = FailingLedger;
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    let result = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::with_selected_action_ids(&plan, [plan.action_ids[0].clone()]),
    );

    assert!(result.is_err());
    assert!(fs::symlink_metadata(target.as_std_path()).unwrap().is_dir());
    assert_eq!(
        fs::read_to_string(target.join("SKILL.md").as_std_path()).unwrap(),
        "canonical skill"
    );
}

#[test]
fn copy_fallback_requires_policy_authorization_and_unavailable_links_before_planning() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset],
        platforms: vec![PlatformId::Cursor],
    };
    fs::create_dir_all(request.deploy_base.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();

    let default_plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    assert_eq!(
        default_plan.actions[0].kind,
        ProjectionActionKind::CreateLink
    );

    let missing_authorization = PlannerContext::with_copy_fallback(
        &ledger,
        LinkAvailability::Unavailable,
        CopyFallbackPolicy::windows_explicit_for([PlatformId::Cursor]),
        CopyFallbackAuthorization::Denied,
    );
    let denied_plan = build_projection_plan(&request, &missing_authorization).unwrap();
    assert_eq!(
        denied_plan.actions[0].kind,
        ProjectionActionKind::CreateLink
    );

    let available_link = PlannerContext::with_copy_fallback(
        &ledger,
        LinkAvailability::Available,
        CopyFallbackPolicy::windows_explicit_for([PlatformId::Cursor]),
        CopyFallbackAuthorization::Granted,
    );
    let available_plan = build_projection_plan(&request, &available_link).unwrap();
    assert_eq!(
        available_plan.actions[0].kind,
        ProjectionActionKind::CreateLink
    );

    let explicit = PlannerContext::with_copy_fallback(
        &ledger,
        LinkAvailability::Unavailable,
        CopyFallbackPolicy::windows_explicit_for([PlatformId::Cursor]),
        CopyFallbackAuthorization::Granted,
    );
    let copy_plan = build_projection_plan(&request, &explicit).unwrap();
    assert_eq!(
        copy_plan.actions[0].kind,
        ProjectionActionKind::CopyFallback
    );
    assert_eq!(copy_plan.actions[0].state.as_deref(), Some("copied"));
}

#[test]
fn explicit_copy_fallback_gates_do_not_upgrade_a_direct_target_without_adapter_permission() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = rule(root, "review");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset],
        platforms: vec![PlatformId::Cursor],
    };
    fs::create_dir_all(request.deploy_base.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let fallback = PlannerContext::with_copy_fallback(
        &ledger,
        LinkAvailability::Unavailable,
        CopyFallbackPolicy::windows_explicit_for([PlatformId::Cursor]),
        CopyFallbackAuthorization::Granted,
    );

    let plan = build_projection_plan(&request, &fallback).unwrap();

    assert_eq!(plan.actions[0].kind, ProjectionActionKind::CreateLink);
}

#[test]
fn explicit_copy_fallback_copies_and_only_retracts_a_ledger_proven_unchanged_target() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    fs::write(
        asset.source_path.join("references.md").as_std_path(),
        "copy this too",
    )
    .unwrap();
    let asset = EffectiveAsset {
        fingerprint: path_content_digest(&asset.source_path).unwrap(),
        ..asset
    };
    let sync_request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset.clone()],
        platforms: vec![PlatformId::Cursor],
    };
    fs::create_dir_all(sync_request.deploy_base.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let fallback = PlannerContext::with_copy_fallback(
        &ledger,
        LinkAvailability::Unavailable,
        CopyFallbackPolicy::windows_explicit_for([PlatformId::Cursor]),
        CopyFallbackAuthorization::Granted,
    );
    let sync_plan = build_projection_plan(&sync_request, &fallback).unwrap();
    assert_eq!(
        sync_plan.actions[0].kind,
        ProjectionActionKind::CopyFallback
    );

    apply_projection_plan(
        &sync_plan,
        &ExecutorContext::new(
            &ledger,
            sync_request.deploy_base.clone(),
            root.join("backups"),
        ),
        ApplyOptions::for_plan(&sync_plan),
    )
    .unwrap();

    let target = sync_request.deploy_base.join(".agents/skills/review");
    assert!(fs::symlink_metadata(target.as_std_path()).unwrap().is_dir());
    assert!(!fs::symlink_metadata(target.as_std_path())
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        fs::read_to_string(target.join("references.md").as_std_path()).unwrap(),
        "copy this too"
    );
    let id = sync_plan.actions[0].members[0].id.clone();
    assert_eq!(
        ledger.get(&id).unwrap().unwrap().mode,
        ProjectionMode::CopyFallback
    );

    let retract_request = ProjectionRequest {
        operation: ProjectionOperation::Retract,
        ..sync_request
    };
    let retract_plan =
        build_projection_plan(&retract_request, &PlannerContext::new(&ledger)).unwrap();
    assert_eq!(
        retract_plan.actions[0].kind,
        ProjectionActionKind::RemoveManagedCopy
    );
    apply_projection_plan(
        &retract_plan,
        &ExecutorContext::new(
            &ledger,
            retract_request.deploy_base.clone(),
            root.join("backups"),
        ),
        ApplyOptions::for_plan(&retract_plan),
    )
    .unwrap();
    assert!(fs::symlink_metadata(target.as_std_path()).is_err());
    assert_eq!(ledger.get(&id).unwrap(), None);
}

#[test]
fn copy_fallback_drift_is_report_only_and_never_deleted_on_retract() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let sync_request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset.clone()],
        platforms: vec![PlatformId::Cursor],
    };
    fs::create_dir_all(sync_request.deploy_base.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let fallback = PlannerContext::with_copy_fallback(
        &ledger,
        LinkAvailability::Unavailable,
        CopyFallbackPolicy::windows_explicit_for([PlatformId::Cursor]),
        CopyFallbackAuthorization::Granted,
    );
    let sync_plan = build_projection_plan(&sync_request, &fallback).unwrap();
    apply_projection_plan(
        &sync_plan,
        &ExecutorContext::new(
            &ledger,
            sync_request.deploy_base.clone(),
            root.join("backups"),
        ),
        ApplyOptions::for_plan(&sync_plan),
    )
    .unwrap();
    let target = sync_request.deploy_base.join(".agents/skills/review");
    fs::write(target.join("user-edit.md").as_std_path(), "do not delete").unwrap();

    let retract_request = ProjectionRequest {
        operation: ProjectionOperation::Retract,
        ..sync_request
    };
    let retract_plan =
        build_projection_plan(&retract_request, &PlannerContext::new(&ledger)).unwrap();
    assert_eq!(
        retract_plan.actions[0].kind,
        ProjectionActionKind::ReportOnly
    );
    assert_eq!(retract_plan.actions[0].state.as_deref(), Some("drifted"));
    assert!(apply_projection_plan(
        &retract_plan,
        &ExecutorContext::new(
            &ledger,
            retract_request.deploy_base.clone(),
            root.join("backups"),
        ),
        ApplyOptions::for_plan(&retract_plan),
    )
    .is_err());
    assert_eq!(
        fs::read_to_string(target.join("user-edit.md").as_std_path()).unwrap(),
        "do not delete"
    );
}

#[cfg(unix)]
#[test]
fn copy_fallback_rejects_a_source_tree_containing_a_symlink_without_creating_the_target() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let outside = root.join("outside.txt");
    fs::write(outside.as_std_path(), "must not be followed").unwrap();
    std::os::unix::fs::symlink(
        outside.as_std_path(),
        asset.source_path.join("outside-link").as_std_path(),
    )
    .unwrap();
    let asset = EffectiveAsset {
        fingerprint: path_content_digest(&asset.source_path).unwrap(),
        ..asset
    };
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset],
        platforms: vec![PlatformId::Cursor],
    };
    fs::create_dir_all(request.deploy_base.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let fallback = PlannerContext::with_copy_fallback(
        &ledger,
        LinkAvailability::Unavailable,
        CopyFallbackPolicy::windows_explicit_for([PlatformId::Cursor]),
        CopyFallbackAuthorization::Granted,
    );
    let plan = build_projection_plan(&request, &fallback).unwrap();
    let target = request.deploy_base.join(".agents/skills/review");

    let result = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    );

    assert!(result.is_err());
    assert!(fs::symlink_metadata(target.as_std_path()).is_err());
    assert_eq!(ledger.get(&plan.actions[0].members[0].id).unwrap(), None);
}

#[test]
fn ledger_failure_rolls_back_a_new_copy_fallback_without_a_partial_record() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset],
        platforms: vec![PlatformId::Cursor],
    };
    fs::create_dir_all(request.deploy_base.as_std_path()).unwrap();
    let ledger = FailingLedger;
    let fallback = PlannerContext::with_copy_fallback(
        &ledger,
        LinkAvailability::Unavailable,
        CopyFallbackPolicy::windows_explicit_for([PlatformId::Cursor]),
        CopyFallbackAuthorization::Granted,
    );
    let plan = build_projection_plan(&request, &fallback).unwrap();
    let target = request.deploy_base.join(".agents/skills/review");

    let failure = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    )
    .unwrap_err();

    assert_eq!(failure.report.rolled_back, 1);
    assert!(fs::symlink_metadata(target.as_std_path()).is_err());
    assert_eq!(ledger.get(&plan.actions[0].members[0].id).unwrap(), None);
}

#[test]
fn ledger_failure_restores_the_old_copied_target_during_a_copy_refresh() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = skill(root, "review");
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![asset.clone()],
        platforms: vec![PlatformId::Cursor],
    };
    fs::create_dir_all(request.deploy_base.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let fallback = PlannerContext::with_copy_fallback(
        &ledger,
        LinkAvailability::Unavailable,
        CopyFallbackPolicy::windows_explicit_for([PlatformId::Cursor]),
        CopyFallbackAuthorization::Granted,
    );
    let initial_plan = build_projection_plan(&request, &fallback).unwrap();
    apply_projection_plan(
        &initial_plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&initial_plan),
    )
    .unwrap();
    let id = initial_plan.actions[0].members[0].id.clone();
    let initial_record = ledger.get(&id).unwrap().unwrap();
    let target = request.deploy_base.join(".agents/skills/review/SKILL.md");
    assert_eq!(
        fs::read_to_string(target.as_std_path()).unwrap(),
        "canonical skill"
    );

    fs::write(
        asset.source_path.join("SKILL.md").as_std_path(),
        "new source body",
    )
    .unwrap();
    let refreshed_asset = EffectiveAsset {
        fingerprint: path_content_digest(&asset.source_path).unwrap(),
        ..asset
    };
    let refresh_request = ProjectionRequest {
        assets: vec![refreshed_asset],
        ..request
    };
    let failing_ledger = FailingLedgerWithRecord {
        record: initial_record,
    };
    let refresh_fallback = PlannerContext::with_copy_fallback(
        &failing_ledger,
        LinkAvailability::Unavailable,
        CopyFallbackPolicy::windows_explicit_for([PlatformId::Cursor]),
        CopyFallbackAuthorization::Granted,
    );
    let refresh_plan = build_projection_plan(&refresh_request, &refresh_fallback).unwrap();
    assert_eq!(
        refresh_plan.actions[0].kind,
        ProjectionActionKind::CopyFallback
    );

    let failure = apply_projection_plan(
        &refresh_plan,
        &ExecutorContext::new(
            &failing_ledger,
            refresh_request.deploy_base.clone(),
            root.join("backups"),
        ),
        ApplyOptions::for_plan(&refresh_plan),
    )
    .unwrap_err();

    assert_eq!(failure.report.rolled_back, 1);
    assert_eq!(
        fs::read_to_string(target.as_std_path()).unwrap(),
        "canonical skill"
    );
}

// T010.6 — an explicitly selected equivalent adoption is a user-visible migration, not an
// ephemeral executor detail.  The public transaction ID lets CLI/MCP expose one reviewed
// rollback handle for the whole selected slice.
#[cfg(unix)]
fn two_equivalent_adoption_request(root: &Utf8Path) -> ProjectionRequest {
    let review = skill(root, "review");
    let audit = skill(root, "audit");
    let deploy_base = root.join("deploy");
    for name in ["review", "audit"] {
        let target = deploy_base.join(".agents/skills").join(name);
        fs::create_dir_all(target.as_std_path()).unwrap();
        fs::write(target.join("SKILL.md").as_std_path(), "canonical skill").unwrap();
    }
    ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/durable-adoption".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base,
        assets: vec![review, audit],
        platforms: vec![PlatformId::Cursor],
    }
}

#[cfg(unix)]
fn durable_adoption_targets(request: &ProjectionRequest) -> [camino::Utf8PathBuf; 2] {
    [
        request.deploy_base.join(".agents/skills/review"),
        request.deploy_base.join(".agents/skills/audit"),
    ]
}

#[cfg(unix)]
fn assert_no_durable_transaction_is_reported_as_applied(root: &Utf8Path) {
    fn visit(path: &Utf8Path, manifests: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(path.as_std_path()) else {
            return;
        };
        for entry in entries.map(Result::unwrap) {
            let path = camino::Utf8PathBuf::from_path_buf(entry.path()).unwrap();
            let metadata = fs::symlink_metadata(path.as_std_path()).unwrap();
            if metadata.is_dir() {
                visit(&path, manifests);
            } else if path.extension() == Some("json") {
                manifests.push(fs::read_to_string(path.as_std_path()).unwrap());
            }
        }
    }

    let mut manifests = Vec::new();
    visit(root, &mut manifests);
    assert!(manifests.into_iter().all(|manifest| {
        !manifest
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>()
            .contains("\"status\":\"applied\"")
    }));
}

#[cfg(unix)]
fn first_backup_skill_file(backup_root: &Utf8Path) -> camino::Utf8PathBuf {
    fn visit(path: &Utf8Path) -> Option<camino::Utf8PathBuf> {
        let entries = fs::read_dir(path.as_std_path()).ok()?;
        for entry in entries.flatten() {
            let path = camino::Utf8PathBuf::from_path_buf(entry.path()).ok()?;
            let metadata = fs::symlink_metadata(path.as_std_path()).ok()?;
            if metadata.is_dir() {
                if let Some(file) = visit(&path) {
                    return Some(file);
                }
            } else if path.file_name() == Some("SKILL.md") {
                return Some(path);
            }
        }
        None
    }

    visit(backup_root).expect("durable adoption must retain a private old-target backup")
}

#[cfg(unix)]
#[test]
fn selected_equivalent_adoptions_share_one_durable_transaction_manifest_and_id() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = two_equivalent_adoption_request(root);
    let backup_root = root.join("backups");
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    assert_eq!(plan.actions.len(), 2);
    assert!(plan
        .actions
        .iter()
        .all(|action| action.kind == ProjectionActionKind::AdoptEquivalent));

    let reports = apply_projection_plans_transactionally(
        [(
            &plan,
            ApplyOptions::with_selected_action_ids(&plan, plan.action_ids.clone()),
        )],
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), backup_root.clone()),
    )
    .unwrap();

    let transaction_id = reports[0]
        .transaction_id
        .clone()
        .expect("a successful selected adoption must expose a durable transaction ID");
    assert!(!transaction_id.is_empty());
    let manifest = backup_root.join(format!("{transaction_id}.json"));
    let manifest = fs::read_to_string(manifest.as_std_path())
        .expect("all selected adoption actions must share one durable manifest");
    assert!(manifest.contains(".agents/skills/review"));
    assert!(manifest.contains(".agents/skills/audit"));
    for target in durable_adoption_targets(&request) {
        assert!(fs::symlink_metadata(target.as_std_path())
            .unwrap()
            .file_type()
            .is_symlink());
    }
}

#[cfg(unix)]
#[test]
fn durable_adoption_rollback_restores_every_original_target_and_revokes_ownership() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = two_equivalent_adoption_request(root);
    let backup_root = root.join("backups");
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    let context = ExecutorContext::new(&ledger, request.deploy_base.clone(), backup_root.clone());
    let reports = apply_projection_plans_transactionally(
        [(
            &plan,
            ApplyOptions::with_selected_action_ids(&plan, plan.action_ids.clone()),
        )],
        &context,
    )
    .unwrap();
    let transaction_id = reports[0].transaction_id.clone().unwrap();

    let rollback = rollback_projection_transaction(&context, &transaction_id).unwrap();

    assert_eq!(rollback.restored, 2);
    for target in durable_adoption_targets(&request) {
        assert!(fs::symlink_metadata(target.as_std_path()).unwrap().is_dir());
        assert_eq!(
            fs::read_to_string(target.join("SKILL.md").as_std_path()).unwrap(),
            "canonical skill"
        );
    }
    for action in &plan.actions {
        assert_eq!(ledger.get(&action.members[0].id).unwrap(), None);
    }
    let manifest = fs::read_to_string(
        backup_root
            .join(format!("{transaction_id}.json"))
            .as_std_path(),
    )
    .unwrap();
    assert!(manifest.contains("\"status\": \"rolled_back\""));
}

#[cfg(unix)]
#[test]
fn durable_adoption_rollback_recovers_a_commit_before_the_applied_status_was_persisted() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = two_equivalent_adoption_request(root);
    let backup_root = root.join("backups");
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    let context = ExecutorContext::new(&ledger, request.deploy_base.clone(), backup_root.clone());
    let reports = apply_projection_plans_transactionally(
        [(
            &plan,
            ApplyOptions::with_selected_action_ids(&plan, plan.action_ids.clone()),
        )],
        &context,
    )
    .unwrap();
    let transaction_id = reports[0].transaction_id.clone().unwrap();
    let manifest_path = backup_root.join(format!("{transaction_id}.json"));
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(manifest_path.as_std_path()).unwrap()).unwrap();
    manifest["status"] = serde_json::Value::String("applying".to_owned());
    fs::write(
        manifest_path.as_std_path(),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let rollback = rollback_projection_transaction(&context, &transaction_id).unwrap();

    assert_eq!(rollback.restored, 2);
    for target in durable_adoption_targets(&request) {
        assert!(fs::symlink_metadata(target.as_std_path()).unwrap().is_dir());
    }
    for action in &plan.actions {
        assert_eq!(ledger.get(&action.members[0].id).unwrap(), None);
    }
}

#[cfg(unix)]
#[test]
fn durable_adoption_rollback_recovers_applying_manifest_before_final_fingerprint_is_persisted() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = two_equivalent_adoption_request(root);
    let backup_root = root.join("backups");
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    let context = ExecutorContext::new(&ledger, request.deploy_base.clone(), backup_root.clone());
    let reports = apply_projection_plans_transactionally(
        [(
            &plan,
            ApplyOptions::with_selected_action_ids(&plan, plan.action_ids.clone()),
        )],
        &context,
    )
    .unwrap();
    let transaction_id = reports[0].transaction_id.clone().unwrap();
    let manifest_path = backup_root.join(format!("{transaction_id}.json"));
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(manifest_path.as_std_path()).unwrap()).unwrap();
    manifest["status"] = serde_json::Value::String("applying".to_owned());
    for action in manifest["actions"].as_array_mut().unwrap() {
        action["after"] = serde_json::Value::Null;
        action["backup_path"] = serde_json::Value::Null;
        action["expected_record"] = serde_json::Value::Null;
    }
    fs::write(
        manifest_path.as_std_path(),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let rollback = rollback_projection_transaction(&context, &transaction_id).unwrap();

    assert_eq!(rollback.restored, 2);
    for target in durable_adoption_targets(&request) {
        assert!(fs::symlink_metadata(target.as_std_path()).unwrap().is_dir());
        assert_eq!(
            fs::read_to_string(target.join("SKILL.md").as_std_path()).unwrap(),
            "canonical skill"
        );
    }
}

#[cfg(unix)]
#[test]
fn durable_adoption_rollback_refuses_a_symlinked_target_parent_without_writing_outside() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = two_equivalent_adoption_request(root);
    let backup_root = root.join("backups");
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    let context = ExecutorContext::new(&ledger, request.deploy_base.clone(), backup_root);
    let reports = apply_projection_plans_transactionally(
        [(
            &plan,
            ApplyOptions::with_selected_action_ids(&plan, plan.action_ids.clone()),
        )],
        &context,
    )
    .unwrap();
    let transaction_id = reports[0].transaction_id.clone().unwrap();
    let agents_parent = request.deploy_base.join(".agents");
    let original_parent = request.deploy_base.join(".agents-after-adoption");
    fs::rename(agents_parent.as_std_path(), original_parent.as_std_path()).unwrap();
    let outside = root.join("outside");
    fs::create_dir_all(outside.join("skills").as_std_path()).unwrap();
    for action in &plan.actions {
        let name = action.members[0].id.name.as_str();
        std::os::unix::fs::symlink(
            action.members[0].source.absolute_path.as_std_path(),
            outside.join("skills").join(name).as_std_path(),
        )
        .unwrap();
    }
    std::os::unix::fs::symlink(outside.as_std_path(), agents_parent.as_std_path()).unwrap();
    let before = ["review", "audit"]
        .map(|name| fs::read_link(outside.join("skills").join(name).as_std_path()).unwrap());

    assert!(rollback_projection_transaction(&context, &transaction_id).is_err());
    assert_eq!(
        ["review", "audit"].map(|name| {
            fs::read_link(outside.join("skills").join(name).as_std_path()).unwrap()
        }),
        before,
        "a symlinked target ancestor must make rollback fail closed before any write"
    );
}

#[cfg(unix)]
#[test]
fn selected_equivalent_adoption_ledger_batch_failure_restores_every_target_and_never_leaves_applied_transaction(
) {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = two_equivalent_adoption_request(root);
    let backup_root = root.join("backups");
    let ledger = FailingLedger;
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    let failure = apply_projection_plans_transactionally(
        [(
            &plan,
            ApplyOptions::with_selected_action_ids(&plan, plan.action_ids.clone()),
        )],
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), backup_root.clone()),
    )
    .unwrap_err();

    assert_eq!(failure.reports.len(), 1);
    assert_eq!(failure.reports[0].rolled_back, 2);
    for target in durable_adoption_targets(&request) {
        assert!(fs::symlink_metadata(target.as_std_path()).unwrap().is_dir());
        assert_eq!(
            fs::read_to_string(target.join("SKILL.md").as_std_path()).unwrap(),
            "canonical skill"
        );
    }
    assert_no_durable_transaction_is_reported_as_applied(&backup_root);
}

#[cfg(unix)]
#[test]
fn durable_adoption_rollback_refuses_target_drift_without_touching_any_member() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = two_equivalent_adoption_request(root);
    let backup_root = root.join("backups");
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    let context = ExecutorContext::new(&ledger, request.deploy_base.clone(), backup_root);
    let reports = apply_projection_plans_transactionally(
        [(
            &plan,
            ApplyOptions::with_selected_action_ids(&plan, plan.action_ids.clone()),
        )],
        &context,
    )
    .unwrap();
    let transaction_id = reports[0].transaction_id.clone().unwrap();
    let [review, audit] = durable_adoption_targets(&request);
    fs::remove_file(review.as_std_path()).unwrap();
    fs::write(review.as_std_path(), "user drift").unwrap();

    assert!(rollback_projection_transaction(&context, &transaction_id).is_err());
    assert_eq!(
        fs::read_to_string(review.as_std_path()).unwrap(),
        "user drift"
    );
    assert!(fs::symlink_metadata(audit.as_std_path())
        .unwrap()
        .file_type()
        .is_symlink());
}

#[cfg(unix)]
#[test]
fn durable_adoption_rollback_refuses_backup_drift_without_touching_any_member() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = two_equivalent_adoption_request(root);
    let backup_root = root.join("backups");
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    let context = ExecutorContext::new(&ledger, request.deploy_base.clone(), backup_root.clone());
    let reports = apply_projection_plans_transactionally(
        [(
            &plan,
            ApplyOptions::with_selected_action_ids(&plan, plan.action_ids.clone()),
        )],
        &context,
    )
    .unwrap();
    let transaction_id = reports[0].transaction_id.clone().unwrap();
    let backup = first_backup_skill_file(&backup_root);
    fs::write(backup.as_std_path(), "backup drift").unwrap();
    let targets = durable_adoption_targets(&request);

    assert!(rollback_projection_transaction(&context, &transaction_id).is_err());
    assert_eq!(
        fs::read_to_string(backup.as_std_path()).unwrap(),
        "backup drift"
    );
    for target in targets {
        assert!(fs::symlink_metadata(target.as_std_path())
            .unwrap()
            .file_type()
            .is_symlink());
    }
}

#[cfg(unix)]
#[test]
fn durable_adoption_rollback_refuses_a_symlinked_backup_without_touching_any_member() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = two_equivalent_adoption_request(root);
    let backup_root = root.join("backups");
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    let context = ExecutorContext::new(&ledger, request.deploy_base.clone(), backup_root.clone());
    let reports = apply_projection_plans_transactionally(
        [(
            &plan,
            ApplyOptions::with_selected_action_ids(&plan, plan.action_ids.clone()),
        )],
        &context,
    )
    .unwrap();
    let transaction_id = reports[0].transaction_id.clone().unwrap();
    let backup_file = first_backup_skill_file(&backup_root);
    let backup_payload = backup_file.parent().unwrap().to_owned();
    let outside = root.join("outside-backup");
    fs::rename(backup_payload.as_std_path(), outside.as_std_path()).unwrap();
    std::os::unix::fs::symlink(outside.as_std_path(), backup_payload.as_std_path()).unwrap();
    let targets = durable_adoption_targets(&request);

    assert!(rollback_projection_transaction(&context, &transaction_id).is_err());
    assert_eq!(
        fs::read_to_string(outside.join("SKILL.md").as_std_path()).unwrap(),
        "canonical skill"
    );
    for target in targets {
        assert!(fs::symlink_metadata(target.as_std_path())
            .unwrap()
            .file_type()
            .is_symlink());
    }
}

#[cfg(unix)]
#[test]
fn durable_adoption_rollback_refuses_ledger_drift_without_touching_any_member() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = two_equivalent_adoption_request(root);
    let backup_root = root.join("backups");
    let ledger = MemoryProjectionLedger::default();
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    let context = ExecutorContext::new(&ledger, request.deploy_base.clone(), backup_root);
    let reports = apply_projection_plans_transactionally(
        [(
            &plan,
            ApplyOptions::with_selected_action_ids(&plan, plan.action_ids.clone()),
        )],
        &context,
    )
    .unwrap();
    let transaction_id = reports[0].transaction_id.clone().unwrap();
    let drifted_id = plan.actions[0].members[0].id.clone();
    let original = ledger.get(&drifted_id).unwrap().unwrap();
    let drifted = ProjectionRecord {
        target_fingerprint: "user-changed-ledger".to_owned(),
        ..original
    };
    ledger
        .apply_batch(&[LedgerMutation::Upsert(drifted.clone())])
        .unwrap();
    let targets = durable_adoption_targets(&request);

    assert!(rollback_projection_transaction(&context, &transaction_id).is_err());
    assert_eq!(ledger.get(&drifted_id).unwrap(), Some(drifted));
    for target in targets {
        assert!(fs::symlink_metadata(target.as_std_path())
            .unwrap()
            .file_type()
            .is_symlink());
    }
}
