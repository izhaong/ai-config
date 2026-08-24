use std::fs;

use ai_config_core::hook_adapter;
use ai_config_core::model::{AssetKind, PlatformId};
use ai_config_core::projection::executor::{apply_projection_plan, ApplyOptions, ExecutorContext};
use ai_config_core::projection::fingerprint::path_content_digest;
use ai_config_core::projection::ledger::{MemoryProjectionLedger, ProjectionLedger};
use ai_config_core::projection::model::{
    DeploymentScope, EffectiveAsset, LedgerMutation, ProjectionMode, ProjectionRecord, SourceLayer,
};
use ai_config_core::projection::planner::{
    build_projection_plan, PlannerContext, ProjectionActionKind, ProjectionOperation,
    ProjectionRequest,
};
use ai_config_core::projection::source::{resolve_effective_assets, OverlayRoots};
use ai_config_core::source::scan_project_root;
use camino::Utf8Path;
use chrono::Utc;
use tempfile::TempDir;

fn hook_asset(root: &Utf8Path, name: &str) -> EffectiveAsset {
    let source_path = root.join("source/hooks").join(name);
    fs::create_dir_all(source_path.parent().unwrap().as_std_path()).unwrap();
    fs::write(source_path.as_std_path(), "#!/bin/sh\necho canonical\n").unwrap();
    EffectiveAsset {
        kind: AssetKind::Hook,
        name: name.to_owned(),
        source_path: source_path.clone(),
        layer: SourceLayer::Project,
        fingerprint: path_content_digest(&source_path).unwrap(),
    }
}

fn hook_bundle(root: &Utf8Path, name: &str) -> EffectiveAsset {
    let source_path = root.join("source/hooks").join(name);
    fs::create_dir_all(source_path.join("scripts").as_std_path()).unwrap();
    fs::write(
        source_path.join("hook.yaml").as_std_path(),
        "entry: scripts/run.sh\n",
    )
    .unwrap();
    fs::write(
        source_path.join("scripts/run.sh").as_std_path(),
        "#!/bin/sh\necho bundle\n",
    )
    .unwrap();
    EffectiveAsset {
        kind: AssetKind::Hook,
        name: name.to_owned(),
        source_path: source_path.clone(),
        layer: SourceLayer::Project,
        fingerprint: path_content_digest(&source_path).unwrap(),
    }
}

fn request(root: &Utf8Path, hook: EffectiveAsset, platform: PlatformId) -> ProjectionRequest {
    ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/hook-fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: vec![hook],
        platforms: vec![platform],
    }
}

fn apply(
    root: &Utf8Path,
    request: &ProjectionRequest,
    ledger: &MemoryProjectionLedger,
) -> Result<ai_config_core::projection::executor::ApplyReport, String> {
    let plan = build_projection_plan(request, &PlannerContext::new(ledger))
        .map_err(|error| error.to_string())?;
    apply_projection_plan(
        &plan,
        &ExecutorContext::new(ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    )
    .map_err(|error| error.to_string())
}

#[test]
fn hook_scan_is_read_only_even_with_an_unregistered_script() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset_root = root.join("assets");
    fs::create_dir_all(asset_root.join("hooks").as_std_path()).unwrap();
    fs::write(
        asset_root.join("hooks/orphan.sh").as_std_path(),
        "#!/bin/sh\n",
    )
    .unwrap();
    let before = path_content_digest(&asset_root).unwrap();

    let result = scan_project_root(&asset_root).unwrap();

    assert_eq!(result.hooks.len(), 1, "scan may inventory legacy scripts");
    assert_eq!(path_content_digest(&asset_root).unwrap(), before);
    assert!(
        !asset_root.join("hooks.json").exists(),
        "scan must not reconcile or create a manifest"
    );
}

#[test]
fn hook_apply_preserves_foreign_cursor_binding_and_unknown_fields() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = request(root, hook_asset(root, "format.sh"), PlatformId::Cursor);
    let config = request.deploy_base.join(".cursor/hooks.json");
    fs::create_dir_all(config.parent().unwrap().as_std_path()).unwrap();
    fs::write(
        config.as_std_path(),
        r#"{"provider":"foreign","hooks":{"afterShellExecution":[{"command":"./hooks/foreign.sh"}]}}"#,
    )
    .unwrap();
    let ledger = MemoryProjectionLedger::default();

    apply(root, &request, &ledger).expect("Hook generated projection should be executable");

    let rendered: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(config.as_std_path()).unwrap()).unwrap();
    assert_eq!(rendered["provider"], "foreign");
    assert_eq!(
        rendered["hooks"]["afterShellExecution"][0]["command"],
        "./hooks/foreign.sh"
    );
    assert!(
        rendered["hooks"]
            .as_object()
            .unwrap()
            .values()
            .flat_map(|items| items.as_array().into_iter().flatten())
            .any(|item| item["hook"] == "format.sh"),
        "the managed binding must be added without replacing the foreign binding"
    );
}

