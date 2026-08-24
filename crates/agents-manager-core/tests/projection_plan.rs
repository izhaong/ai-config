use std::fs;
use std::time::{Duration, Instant};

use agents_manager_core::error::CoreError;
use agents_manager_core::model::{AssetKind, McpServer, McpTransport, PlatformId};
use agents_manager_core::projection::fingerprint::path_content_digest;
use agents_manager_core::projection::ledger::{MemoryProjectionLedger, ProjectionLedger};
use agents_manager_core::projection::mcp::entry_fingerprint::inspect_cursor_mcp_entries;
use agents_manager_core::projection::mcp::source::{EffectiveMcpDefinition, McpDefinition};
use agents_manager_core::projection::model::{
    DeploymentScope, EffectiveAsset, LedgerMutation, ProjectionId, ProjectionMode,
    ProjectionRecord, ProjectionSurface, SourceLayer, SourceRef,
};
use agents_manager_core::projection::planner::{
    build_mcp_projection_plan, build_projection_plan, GeneratedContainerRenderer, PlannerContext,
    ProjectionActionKind, ProjectionOperation, ProjectionRequest,
};
use agents_manager_core::projection::platform_adapter::TrustRequirement;
use camino::Utf8Path;
use tempfile::TempDir;

fn asset(root: &Utf8Path, kind: AssetKind, name: &str, contents: &str) -> EffectiveAsset {
    let source_path = if kind == AssetKind::Skill {
        let source_dir = root.join("source/skills").join(name);
        fs::create_dir_all(source_dir.as_std_path()).unwrap();
        fs::write(source_dir.join("SKILL.md").as_std_path(), contents).unwrap();
        source_dir
    } else {
        let source_file = root.join("source").join(format!("{name}.md"));
        fs::create_dir_all(source_file.parent().unwrap().as_std_path()).unwrap();
        fs::write(source_file.as_std_path(), contents).unwrap();
        source_file
    };
    let fingerprint = path_content_digest(&source_path).unwrap();
    EffectiveAsset {
        kind,
        name: name.to_owned(),
        source_path,
        layer: SourceLayer::Project,
        fingerprint,
    }
}

struct FailingLedger;

impl ProjectionLedger for FailingLedger {
    fn get(
        &self,
        _id: &agents_manager_core::projection::model::ProjectionId,
    ) -> Result<Option<agents_manager_core::projection::model::ProjectionRecord>, CoreError> {
        Err(CoreError::ProjectionLedger(
            "fixture read failure".to_owned(),
        ))
    }

    fn get_many(
        &self,
        _ids: &[agents_manager_core::projection::model::ProjectionId],
    ) -> Result<Vec<agents_manager_core::projection::model::ProjectionRecord>, CoreError> {
        Err(CoreError::ProjectionLedger(
            "fixture read failure".to_owned(),
        ))
    }

    fn list_scope(
        &self,
        _scope_key: &str,
    ) -> Result<Vec<agents_manager_core::projection::model::ProjectionRecord>, CoreError> {
        Err(CoreError::ProjectionLedger(
            "fixture read failure".to_owned(),
        ))
    }

    fn apply_batch(
        &self,
        _mutations: &[agents_manager_core::projection::model::LedgerMutation],
    ) -> Result<(), CoreError> {
        Err(CoreError::ProjectionLedger(
            "fixture write failure".to_owned(),
        ))
    }
}

fn request(
    root: &Utf8Path,
    assets: Vec<EffectiveAsset>,
    platforms: Vec<PlatformId>,
) -> ProjectionRequest {
    ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets,
        platforms,
    }
}

