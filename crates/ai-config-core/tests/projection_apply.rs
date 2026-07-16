use std::fs;

use ai_config_core::error::CoreError;
use ai_config_core::model::{AssetKind, PlatformId};
use ai_config_core::projection::executor::{apply_projection_plan, ApplyOptions, ExecutorContext};
use ai_config_core::projection::fingerprint::path_content_digest;
use ai_config_core::projection::ledger::MemoryProjectionLedger;
use ai_config_core::projection::ledger::ProjectionLedger;
use ai_config_core::projection::model::{
    DeploymentScope, EffectiveAsset, LedgerMutation, ProjectionId, ProjectionMode,
    ProjectionRecord, SourceLayer,
};
use ai_config_core::projection::planner::{
    build_projection_plan, PlannerContext, ProjectionOperation, ProjectionRequest,
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

    assert!(result.is_err());
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