#[test]
fn hook_apply_preserves_foreign_codex_group_and_unknown_fields() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = request(root, hook_asset(root, "format.sh"), PlatformId::Codex);
    let config = request.deploy_base.join(".codex/hooks.json");
    fs::create_dir_all(config.parent().unwrap().as_std_path()).unwrap();
    fs::write(
        config.as_std_path(),
        r#"{"provider":"foreign","hooks":{"PostToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"./hooks/foreign.sh"}]}]}}"#,
    )
    .unwrap();
    let ledger = MemoryProjectionLedger::default();

    apply(root, &request, &ledger).expect("Codex Hook projection should preserve foreign config");

    let rendered: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(config.as_std_path()).unwrap()).unwrap();
    assert_eq!(rendered["provider"], "foreign");
    assert_eq!(
        rendered["hooks"]["PostToolUse"][0]["hooks"][0]["command"],
        "./hooks/foreign.sh"
    );
    assert!(rendered["hooks"]["PostToolUse"]
        .as_array()
        .unwrap()
        .iter()
        .any(|group| group["hooks"]
            .as_array()
            .is_some_and(|hooks| { hooks.iter().any(|hook| hook["hook"] == "format.sh") })));
}

#[test]
fn hook_apply_preserves_foreign_claude_group_and_unknown_fields() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = request(root, hook_asset(root, "format.sh"), PlatformId::Claude);
    let config = request.deploy_base.join(".claude/settings.json");
    fs::create_dir_all(config.parent().unwrap().as_std_path()).unwrap();
    fs::write(
        config.as_std_path(),
        r#"{"mcpServers":{"foreign":{"command":"foreign"}},"customSetting":true,"hooks":{"PostToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"./hooks/foreign.sh"}]}]}}"#,
    )
    .unwrap();
    let ledger = MemoryProjectionLedger::default();

    apply(root, &request, &ledger).expect("Claude Hook projection should preserve foreign config");

    let rendered: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(config.as_std_path()).unwrap()).unwrap();
    assert_eq!(rendered["customSetting"], true);
    assert_eq!(rendered["mcpServers"]["foreign"]["command"], "foreign");
    assert_eq!(
        rendered["hooks"]["PostToolUse"][0]["hooks"][0]["command"],
        "./hooks/foreign.sh"
    );
    assert!(rendered["hooks"]["PostToolUse"]
        .as_array()
        .unwrap()
        .iter()
        .any(|group| group["hooks"]
            .as_array()
            .is_some_and(|hooks| { hooks.iter().any(|hook| hook["hook"] == "format.sh") })));
}

#[test]
fn hook_bundle_is_linked_as_one_unit_after_its_binding_is_generated() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let hook = hook_bundle(root, "lint-bundle");
    let request = request(root, hook.clone(), PlatformId::Codex);
    let ledger = MemoryProjectionLedger::default();

    apply(root, &request, &ledger).expect("bundle Hook apply should succeed");

    let target = request.deploy_base.join(".codex/hooks/lint-bundle");
    assert_eq!(
        fs::read_link(target.as_std_path()).unwrap(),
        hook.source_path,
        "a directory bundle is one direct-link unit, never copied script-by-script"
    );
    assert!(request.deploy_base.join(".codex/hooks.json").is_file());
}