fn mcp_definition(
    root: &Utf8Path,
    name: &str,
    config: serde_json::Value,
    targets: Vec<PlatformId>,
) -> EffectiveMcpDefinition {
    let source_path = root.join("mcp-sources").join(format!("{name}.json"));
    fs::create_dir_all(source_path.parent().unwrap().as_std_path()).unwrap();
    fs::write(
        source_path.as_std_path(),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let source = SourceRef {
        layer: SourceLayer::Project,
        absolute_path: source_path.clone(),
        fingerprint: path_content_digest(&source_path).unwrap(),
    };
    EffectiveMcpDefinition {
        definition: McpDefinition {
            server: McpServer {
                project_id: 0,
                name: name.to_owned(),
                transport: McpTransport::Stdio,
                config,
                secret_keys: Vec::new(),
                enabled: true,
            },
            targets,
            source_path,
        },
        source,
    }
}

fn mcp_request(root: &Utf8Path, platforms: Vec<PlatformId>) -> ProjectionRequest {
    request(root, Vec::new(), platforms)
}

#[cfg(unix)]
#[test]
fn retract_only_plans_removal_for_an_exact_managed_link() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = asset(root, AssetKind::Skill, "review", "body");
    let mut request = request(root, vec![asset.clone()], vec![PlatformId::Cursor]);
    request.operation = ProjectionOperation::Retract;
    let target = request.deploy_base.join(".agents/skills/review");
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    std::os::unix::fs::symlink(asset.source_path.as_std_path(), target.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();

    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    assert!(matches!(
        plan.actions[0].kind,
        ProjectionActionKind::RemoveManagedLink
    ));
    assert!(std::fs::symlink_metadata(target.as_std_path()).is_ok());
}

#[test]
fn missing_shared_skill_creates_one_read_only_link_action_with_stable_digest() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = request(
        root,
        vec![asset(
            root,
            AssetKind::Skill,
            "review",
            "SENTINEL_SECRET_VALUE",
        )],
        vec![PlatformId::Codex, PlatformId::Cursor],
    );
    let target = request.deploy_base.join(".agents/skills/review");
    let ledger = MemoryProjectionLedger::default();
    let context = PlannerContext::new(&ledger);

    let first = build_projection_plan(&request, &context).unwrap();
    let second = build_projection_plan(&request, &context).unwrap();

    assert!(
        !target.exists(),
        "planning must not create a platform target"
    );
    assert_eq!(first.plan_digest, second.plan_digest);
    assert_eq!(first.action_ids, second.action_ids);
    assert_eq!(first.action_ids.len(), first.actions.len());
    assert_eq!(
        first.action_ids[0].len(),
        64,
        "action id is a full SHA-256 digest"
    );
    assert_eq!(
        first.actions.len(),
        1,
        "shared physical target is one action"
    );
    assert!(matches!(
        first.actions[0].kind,
        ProjectionActionKind::CreateLink
    ));
    assert_eq!(
        first.actions[0].consumers,
        vec![PlatformId::Cursor, PlatformId::Codex]
    );
    assert_eq!(first.actions[0].target.as_ref().unwrap().path, target);
    assert!(!serde_json::to_string(&first)
        .unwrap()
        .contains("SENTINEL_SECRET_VALUE"));
}

#[test]
fn unsupported_contract_is_report_only_and_never_creates_a_path() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = request(
        root,
        vec![asset(root, AssetKind::Command, "review", "body")],
        vec![PlatformId::Codex],
    );
    let ledger = MemoryProjectionLedger::default();

    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    assert_eq!(plan.actions.len(), 1);
    assert!(matches!(
        plan.actions[0].kind,
        ProjectionActionKind::ReportOnly
    ));
    assert_eq!(plan.actions[0].state.as_deref(), Some("unsupported"));
    assert!(!request.deploy_base.exists());
}

