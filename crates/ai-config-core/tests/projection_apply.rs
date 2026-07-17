use std::fs;

use ai_config_core::error::CoreError;
use ai_config_core::model::{AssetKind, PlatformId};
use ai_config_core::projection::executor::{
    apply_projection_plan, apply_projection_plans_transactionally, ApplyActionStatus, ApplyOptions,
    ExecutorContext,
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
            .join(".ai-config-projection.lock")
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
    assert_eq!(
        fs::read_link(target.as_std_path()).unwrap(),
        asset.source_path
    );
    assert!(backup_root.exists());
    assert!(backup_root.read_dir().unwrap().next().is_some());
    let manifest = fs::read_dir(backup_root.as_std_path())
        .unwrap()
        .map(Result::unwrap)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .ends_with(".manifest.json")
        })
        .expect("adoption must leave a backup manifest");
    let manifest = fs::read_to_string(manifest.path()).unwrap();
    assert!(manifest.contains(".agents/skills/review"));
    assert!(!manifest.contains("canonical skill"));
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