#[test]
fn hook_apply_failure_restores_config_and_script_link_together() {
    struct FailingLedger;

    impl ProjectionLedger for FailingLedger {
        fn get(
            &self,
            _id: &ai_config_core::projection::model::ProjectionId,
        ) -> Result<Option<ProjectionRecord>, ai_config_core::error::CoreError> {
            Ok(None)
        }

        fn get_many(
            &self,
            _ids: &[ai_config_core::projection::model::ProjectionId],
        ) -> Result<Vec<ProjectionRecord>, ai_config_core::error::CoreError> {
            Ok(Vec::new())
        }

        fn list_scope(
            &self,
            _scope_key: &str,
        ) -> Result<Vec<ProjectionRecord>, ai_config_core::error::CoreError> {
            Ok(Vec::new())
        }

        fn apply_batch(
            &self,
            _mutations: &[LedgerMutation],
        ) -> Result<(), ai_config_core::error::CoreError> {
            Err(ai_config_core::error::CoreError::ProjectionLedger(
                "forced ledger failure".to_owned(),
            ))
        }
    }

    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = request(root, hook_asset(root, "rollback.sh"), PlatformId::Cursor);
    let config = request.deploy_base.join(".cursor/hooks.json");
    fs::create_dir_all(config.parent().unwrap().as_std_path()).unwrap();
    fs::write(config.as_std_path(), r#"{"foreign":true,"hooks":{}}"#).unwrap();
    let before = fs::read_to_string(config.as_std_path()).unwrap();
    let ledger = FailingLedger;
    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    let failure = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    );

    assert!(failure.is_err());
    assert_eq!(fs::read_to_string(config.as_std_path()).unwrap(), before);
    assert!(
        !request
            .deploy_base
            .join(".cursor/hooks/rollback.sh")
            .exists(),
        "the linked script must roll back with the generated binding"
    );
}

#[test]
fn hook_retract_removes_only_ledger_owned_binding_and_link() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let hook = hook_asset(root, "owned.sh");
    let mut request = request(root, hook.clone(), PlatformId::Cursor);
    let config = request.deploy_base.join(".cursor/hooks.json");
    let script = request.deploy_base.join(".cursor/hooks/owned.sh");
    fs::create_dir_all(request.deploy_base.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let sync = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    fs::create_dir_all(script.parent().unwrap().as_std_path()).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(hook.source_path.as_std_path(), script.as_std_path()).unwrap();
    #[cfg(not(unix))]
    fs::write(script.as_std_path(), "fixture").unwrap();
    fs::write(
        config.as_std_path(),
        r#"{"foreign":"keep","hooks":{"afterShellExecution":[{"command":"./hooks/foreign.sh"},{"command":".cursor/hooks/owned.sh","managedBy":"agent-manager","hook":"owned.sh"}]}}"#,
    )
    .unwrap();
    for action in &sync.actions {
        for member in &action.members {
            let target = action.target.as_ref().unwrap();
            ledger
                .apply_batch(&[LedgerMutation::Upsert(ProjectionRecord {
                    id: member.id.clone(),
                    mode: match action.kind {
                        ProjectionActionKind::CreateLink | ProjectionActionKind::Noop => {
                            ProjectionMode::DirectLink
                        }
                        ProjectionActionKind::UpsertGeneratedBatch => ProjectionMode::GeneratedJson,
                        _ => continue,
                    },
                    source_path: member.source.absolute_path.clone(),
                    target_path: target.path.clone(),
                    entry_key: member.entry_key.clone(),
                    source_fingerprint: member.source.fingerprint.clone(),
                    entry_fingerprint: None,
                    target_fingerprint: path_content_digest(&target.path).unwrap(),
                    applied_at: Utc::now(),
                })])
                .unwrap();
        }
    }
    request.operation = ProjectionOperation::Retract;

    apply(root, &request, &ledger).expect("ledger-owned Hook retract should succeed");

    let rendered = fs::read_to_string(config.as_std_path()).unwrap();
    assert!(rendered.contains("foreign.sh"));
    assert!(!rendered.contains("owned.sh"));
    assert!(!script.exists());
}