#[test]
fn generated_members_for_one_container_are_batched_without_writing() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = mcp_request(root, vec![PlatformId::Cursor]);
    let definitions = vec![
        mcp_definition(
            root,
            "catalog",
            serde_json::json!({"command":"catalog"}),
            vec![PlatformId::Cursor],
        ),
        mcp_definition(
            root,
            "search",
            serde_json::json!({"command":"search"}),
            vec![PlatformId::Cursor],
        ),
    ];
    let target = request.deploy_base.join(".cursor/mcp.json");
    let ledger = MemoryProjectionLedger::default();

    let plan =
        build_mcp_projection_plan(&request, &definitions, &PlannerContext::new(&ledger)).unwrap();

    assert_eq!(plan.actions.len(), 1);
    assert!(matches!(
        plan.actions[0].kind,
        ProjectionActionKind::UpsertGeneratedBatch
    ));
    assert!(plan.actions[0].members.is_empty());
    assert_eq!(plan.actions[0].mcp_members.len(), 2);
    assert_eq!(
        plan.actions[0].generated_renderer,
        Some(GeneratedContainerRenderer::McpJson),
        "the reviewed plan must bind a generated action to one container renderer"
    );
    assert_eq!(plan.actions[0].target.as_ref().unwrap().path, target);
    assert_eq!(
        plan.actions[0]
            .mcp_members
            .iter()
            .map(|member| Some(member.entry_key.as_str()))
            .collect::<Vec<_>>(),
        vec![Some("mcpServers.catalog"), Some("mcpServers.search")],
        "the batch must retain per-entry renderer keys"
    );
    assert_eq!(
        plan.actions[0].target.as_ref().unwrap().entry_key,
        None,
        "the batch target denotes the container, not one member entry"
    );
    assert!(!target.exists());
}

#[test]
fn generated_retract_removes_only_ledger_owned_entries_not_the_container() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let definition = mcp_definition(
        root,
        "catalog",
        serde_json::json!({"command":"catalog"}),
        vec![PlatformId::Cursor],
    );
    let mut request = mcp_request(root, vec![PlatformId::Cursor]);
    request.operation = ProjectionOperation::Retract;
    let target = request.deploy_base.join(".cursor/mcp.json");
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    fs::write(target.as_std_path(), "{\"mcpServers\":{\"catalog\":{}}}").unwrap();
    let target_fingerprint = path_content_digest(&target).unwrap();
    let ledger = MemoryProjectionLedger::default();
    ledger
        .apply_batch(&[LedgerMutation::Upsert(ProjectionRecord {
            id: ProjectionId {
                scope_key: request.scope_key.clone(),
                kind: AssetKind::Mcp,
                name: "catalog".to_owned(),
                surface: ProjectionSurface::Platform(PlatformId::Cursor),
            },
            mode: ProjectionMode::GeneratedJson,
            source_path: definition.source.absolute_path.clone(),
            target_path: target.clone(),
            entry_key: Some("mcpServers.catalog".to_owned()),
            source_fingerprint: definition.source.fingerprint.clone(),
            entry_fingerprint: Some(
                inspect_cursor_mcp_entries(&fs::read_to_string(target.as_std_path()).unwrap())
                    .unwrap()[0]
                    .digest
                    .clone(),
            ),
            target_fingerprint,
            applied_at: chrono::Utc::now(),
        })])
        .unwrap();

    let plan =
        build_mcp_projection_plan(&request, &[definition], &PlannerContext::new(&ledger)).unwrap();

    assert!(matches!(
        plan.actions[0].kind,
        ProjectionActionKind::RemoveGeneratedEntries
    ));
    assert_eq!(plan.actions[0].mcp_members.len(), 1);
    assert!(target.exists(), "planning must not remove the container");
}

#[test]
fn generated_source_change_plans_an_upsert_without_treating_the_target_as_foreign() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let definition = mcp_definition(
        root,
        "catalog",
        serde_json::json!({"command":"catalog-v2"}),
        vec![PlatformId::Cursor],
    );
    let request = mcp_request(root, vec![PlatformId::Cursor]);
    let target = request.deploy_base.join(".cursor/mcp.json");
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    fs::write(target.as_std_path(), "{\"mcpServers\":{\"catalog\":{}}}").unwrap();
    let ledger = MemoryProjectionLedger::default();
    ledger
        .apply_batch(&[LedgerMutation::Upsert(ProjectionRecord {
            id: ProjectionId {
                scope_key: request.scope_key.clone(),
                kind: AssetKind::Mcp,
                name: "catalog".to_owned(),
                surface: ProjectionSurface::Platform(PlatformId::Cursor),
            },
            mode: ProjectionMode::GeneratedJson,
            source_path: definition.source.absolute_path.clone(),
            target_path: target,
            entry_key: Some("mcpServers.catalog".to_owned()),
            source_fingerprint: "old-source-fingerprint".to_owned(),
            entry_fingerprint: Some(
                inspect_cursor_mcp_entries(
                    &fs::read_to_string(request.deploy_base.join(".cursor/mcp.json").as_std_path())
                        .unwrap(),
                )
                .unwrap()[0]
                    .digest
                    .clone(),
            ),
            target_fingerprint: path_content_digest(&request.deploy_base.join(".cursor/mcp.json"))
                .unwrap(),
            applied_at: chrono::Utc::now(),
        })])
        .unwrap();

    let plan =
        build_mcp_projection_plan(&request, &[definition], &PlannerContext::new(&ledger)).unwrap();

    assert!(matches!(
        plan.actions[0].kind,
        ProjectionActionKind::UpsertGeneratedBatch
    ));
    assert_eq!(plan.actions[0].reason_code, "generated_source_changed");
}

#[test]
fn unchanged_generated_entry_with_matching_ledger_is_a_noop() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let definition = mcp_definition(
        root,
        "catalog",
        serde_json::json!({"command":"catalog"}),
        vec![PlatformId::Cursor],
    );
    let request = mcp_request(root, vec![PlatformId::Cursor]);
    let target = request.deploy_base.join(".cursor/mcp.json");
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    fs::write(target.as_std_path(), "{\"mcpServers\":{\"catalog\":{}}}").unwrap();
    let target_fingerprint = path_content_digest(&target).unwrap();
    let ledger = MemoryProjectionLedger::default();
    ledger
        .apply_batch(&[LedgerMutation::Upsert(ProjectionRecord {
            id: ProjectionId {
                scope_key: request.scope_key.clone(),
                kind: AssetKind::Mcp,
                name: "catalog".to_owned(),
                surface: ProjectionSurface::Platform(PlatformId::Cursor),
            },
            mode: ProjectionMode::GeneratedJson,
            source_path: definition.source.absolute_path.clone(),
            target_path: target.clone(),
            entry_key: Some("mcpServers.catalog".to_owned()),
            source_fingerprint: definition.source.fingerprint.clone(),
            entry_fingerprint: Some(
                inspect_cursor_mcp_entries(&fs::read_to_string(target.as_std_path()).unwrap())
                    .unwrap()[0]
                    .digest
                    .clone(),
            ),
            target_fingerprint,
            applied_at: chrono::Utc::now(),
        })])
        .unwrap();

    let plan =
        build_mcp_projection_plan(&request, &[definition], &PlannerContext::new(&ledger)).unwrap();

    assert!(matches!(plan.actions[0].kind, ProjectionActionKind::Noop));
    assert_eq!(plan.actions[0].reason_code, "managed_generated_noop");
    assert_eq!(
        Some(plan.actions[0].mcp_members[0].entry_key.as_str()),
        Some("mcpServers.catalog")
    );
    assert_eq!(plan.actions[0].target.as_ref().unwrap().entry_key, None);
}

#[test]
fn prompt_uses_one_project_entry_instead_of_platform_specific_legacy_paths() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = request(
        root,
        vec![asset(root, AssetKind::Prompt, "AGENTS", "canonical prompt")],
        vec![PlatformId::Codex, PlatformId::Cursor, PlatformId::Claude],
    );
    let ledger = MemoryProjectionLedger::default();

    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    assert_eq!(plan.actions.len(), 1);
    assert!(matches!(
        plan.actions[0].kind,
        ProjectionActionKind::CreateLink
    ));
    assert_eq!(
        plan.actions[0].target.as_ref().unwrap().path,
        request.deploy_base.join("AGENTS.md")
    );
    assert_eq!(
        plan.actions[0].consumers,
        vec![PlatformId::Cursor, PlatformId::Codex, PlatformId::Claude]
    );
}