#[test]
fn hook_drift_blocks_retract_before_touching_foreign_or_script() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let hook = hook_asset(root, "drift.sh");
    let mut request = request(root, hook.clone(), PlatformId::Cursor);
    let config = request.deploy_base.join(".cursor/hooks.json");
    let script = request.deploy_base.join(".cursor/hooks/drift.sh");
    fs::create_dir_all(request.deploy_base.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let sync = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    fs::create_dir_all(script.parent().unwrap().as_std_path()).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(hook.source_path.as_std_path(), script.as_std_path()).unwrap();
    #[cfg(not(unix))]
    fs::write(script.as_std_path(), "fixture").unwrap();
    fs::write(
        config.as_std_path(),
        r#"{"hooks":{"afterShellExecution":[]}}"#,
    )
    .unwrap();
    let generated = sync
        .actions
        .iter()
        .find(|action| action.kind == ProjectionActionKind::UpsertGeneratedBatch)
        .unwrap();
    let member = generated.members.first().unwrap();
    ledger
        .apply_batch(&[LedgerMutation::Upsert(ProjectionRecord {
            id: member.id.clone(),
            mode: ProjectionMode::GeneratedJson,
            source_path: member.source.absolute_path.clone(),
            target_path: config.clone(),
            entry_key: member.entry_key.clone(),
            source_fingerprint: member.source.fingerprint.clone(),
            entry_fingerprint: None,
            target_fingerprint: path_content_digest(&config).unwrap(),
            applied_at: Utc::now(),
        })])
        .unwrap();
    fs::write(
        config.as_std_path(),
        r#"{"foreign":"changed-after-plan","hooks":{"afterShellExecution":[]}}"#,
    )
    .unwrap();
    let before = fs::read_to_string(config.as_std_path()).unwrap();
    request.operation = ProjectionOperation::Retract;

    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    assert!(plan.actions.iter().any(|action| {
        action.kind == ProjectionActionKind::ReportOnly
            && action.state.as_deref() == Some("drifted")
    }));
    assert!(apply(root, &request, &ledger).is_err());
    assert_eq!(fs::read_to_string(config.as_std_path()).unwrap(), before);
    assert!(script.exists());
}

#[test]
fn hook_overlay_uses_project_bundle_without_writing_any_target() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let global = root.join("global");
    let project = root.join("project");
    fs::create_dir_all(global.join("hooks").as_std_path()).unwrap();
    fs::create_dir_all(project.join("hooks/lint").as_std_path()).unwrap();
    fs::write(global.join("hooks/lint").as_std_path(), "global\n").unwrap();
    fs::write(
        project.join("hooks/lint/hook.yaml").as_std_path(),
        "project\n",
    )
    .unwrap();

    let assets = resolve_effective_assets(&OverlayRoots {
        global,
        workspace: None,
        project: project.clone(),
    })
    .unwrap();

    let hook = assets
        .iter()
        .find(|asset| asset.kind == AssetKind::Hook && asset.name == "lint")
        .unwrap();
    assert_eq!(hook.layer, SourceLayer::Project);
    assert_eq!(hook.source_path, project.join("hooks/lint"));
    assert!(!root.join("deploy").exists());
}

#[test]
fn legacy_hook_import_refuses_to_replace_source_without_a_transaction_backup() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset_root = root.join("assets");
    let deploy_base = root.join("deploy");
    fs::create_dir_all(asset_root.join("hooks").as_std_path()).unwrap();
    fs::write(
        asset_root.join("hooks/import.sh").as_std_path(),
        "old canonical\n",
    )
    .unwrap();
    fs::create_dir_all(deploy_base.join(".cursor/hooks").as_std_path()).unwrap();
    fs::write(
        deploy_base.join(".cursor/hooks/import.sh").as_std_path(),
        "platform replacement\n",
    )
    .unwrap();
    fs::write(
        deploy_base.join(".cursor/hooks.json").as_std_path(),
        r#"{"hooks":{"afterShellExecution":[{"command":"./hooks/import.sh"}]}}"#,
    )
    .unwrap();

    let result =
        hook_adapter::import_to_source(&asset_root, &deploy_base, "import.sh", PlatformId::Cursor);

    assert!(
        result.is_err(),
        "legacy import must be gated until an explicit migration transaction records a backup"
    );
    assert_eq!(
        fs::read_to_string(asset_root.join("hooks/import.sh").as_std_path()).unwrap(),
        "old canonical\n"
    );
}

#[test]
fn hermes_project_hook_is_unsupported_and_never_changes_global_config() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = request(root, hook_asset(root, "hermes.sh"), PlatformId::Hermes);
    let global_config = root.join("not-home/.hermes/config.yaml");
    fs::create_dir_all(global_config.parent().unwrap().as_std_path()).unwrap();
    fs::write(global_config.as_std_path(), "provider: foreign\n").unwrap();
    let before = fs::read_to_string(global_config.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();

    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    assert_eq!(plan.actions.len(), 1);
    assert_eq!(plan.actions[0].kind, ProjectionActionKind::ReportOnly);
    assert_eq!(plan.actions[0].state.as_deref(), Some("unsupported"));
    assert!(!request.deploy_base.join(".hermes/config.yaml").exists());
    assert_eq!(
        fs::read_to_string(global_config.as_std_path()).unwrap(),
        before
    );
}