#[test]
fn hook_plans_generated_binding_and_linked_script_as_distinct_zero_write_actions() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = request(
        root,
        vec![asset(
            root,
            AssetKind::Hook,
            "check.sh",
            "#!/bin/sh\nexit 0\n",
        )],
        vec![PlatformId::Codex],
    );
    let ledger = MemoryProjectionLedger::default();

    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    assert_eq!(plan.actions.len(), 2);
    let binding =
        plan.actions
            .iter()
            .find(|action| {
                action.target.as_ref().is_some_and(|target| {
                    target.path == request.deploy_base.join(".codex/hooks.json")
                })
            })
            .unwrap();
    let script = plan
        .actions
        .iter()
        .find(|action| {
            action.target.as_ref().is_some_and(|target| {
                target.path == request.deploy_base.join(".codex/hooks/check.sh")
            })
        })
        .unwrap();
    assert!(matches!(
        binding.kind,
        ProjectionActionKind::UpsertGeneratedBatch
    ));
    assert!(matches!(script.kind, ProjectionActionKind::CreateLink));
    assert_ne!(binding.members[0].id.surface, script.members[0].id.surface);
    assert!(plan
        .trust_requirements
        .iter()
        .all(|trust| *trust == TrustRequirement::TrustedProjectWithIndependentReview));
    assert!(!request.deploy_base.exists());
}

#[test]
fn codex_project_mcp_plan_declares_trusted_project_precondition() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = mcp_request(root, vec![PlatformId::Codex]);
    let definitions = vec![mcp_definition(
        root,
        "catalog",
        serde_json::json!({"command":"catalog"}),
        vec![PlatformId::Codex],
    )];
    let ledger = MemoryProjectionLedger::default();

    let plan = build_mcp_projection_plan(&request, &definitions, &PlannerContext::new(&ledger));
    let plan = plan.unwrap();

    assert_eq!(
        plan.trust_requirements,
        vec![TrustRequirement::TrustedProject]
    );
}

#[test]
fn repeated_planning_is_read_only_and_keeps_ids_and_digest_stable() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = request(
        root,
        vec![
            asset(root, AssetKind::Skill, "review", "skill"),
            asset(root, AssetKind::Mcp, "catalog", "{\"command\":\"catalog\"}"),
            asset(root, AssetKind::Hook, "check.sh", "#!/bin/sh\nexit 0\n"),
        ],
        vec![PlatformId::Cursor, PlatformId::Codex],
    );
    let ledger = MemoryProjectionLedger::default();
    let before = agents_manager_core::projection::fingerprint::directory_digest(root).unwrap();
    let baseline = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    for _ in 0..100 {
        let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
        assert_eq!(plan.plan_digest, baseline.plan_digest);
        assert_eq!(plan.action_ids, baseline.action_ids);
    }

    assert_eq!(
        agents_manager_core::projection::fingerprint::directory_digest(root).unwrap(),
        before,
        "planning must not create target directories, ledger records, or files"
    );
}

#[test]
fn sync_reports_ledger_orphans_without_removing_or_writing_them() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = request(root, Vec::new(), vec![PlatformId::Cursor]);
    let orphan_target = request.deploy_base.join(".cursor/commands/obsolete.md");
    let ledger = MemoryProjectionLedger::default();
    ledger
        .apply_batch(&[LedgerMutation::Upsert(ProjectionRecord {
            id: ProjectionId {
                scope_key: request.scope_key.clone(),
                kind: AssetKind::Command,
                name: "obsolete".to_owned(),
                surface: ProjectionSurface::Platform(PlatformId::Cursor),
            },
            mode: ProjectionMode::DirectLink,
            source_path: root.join("old-source.md"),
            target_path: orphan_target.clone(),
            entry_key: None,
            source_fingerprint: "old-source".to_owned(),
            entry_fingerprint: None,
            target_fingerprint: "old-target".to_owned(),
            applied_at: chrono::Utc::now(),
        })])
        .unwrap();

    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    assert_eq!(plan.actions.len(), 1);
    assert!(matches!(
        plan.actions[0].kind,
        ProjectionActionKind::ReportOnly
    ));
    assert_eq!(plan.actions[0].state.as_deref(), Some("orphan_candidate"));
    assert_eq!(plan.actions[0].target.as_ref().unwrap().path, orphan_target);
    assert!(!request.deploy_base.exists());
}

#[cfg(unix)]
#[test]
fn cleanup_orphans_requires_unchanged_ledger_owned_target_and_stays_read_only() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let mut request = request(root, Vec::new(), vec![PlatformId::Cursor]);
    request.operation = ProjectionOperation::CleanupOrphans;
    let target = request.deploy_base.join(".cursor/commands/obsolete.md");
    let source = root.join("retired/obsolete.md");
    fs::create_dir_all(source.parent().unwrap().as_std_path()).unwrap();
    fs::write(source.as_std_path(), "retired command").unwrap();
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    std::os::unix::fs::symlink(source.as_std_path(), target.as_std_path()).unwrap();
    let target_fingerprint = path_content_digest(&target).unwrap();
    let ledger = MemoryProjectionLedger::default();
    ledger
        .apply_batch(&[LedgerMutation::Upsert(ProjectionRecord {
            id: ProjectionId {
                scope_key: request.scope_key.clone(),
                kind: AssetKind::Command,
                name: "obsolete".to_owned(),
                surface: ProjectionSurface::Platform(PlatformId::Cursor),
            },
            mode: ProjectionMode::DirectLink,
            source_path: source,
            target_path: target.clone(),
            entry_key: None,
            source_fingerprint: "deleted-source".to_owned(),
            entry_fingerprint: None,
            target_fingerprint: target_fingerprint.clone(),
            applied_at: chrono::Utc::now(),
        })])
        .unwrap();

    let cleanup = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    assert!(matches!(
        cleanup.actions[0].kind,
        ProjectionActionKind::CleanupOrphan
    ));
    assert_eq!(
        cleanup.actions[0]
            .ownership_fingerprint
            .as_ref()
            .map(String::len),
        Some(64),
        "the cleanup selector is a hash of the full ownership record"
    );
    assert!(
        target.exists(),
        "planning must not delete the orphan target"
    );

    fs::remove_file(target.as_std_path()).unwrap();
    fs::write(target.as_std_path(), "drifted external content").unwrap();
    let drifted = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    assert!(matches!(
        drifted.actions[0].kind,
        ProjectionActionKind::ReportOnly
    ));
    assert_eq!(drifted.actions[0].state.as_deref(), Some("drifted"));
    assert!(target.exists(), "drifted target must remain untouched");
}

#[test]
fn cleanup_orphans_never_authorizes_a_regular_file_from_a_direct_link_record() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let mut request = request(root, Vec::new(), vec![PlatformId::Cursor]);
    request.operation = ProjectionOperation::CleanupOrphans;
    let target = request.deploy_base.join(".cursor/commands/obsolete.md");
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    fs::write(target.as_std_path(), "foreign regular file").unwrap();
    let ledger = MemoryProjectionLedger::default();
    ledger
        .apply_batch(&[LedgerMutation::Upsert(ProjectionRecord {
            id: ProjectionId {
                scope_key: request.scope_key.clone(),
                kind: AssetKind::Command,
                name: "obsolete".to_owned(),
                surface: ProjectionSurface::Platform(PlatformId::Cursor),
            },
            mode: ProjectionMode::DirectLink,
            source_path: root.join("retired/obsolete.md"),
            target_path: target.clone(),
            entry_key: None,
            source_fingerprint: "retired-source".to_owned(),
            entry_fingerprint: None,
            target_fingerprint: path_content_digest(&target).unwrap(),
            applied_at: chrono::Utc::now(),
        })])
        .unwrap();

    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    assert!(matches!(
        plan.actions[0].kind,
        ProjectionActionKind::ReportOnly
    ));
    assert_eq!(plan.actions[0].state.as_deref(), Some("drifted"));
    assert!(target.exists(), "planning must not delete a regular file");
}

#[test]
fn import_and_migrate_operations_are_explicit_report_only_until_their_tasks_arrive() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = asset(root, AssetKind::Skill, "review", "skill");
    let ledger = MemoryProjectionLedger::default();

    for operation in [ProjectionOperation::Import, ProjectionOperation::Migrate] {
        let mut request = request(root, vec![asset.clone()], vec![PlatformId::Cursor]);
        request.operation = operation;
        let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

        assert_eq!(plan.actions.len(), 1);
        assert!(matches!(
            plan.actions[0].kind,
            ProjectionActionKind::ReportOnly
        ));
        assert_eq!(plan.actions[0].reason_code, "operation_not_implemented");
        assert!(!request.deploy_base.exists());
    }
}

#[test]
fn generic_mcp_assets_are_fail_closed_and_never_join_a_hook_generated_container() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let mut request = request(
        root,
        vec![
            asset(root, AssetKind::Mcp, "catalog", "{\"command\":\"catalog\"}"),
            asset(root, AssetKind::Hook, "check.sh", "#!/bin/sh\nexit 0\n"),
        ],
        vec![PlatformId::Hermes],
    );
    request.scope = DeploymentScope::User;
    let ledger = MemoryProjectionLedger::default();

    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    assert!(plan.actions.iter().any(|action| {
        action.reason_code == "mcp_requires_source_first_definition"
            && matches!(action.kind, ProjectionActionKind::ReportOnly)
            && action.target.is_none()
    }));
    assert!(!request.deploy_base.exists());
}

#[cfg(unix)]
#[test]
fn direct_link_sync_distinguishes_managed_link_from_foreign_link_without_writing() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = asset(root, AssetKind::Command, "review", "command");
    let request = request(root, vec![asset.clone()], vec![PlatformId::Cursor]);
    let target = request.deploy_base.join(".cursor/commands/review.md");
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    std::os::unix::fs::symlink(asset.source_path.as_std_path(), target.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();

    let managed = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    assert!(matches!(
        managed.actions[0].kind,
        ProjectionActionKind::Noop
    ));
    assert_eq!(managed.actions[0].state.as_deref(), Some("managed_link"));

    fs::remove_file(target.as_std_path()).unwrap();
    let foreign_source = root.join("third-party/review.md");
    fs::create_dir_all(foreign_source.parent().unwrap().as_std_path()).unwrap();
    fs::write(foreign_source.as_std_path(), "foreign command").unwrap();
    std::os::unix::fs::symlink(foreign_source.as_std_path(), target.as_std_path()).unwrap();

    let foreign = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    assert!(matches!(
        foreign.actions[0].kind,
        ProjectionActionKind::ReportOnly
    ));
    assert_eq!(foreign.actions[0].state.as_deref(), Some("foreign"));
    assert_eq!(foreign.actions[0].reason_code, "direct_target_conflict");
    assert!(std::fs::symlink_metadata(target.as_std_path()).is_ok());
}

#[test]
fn thousand_item_plan_p95_stays_within_read_only_budget() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let assets = (0..1_000)
        .map(|index| {
            asset(
                root,
                AssetKind::Command,
                &format!("command-{index:04}"),
                "body",
            )
        })
        .collect::<Vec<_>>();
    let request = request(root, assets, vec![PlatformId::Cursor]);
    let ledger = MemoryProjectionLedger::default();
    let before = agents_manager_core::projection::fingerprint::directory_digest(root).unwrap();
    let mut samples = Vec::with_capacity(100);

    for _ in 0..100 {
        let start = Instant::now();
        let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
        assert_eq!(plan.actions.len(), 1_000);
        samples.push(start.elapsed());
    }

    samples.sort();
    assert!(
        samples[94] < Duration::from_secs(2),
        "p95 planner latency exceeded 2 seconds: {:?}",
        samples[94]
    );
    assert_eq!(
        agents_manager_core::projection::fingerprint::directory_digest(root).unwrap(),
        before,
        "benchmark must remain read-only"
    );
}

#[test]
fn equivalent_regular_file_is_an_explicit_adoption_candidate_not_an_overwrite() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let asset = asset(root, AssetKind::Command, "review", "same command");
    let request = request(root, vec![asset], vec![PlatformId::Cursor]);
    let target = request.deploy_base.join(".cursor/commands/review.md");
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    fs::write(target.as_std_path(), "same command").unwrap();
    let ledger = MemoryProjectionLedger::default();

    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    assert!(matches!(
        plan.actions[0].kind,
        ProjectionActionKind::AdoptEquivalent
    ));
    assert_eq!(
        plan.actions[0].reason_code,
        "equivalent_requires_explicit_adoption"
    );
    assert_eq!(
        fs::read_to_string(target.as_std_path()).unwrap(),
        "same command"
    );
}

#[test]
fn ledger_read_failure_warns_and_never_proves_generated_ownership() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = mcp_request(root, vec![PlatformId::Cursor]);
    let definitions = vec![mcp_definition(
        root,
        "catalog",
        serde_json::json!({"command":"catalog"}),
        vec![PlatformId::Cursor],
    )];
    let target = request.deploy_base.join(".cursor/mcp.json");
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    fs::write(
        target.as_std_path(),
        "{\"mcpServers\":{\"catalog\":{\"command\":\"catalog\"}}}",
    )
    .unwrap();

    let plan =
        build_mcp_projection_plan(&request, &definitions, &PlannerContext::new(&FailingLedger))
            .unwrap();

    assert!(matches!(
        plan.actions[0].kind,
        ProjectionActionKind::ReportOnly
    ));
    assert_eq!(plan.actions[0].state.as_deref(), Some("foreign"));
    assert_eq!(plan.warnings.len(), 1);
    assert_eq!(plan.warnings[0].code, "ledger_unavailable");
}

#[test]
fn source_occupying_shared_agents_skills_target_is_noop() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let deploy_base = root.join("deploy");
    let source_dir = deploy_base.join(".agents/skills/review");
    fs::create_dir_all(source_dir.as_std_path()).unwrap();
    fs::write(source_dir.join("SKILL.md").as_std_path(), "canonical\n").unwrap();
    let fingerprint = path_content_digest(&source_dir).unwrap();
    let asset = EffectiveAsset {
        kind: AssetKind::Skill,
        name: "review".to_owned(),
        source_path: source_dir.clone(),
        layer: SourceLayer::Project,
        fingerprint,
    };
    let mut request = request(root, vec![asset], vec![PlatformId::Cursor, PlatformId::Codex]);
    request.deploy_base = deploy_base;
    let ledger = MemoryProjectionLedger::default();

    let sync = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    assert_eq!(sync.actions.len(), 1);
    assert!(matches!(sync.actions[0].kind, ProjectionActionKind::Noop));
    assert_eq!(sync.actions[0].reason_code, "source_is_canonical_target");
    assert!(ledger.list_scope("project:/fixture").unwrap().is_empty());

    request.operation = ProjectionOperation::Retract;
    let retract = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    assert!(matches!(retract.actions[0].kind, ProjectionActionKind::Noop));
    assert_eq!(retract.actions[0].reason_code, "source_is_canonical_target");
    assert!(source_dir.join("SKILL.md").is_file());
}
