use std::fs;

use ai_config_core::error::CoreError;
use ai_config_core::model::PlatformId;
use ai_config_core::projection::executor::{
    apply_projection_plan, ApplyOptions, ExecutorContext, McpSecretProvider,
};
use ai_config_core::projection::ledger::{MemoryProjectionLedger, ProjectionLedger};
use ai_config_core::projection::mcp::claude_json::{
    render_claude_mcp_json, ClaudeJsonServerIntent,
};
use ai_config_core::projection::mcp::codex_toml::{render_codex_mcp_toml, TomlServerIntent};
use ai_config_core::projection::mcp::cursor_json::{render_cursor_mcp_json, JsonServerIntent};
use ai_config_core::projection::mcp::entry_fingerprint::{
    inspect_claude_mcp_entries, inspect_codex_mcp_entries, inspect_cursor_mcp_entries,
    inspect_hermes_mcp_entries,
};
use ai_config_core::projection::mcp::hermes_yaml::{render_hermes_mcp_yaml, YamlServerIntent};
use ai_config_core::projection::mcp::source::{
    load_mcp_definitions, resolve_effective_mcp_definitions,
};
use ai_config_core::projection::model::SourceLayer;
use ai_config_core::projection::model::{
    DeploymentScope, LedgerMutation, ProjectionId, ProjectionMode, ProjectionRecord,
    ProjectionSurface,
};
use ai_config_core::projection::planner::{
    build_mcp_projection_plan, McpSecretAvailability, PlannerContext, ProjectionActionKind,
    ProjectionOperation, ProjectionRequest,
};
use ai_config_core::projection::source::OverlayRoots;
use camino::Utf8Path;
use tempfile::TempDir;

fn write_server(root: &Utf8Path, name: &str, body: &str) {
    let path = root.join("mcp/servers").join(format!("{name}.json"));
    fs::create_dir_all(path.parent().unwrap().as_std_path()).unwrap();
    fs::write(path.as_std_path(), body).unwrap();
}

fn mcp_request(root: &Utf8Path, platforms: Vec<PlatformId>) -> ProjectionRequest {
    ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/mcp-fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("deploy"),
        assets: Vec::new(),
        platforms,
    }
}

struct StaticSecretProvider;

impl McpSecretProvider for StaticSecretProvider {
    fn resolve(&self, key: &str) -> Result<Option<String>, CoreError> {
        Ok((key == "CATALOG_TOKEN").then(|| "test-secret-value".to_owned()))
    }
}

struct MissingSecretAvailability;

impl McpSecretAvailability for MissingSecretAvailability {
    fn missing_secret_keys(&self, declared_keys: &[String]) -> Result<Vec<String>, CoreError> {
        Ok(declared_keys
            .iter()
            .filter(|key| key.as_str() == "MISSING_TOKEN")
            .cloned()
            .collect())
    }
}

struct FailingMcpLedger;

impl ProjectionLedger for FailingMcpLedger {
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
            "test ledger failure".to_owned(),
        ))
    }
}

#[test]
fn mcp_planner_emits_only_enabled_targeted_source_intents_without_config_values() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{
          "enabled": true,
          "targets": ["cursor"],
          "config": {
            "command": "catalog-mcp",
            "env": {"CATALOG_TOKEN": "${CATALOG_TOKEN}"}
          }
        }"#,
    );
    write_server(
        root,
        "disabled",
        r#"{"enabled":false,"targets":["cursor"],"config":{"command":"disabled-mcp"}}"#,
    );
    write_server(
        root,
        "codex-only",
        r#"{"targets":["codex"],"config":{"command":"codex-mcp"}}"#,
    );
    let definitions = resolve_effective_mcp_definitions(&OverlayRoots {
        global: root.to_path_buf(),
        workspace: None,
        project: root.join("empty-project"),
    })
    .unwrap();
    let ledger = MemoryProjectionLedger::default();

    let plan = build_mcp_projection_plan(
        &mcp_request(root, vec![PlatformId::Cursor]),
        &definitions,
        &PlannerContext::new(&ledger),
    )
    .unwrap();

    assert_eq!(plan.actions.len(), 1);
    let action = &plan.actions[0];
    assert!(matches!(
        action.kind,
        ProjectionActionKind::UpsertGeneratedBatch
    ));
    assert!(
        action.members.is_empty(),
        "MCP uses its dedicated intent list"
    );
    assert_eq!(action.mcp_members.len(), 1);
    assert_eq!(action.mcp_members[0].name, "catalog");
    assert_eq!(action.mcp_members[0].entry_key, "mcpServers.catalog");
    assert_eq!(action.mcp_members[0].secret_keys, vec!["CATALOG_TOKEN"]);

    let rendered = serde_json::to_string(&plan).unwrap();
    assert!(!rendered.contains("catalog-mcp"));
    assert!(!rendered.contains("disabled-mcp"));
    assert!(!rendered.contains("codex-mcp"));
}

#[test]
fn mcp_planner_can_add_a_missing_managed_entry_without_claiming_foreign_entries() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{"targets":["cursor"],"config":{"command":"catalog-mcp"}}"#,
    );
    let definitions = resolve_effective_mcp_definitions(&OverlayRoots {
        global: root.to_path_buf(),
        workspace: None,
        project: root.join("empty-project"),
    })
    .unwrap();
    let request = mcp_request(root, vec![PlatformId::Cursor]);
    let target = request.deploy_base.join(".cursor/mcp.json");
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    fs::write(
        target.as_std_path(),
        r#"{"mcpServers":{"foreign":{"command":"external"}},"userField":true}"#,
    )
    .unwrap();

    let plan = build_mcp_projection_plan(
        &request,
        &definitions,
        &PlannerContext::new(&MemoryProjectionLedger::default()),
    )
    .unwrap();

    assert!(matches!(
        plan.actions[0].kind,
        ProjectionActionKind::UpsertGeneratedBatch
    ));
    assert_eq!(plan.actions[0].reason_code, "generated_entries_missing");
    assert_eq!(plan.actions[0].mcp_members[0].name, "catalog");
}

#[test]
fn mcp_apply_updates_only_the_planned_cursor_entry_and_records_entry_ownership() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{"targets":["cursor"],"config":{"command":"catalog-mcp"}}"#,
    );
    let definitions = resolve_effective_mcp_definitions(&OverlayRoots {
        global: root.to_path_buf(),
        workspace: None,
        project: root.join("empty-project"),
    })
    .unwrap();
    let request = mcp_request(root, vec![PlatformId::Cursor]);
    let target = request.deploy_base.join(".cursor/mcp.json");
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    fs::write(
        target.as_std_path(),
        r#"{"mcpServers":{"foreign":{"command":"external"}},"userField":true}"#,
    )
    .unwrap();
    let ledger = MemoryProjectionLedger::default();
    let plan =
        build_mcp_projection_plan(&request, &definitions, &PlannerContext::new(&ledger)).unwrap();

    apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    )
    .unwrap();

    let rendered: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(target.as_std_path()).unwrap()).unwrap();
    assert_eq!(rendered["userField"], true);
    assert_eq!(rendered["mcpServers"]["foreign"]["command"], "external");
    assert_eq!(rendered["mcpServers"]["catalog"]["command"], "catalog-mcp");

    let record = ledger
        .get(&plan.actions[0].mcp_members[0].id)
        .unwrap()
        .unwrap();
    assert_eq!(record.entry_key.as_deref(), Some("mcpServers.catalog"));
    assert!(record.entry_fingerprint.is_some());
}

#[test]
fn mcp_retract_removes_only_the_ledger_proven_entry_and_preserves_foreign_content() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{"targets":["cursor"],"config":{"command":"catalog-mcp"}}"#,
    );
    let definitions = resolve_effective_mcp_definitions(&OverlayRoots {
        global: root.to_path_buf(),
        workspace: None,
        project: root.join("empty-project"),
    })
    .unwrap();
    let request = mcp_request(root, vec![PlatformId::Cursor]);
    let target = request.deploy_base.join(".cursor/mcp.json");
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    fs::write(
        target.as_std_path(),
        r#"{"mcpServers":{"foreign":{"command":"external"}},"userField":true}"#,
    )
    .unwrap();
    let ledger = MemoryProjectionLedger::default();
    let install_plan =
        build_mcp_projection_plan(&request, &definitions, &PlannerContext::new(&ledger)).unwrap();
    apply_projection_plan(
        &install_plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&install_plan),
    )
    .unwrap();

    let mut retract_request = request.clone();
    retract_request.operation = ProjectionOperation::Retract;
    let retract_plan = build_mcp_projection_plan(
        &retract_request,
        &definitions,
        &PlannerContext::new(&ledger),
    )
    .unwrap();
    assert!(matches!(
        retract_plan.actions[0].kind,
        ProjectionActionKind::RemoveGeneratedEntries
    ));

    let report = apply_projection_plan(
        &retract_plan,
        &ExecutorContext::new(
            &ledger,
            retract_request.deploy_base.clone(),
            root.join("retract-backups"),
        ),
        ApplyOptions::for_plan(&retract_plan),
    )
    .unwrap();

    assert_eq!(report.changed, 1);
    let rendered: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(target.as_std_path()).unwrap()).unwrap();
    assert_eq!(rendered["userField"], true);
    assert_eq!(rendered["mcpServers"]["foreign"]["command"], "external");
    assert!(rendered["mcpServers"].get("catalog").is_none());
    assert!(ledger
        .get(&retract_plan.actions[0].mcp_members[0].id)
        .unwrap()
        .is_none());
}

#[test]
fn mcp_retract_keeps_ledger_owned_entries_addressable_after_disable_or_target_removal() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{"enabled":true,"targets":["cursor"],"config":{"command":"catalog-mcp"}}"#,
    );
    let initial_definitions = resolve_effective_mcp_definitions(&OverlayRoots {
        global: root.to_path_buf(),
        workspace: None,
        project: root.join("empty-project"),
    })
    .unwrap();
    let request = mcp_request(root, vec![PlatformId::Cursor]);
    fs::create_dir_all(request.deploy_base.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let install_plan = build_mcp_projection_plan(
        &request,
        &initial_definitions,
        &PlannerContext::new(&ledger),
    )
    .unwrap();
    apply_projection_plan(
        &install_plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&install_plan),
    )
    .unwrap();

    let mut retract_request = request.clone();
    retract_request.operation = ProjectionOperation::Retract;
    for replacement in [
        r#"{"enabled":false,"targets":["cursor"],"config":{"command":"catalog-mcp"}}"#,
        r#"{"enabled":true,"targets":["codex"],"config":{"command":"catalog-mcp"}}"#,
    ] {
        write_server(root, "catalog", replacement);
        let definitions = resolve_effective_mcp_definitions(&OverlayRoots {
            global: root.to_path_buf(),
            workspace: None,
            project: root.join("empty-project"),
        })
        .unwrap();
        let plan = build_mcp_projection_plan(
            &retract_request,
            &definitions,
            &PlannerContext::new(&ledger),
        )
        .unwrap();

        assert_eq!(plan.actions.len(), 1);
        assert!(matches!(
            plan.actions[0].kind,
            ProjectionActionKind::RemoveGeneratedEntries
        ));
    }
}

#[test]
fn mcp_retract_refuses_a_stale_entry_fingerprint_without_writing() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{"targets":["cursor"],"config":{"command":"catalog-mcp"}}"#,
    );
    let definitions = resolve_effective_mcp_definitions(&OverlayRoots {
        global: root.to_path_buf(),
        workspace: None,
        project: root.join("empty-project"),
    })
    .unwrap();
    let request = mcp_request(root, vec![PlatformId::Cursor]);
    let target = request.deploy_base.join(".cursor/mcp.json");
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    fs::write(
        target.as_std_path(),
        r#"{"mcpServers":{},"userField":true}"#,
    )
    .unwrap();
    let ledger = MemoryProjectionLedger::default();
    let install_plan =
        build_mcp_projection_plan(&request, &definitions, &PlannerContext::new(&ledger)).unwrap();
    apply_projection_plan(
        &install_plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&install_plan),
    )
    .unwrap();
    let stale = r#"{"mcpServers":{"catalog":{"command":"changed-by-user"}},"userField":true}"#;
    fs::write(target.as_std_path(), stale).unwrap();

    let mut retract_request = request.clone();
    retract_request.operation = ProjectionOperation::Retract;
    let plan = build_mcp_projection_plan(
        &retract_request,
        &definitions,
        &PlannerContext::new(&ledger),
    )
    .unwrap();
    assert!(matches!(
        plan.actions[0].kind,
        ProjectionActionKind::ReportOnly
    ));
    assert_eq!(plan.actions[0].reason_code, "generated_target_drifted");

    let error = apply_projection_plan(
        &plan,
        &ExecutorContext::new(
            &ledger,
            request.deploy_base.clone(),
            root.join("retract-backups"),
        ),
        ApplyOptions::for_plan(&plan),
    )
    .unwrap_err();
    assert_eq!(
        error
            .report
            .failure
            .as_ref()
            .map(|failure| failure.code.as_str()),
        Some("blocking_conflict")
    );
    assert_eq!(fs::read_to_string(target.as_std_path()).unwrap(), stale);
}

#[test]
fn mcp_retract_refuses_a_legacy_record_without_entry_fingerprint_without_writing() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{"targets":["cursor"],"config":{"command":"catalog-mcp"}}"#,
    );
    let definitions = resolve_effective_mcp_definitions(&OverlayRoots {
        global: root.to_path_buf(),
        workspace: None,
        project: root.join("empty-project"),
    })
    .unwrap();
    let request = mcp_request(root, vec![PlatformId::Cursor]);
    let target = request.deploy_base.join(".cursor/mcp.json");
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    fs::write(
        target.as_std_path(),
        r#"{"mcpServers":{},"userField":true}"#,
    )
    .unwrap();
    let ledger = MemoryProjectionLedger::default();
    let install_plan =
        build_mcp_projection_plan(&request, &definitions, &PlannerContext::new(&ledger)).unwrap();
    apply_projection_plan(
        &install_plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&install_plan),
    )
    .unwrap();
    let id = install_plan.actions[0].mcp_members[0].id.clone();
    let mut legacy = ledger.get(&id).unwrap().unwrap();
    legacy.entry_fingerprint = None;
    ledger
        .apply_batch(&[LedgerMutation::Upsert(legacy)])
        .unwrap();
    let before = fs::read_to_string(target.as_std_path()).unwrap();

    let mut retract_request = request.clone();
    retract_request.operation = ProjectionOperation::Retract;
    let plan = build_mcp_projection_plan(
        &retract_request,
        &definitions,
        &PlannerContext::new(&ledger),
    )
    .unwrap();
    assert!(matches!(
        plan.actions[0].kind,
        ProjectionActionKind::ReportOnly
    ));
    assert_eq!(plan.actions[0].reason_code, "generated_ownership_unproven");

    let error = apply_projection_plan(
        &plan,
        &ExecutorContext::new(
            &ledger,
            request.deploy_base.clone(),
            root.join("retract-backups"),
        ),
        ApplyOptions::for_plan(&plan),
    )
    .unwrap_err();
    assert_eq!(
        error
            .report
            .failure
            .as_ref()
            .map(|failure| failure.code.as_str()),
        Some("blocking_conflict")
    );
    assert_eq!(fs::read_to_string(target.as_std_path()).unwrap(), before);
}

#[test]
fn mcp_retract_restores_the_original_container_when_ledger_commit_fails() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{"targets":["cursor"],"config":{"command":"catalog-mcp"}}"#,
    );
    let definitions = resolve_effective_mcp_definitions(&OverlayRoots {
        global: root.to_path_buf(),
        workspace: None,
        project: root.join("empty-project"),
    })
    .unwrap();
    let request = mcp_request(root, vec![PlatformId::Cursor]);
    let target = request.deploy_base.join(".cursor/mcp.json");
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    fs::write(
        target.as_std_path(),
        r#"{"mcpServers":{"foreign":{"command":"external"}},"userField":true}"#,
    )
    .unwrap();
    let planning_ledger = MemoryProjectionLedger::default();
    let install_plan = build_mcp_projection_plan(
        &request,
        &definitions,
        &PlannerContext::new(&planning_ledger),
    )
    .unwrap();
    apply_projection_plan(
        &install_plan,
        &ExecutorContext::new(
            &planning_ledger,
            request.deploy_base.clone(),
            root.join("backups"),
        ),
        ApplyOptions::for_plan(&install_plan),
    )
    .unwrap();
    let original = fs::read_to_string(target.as_std_path()).unwrap();

    let mut retract_request = request.clone();
    retract_request.operation = ProjectionOperation::Retract;
    let plan = build_mcp_projection_plan(
        &retract_request,
        &definitions,
        &PlannerContext::new(&planning_ledger),
    )
    .unwrap();
    let error = apply_projection_plan(
        &plan,
        &ExecutorContext::new(
            &FailingMcpLedger,
            retract_request.deploy_base.clone(),
            root.join("retract-backups"),
        ),
        ApplyOptions::for_plan(&plan),
    )
    .unwrap_err();

    assert_eq!(
        error
            .report
            .failure
            .as_ref()
            .map(|failure| failure.code.as_str()),
        Some("ledger_apply_failed")
    );
    assert_eq!(fs::read_to_string(target.as_std_path()).unwrap(), original);
}

#[test]
fn mcp_apply_refuses_a_source_that_changed_after_planning_before_writing() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{"targets":["cursor"],"config":{"command":"catalog-mcp"}}"#,
    );
    let definitions = resolve_effective_mcp_definitions(&OverlayRoots {
        global: root.to_path_buf(),
        workspace: None,
        project: root.join("empty-project"),
    })
    .unwrap();
    let request = mcp_request(root, vec![PlatformId::Cursor]);
    let target = request.deploy_base.join(".cursor/mcp.json");
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    fs::write(
        target.as_std_path(),
        r#"{"mcpServers":{"foreign":{"command":"external"}},"userField":true}"#,
    )
    .unwrap();
    let ledger = MemoryProjectionLedger::default();
    let plan =
        build_mcp_projection_plan(&request, &definitions, &PlannerContext::new(&ledger)).unwrap();
    write_server(
        root,
        "catalog",
        r#"{"targets":["cursor"],"config":{"command":"changed-after-plan"}}"#,
    );

    let error = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    )
    .unwrap_err();

    assert!(error.to_string().contains("canonical source changed"));
    assert_eq!(
        error
            .report
            .failure
            .as_ref()
            .map(|failure| failure.code.as_str()),
        Some("action_apply_failed")
    );
    assert_eq!(
        fs::read_to_string(target.as_std_path()).unwrap(),
        r#"{"mcpServers":{"foreign":{"command":"external"}},"userField":true}"#
    );
}

#[test]
fn mcp_apply_uses_the_mcp_renderer_for_each_supported_platform_container() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{"targets":["cursor","codex","claude","hermes"],"config":{"command":"catalog-mcp"}}"#,
    );
    let definitions = resolve_effective_mcp_definitions(&OverlayRoots {
        global: root.to_path_buf(),
        workspace: None,
        project: root.join("empty-project"),
    })
    .unwrap();
    let request = ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "user:/mcp-fixture".to_owned(),
        scope: DeploymentScope::User,
        deploy_base: root.join("deploy"),
        assets: Vec::new(),
        platforms: vec![
            PlatformId::Cursor,
            PlatformId::Codex,
            PlatformId::Claude,
            PlatformId::Hermes,
        ],
    };
    let cursor = request.deploy_base.join(".cursor/mcp.json");
    let codex = request.deploy_base.join(".codex/config.toml");
    let claude = request.deploy_base.join(".claude.json");
    let hermes = request.deploy_base.join(".hermes/config.yaml");
    for (path, content) in [
        (
            &cursor,
            r#"{"mcpServers":{"foreign":{"command":"cursor-foreign"}},"keep":true}"#,
        ),
        (
            &codex,
            "[mcp_servers.foreign]\ncommand = \"codex-foreign\"\n[keep]\nvalue = true\n",
        ),
        (
            &claude,
            r#"{"mcpServers":{"foreign":{"command":"claude-foreign"}},"projects":{}}"#,
        ),
        (
            &hermes,
            "provider: openai\nmcp_servers:\n  foreign:\n    command: hermes-foreign\n",
        ),
    ] {
        fs::create_dir_all(path.parent().unwrap().as_std_path()).unwrap();
        fs::write(path.as_std_path(), content).unwrap();
    }
    let ledger = MemoryProjectionLedger::default();
    let plan =
        build_mcp_projection_plan(&request, &definitions, &PlannerContext::new(&ledger)).unwrap();

    apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    )
    .unwrap();

    let cursor: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(cursor).unwrap()).unwrap();
    let claude: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(claude).unwrap()).unwrap();
    let codex: toml::Value = fs::read_to_string(codex).unwrap().parse().unwrap();
    let hermes: serde_yaml::Value =
        serde_yaml::from_str(&fs::read_to_string(hermes).unwrap()).unwrap();
    assert_eq!(cursor["mcpServers"]["foreign"]["command"], "cursor-foreign");
    assert_eq!(claude["mcpServers"]["foreign"]["command"], "claude-foreign");
    assert_eq!(
        codex["mcp_servers"]["foreign"]["command"].as_str(),
        Some("codex-foreign")
    );
    assert_eq!(
        hermes["mcp_servers"]["foreign"]["command"],
        serde_yaml::Value::String("hermes-foreign".to_owned())
    );
    assert_eq!(cursor["mcpServers"]["catalog"]["command"], "catalog-mcp");
    assert_eq!(claude["mcpServers"]["catalog"]["command"], "catalog-mcp");
    assert_eq!(
        codex["mcp_servers"]["catalog"]["command"].as_str(),
        Some("catalog-mcp")
    );
    assert_eq!(
        hermes["mcp_servers"]["catalog"]["command"],
        serde_yaml::Value::String("catalog-mcp".to_owned())
    );
}

#[test]
fn mcp_apply_hydrates_declared_secrets_only_through_the_injected_provider() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{"targets":["cursor"],"config":{"command":"catalog-mcp","env":{"TOKEN":"${CATALOG_TOKEN}"}}}"#,
    );
    let definitions = resolve_effective_mcp_definitions(&OverlayRoots {
        global: root.to_path_buf(),
        workspace: None,
        project: root.join("empty-project"),
    })
    .unwrap();
    let request = mcp_request(root, vec![PlatformId::Cursor]);
    fs::create_dir_all(request.deploy_base.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let plan =
        build_mcp_projection_plan(&request, &definitions, &PlannerContext::new(&ledger)).unwrap();
    assert!(!serde_json::to_string(&plan)
        .unwrap()
        .contains("test-secret-value"));

    let missing_provider = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    )
    .unwrap_err();
    assert!(missing_provider
        .to_string()
        .contains("caller-provided secret resolver"));

    let report = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups"))
            .with_mcp_secret_provider(&StaticSecretProvider),
        ApplyOptions::for_plan(&plan),
    )
    .unwrap();
    assert_eq!(report.changed, 1);
    let rendered: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(request.deploy_base.join(".cursor/mcp.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        rendered["mcpServers"]["catalog"]["env"]["TOKEN"],
        "test-secret-value"
    );
}

#[test]
fn mcp_apply_skips_only_members_with_missing_secret_keys_in_a_shared_container() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "public-catalog",
        r#"{"targets":["cursor"],"config":{"command":"public-catalog-mcp","env":{"TOKEN":"${CATALOG_TOKEN}"}}}"#,
    );
    write_server(
        root,
        "private-catalog",
        r#"{"targets":["cursor"],"config":{"command":"private-catalog-mcp","env":{"TOKEN":"${MISSING_TOKEN}"}}}"#,
    );
    let definitions = resolve_effective_mcp_definitions(&OverlayRoots {
        global: root.to_path_buf(),
        workspace: None,
        project: root.join("empty-project"),
    })
    .unwrap();
    let request = mcp_request(root, vec![PlatformId::Cursor]);
    let target = request.deploy_base.join(".cursor/mcp.json");
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    fs::write(
        target.as_std_path(),
        r#"{"mcpServers":{"foreign":{"command":"external"}},"userField":true}"#,
    )
    .unwrap();
    let ledger = MemoryProjectionLedger::default();
    let plan = build_mcp_projection_plan(
        &request,
        &definitions,
        &PlannerContext::new(&ledger).with_mcp_secret_availability(&MissingSecretAvailability),
    )
    .unwrap();
    let planned_skip = plan
        .actions
        .iter()
        .find(|action| action.reason_code == "mcp_missing_secret_keys")
        .unwrap();
    assert_eq!(planned_skip.state.as_deref(), Some("skipped"));
    assert_eq!(planned_skip.mcp_members[0].name, "private-catalog");
    assert_eq!(
        planned_skip.mcp_members[0].missing_secret_keys,
        vec!["MISSING_TOKEN"]
    );
    let serialized_plan = serde_json::to_string(&plan).unwrap();
    assert!(serialized_plan.contains("MISSING_TOKEN"));
    assert!(!serialized_plan.contains("test-secret-value"));

    let report = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups"))
            .with_mcp_secret_provider(&StaticSecretProvider),
        ApplyOptions::for_plan(&plan),
    )
    .unwrap();
    assert_eq!(report.changed, 1);
    assert_eq!(report.skipped, 1);

    let rendered: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(target.as_std_path()).unwrap()).unwrap();
    assert_eq!(rendered["userField"], true);
    assert_eq!(rendered["mcpServers"]["foreign"]["command"], "external");
    assert_eq!(
        rendered["mcpServers"]["public-catalog"]["command"],
        "public-catalog-mcp"
    );
    assert_eq!(
        rendered["mcpServers"]["public-catalog"]["env"]["TOKEN"],
        "test-secret-value"
    );
    assert!(rendered["mcpServers"].get("private-catalog").is_none());
    let public_member = plan
        .actions
        .iter()
        .flat_map(|action| action.mcp_members.iter())
        .find(|member| member.name == "public-catalog")
        .unwrap();
    let private_member = plan
        .actions
        .iter()
        .flat_map(|action| action.mcp_members.iter())
        .find(|member| member.name == "private-catalog")
        .unwrap();
    assert!(ledger.get(&public_member.id).unwrap().is_some());
    assert!(ledger.get(&private_member.id).unwrap().is_none());
    assert_eq!(report.mcp_skipped_members.len(), 1);
    assert_eq!(
        report.mcp_skipped_members[0].entry_key,
        "mcpServers.private-catalog"
    );
    assert_eq!(
        report.mcp_skipped_members[0].missing_secret_keys,
        vec!["MISSING_TOKEN"]
    );
    let serialized = serde_json::to_string(&report).unwrap();
    assert!(serialized.contains("MISSING_TOKEN"));
    assert!(!serialized.contains("test-secret-value"));
}

#[test]
fn mcp_apply_restores_the_original_container_when_ledger_commit_fails() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{"targets":["cursor"],"config":{"command":"catalog-mcp"}}"#,
    );
    let definitions = resolve_effective_mcp_definitions(&OverlayRoots {
        global: root.to_path_buf(),
        workspace: None,
        project: root.join("empty-project"),
    })
    .unwrap();
    let request = mcp_request(root, vec![PlatformId::Cursor]);
    let target = request.deploy_base.join(".cursor/mcp.json");
    let original = r#"{"mcpServers":{"foreign":{"command":"external"}},"userField":true}"#;
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    fs::write(target.as_std_path(), original).unwrap();
    let planning_ledger = MemoryProjectionLedger::default();
    let plan = build_mcp_projection_plan(
        &request,
        &definitions,
        &PlannerContext::new(&planning_ledger),
    )
    .unwrap();

    let error = apply_projection_plan(
        &plan,
        &ExecutorContext::new(
            &FailingMcpLedger,
            request.deploy_base.clone(),
            root.join("backups"),
        ),
        ApplyOptions::for_plan(&plan),
    )
    .unwrap_err();

    assert_eq!(
        error
            .report
            .failure
            .as_ref()
            .map(|failure| failure.code.as_str()),
        Some("ledger_apply_failed")
    );
    assert_eq!(fs::read_to_string(target.as_std_path()).unwrap(), original);
}

#[test]
fn mcp_entry_ownership_requires_entry_proof_but_ignores_unrelated_container_edits() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{"targets":["cursor"],"config":{"command":"catalog-mcp"}}"#,
    );
    let definitions = resolve_effective_mcp_definitions(&OverlayRoots {
        global: root.to_path_buf(),
        workspace: None,
        project: root.join("empty-project"),
    })
    .unwrap();
    let request = mcp_request(root, vec![PlatformId::Cursor]);
    let target = request.deploy_base.join(".cursor/mcp.json");
    fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
    fs::write(
        target.as_std_path(),
        r#"{"mcpServers":{"catalog":{"command":"catalog-mcp"},"foreign":{"command":"one"}}}"#,
    )
    .unwrap();
    let entry_fingerprint =
        inspect_cursor_mcp_entries(&fs::read_to_string(target.as_std_path()).unwrap())
            .unwrap()
            .into_iter()
            .find(|entry| entry.name == "catalog")
            .unwrap()
            .digest;
    let source = &definitions[0].source;
    let ledger = MemoryProjectionLedger::default();
    ledger
        .apply_batch(&[LedgerMutation::Upsert(ProjectionRecord {
            id: ProjectionId {
                scope_key: request.scope_key.clone(),
                kind: ai_config_core::model::AssetKind::Mcp,
                name: "catalog".to_owned(),
                surface: ProjectionSurface::Platform(PlatformId::Cursor),
            },
            mode: ProjectionMode::GeneratedJson,
            source_path: source.absolute_path.clone(),
            target_path: target.clone(),
            entry_key: Some("mcpServers.catalog".to_owned()),
            source_fingerprint: source.fingerprint.clone(),
            entry_fingerprint: Some(entry_fingerprint),
            target_fingerprint: "intentionally-stale-container-digest".to_owned(),
            applied_at: chrono::Utc::now(),
        })])
        .unwrap();

    let managed =
        build_mcp_projection_plan(&request, &definitions, &PlannerContext::new(&ledger)).unwrap();
    assert!(matches!(
        managed.actions[0].kind,
        ProjectionActionKind::Noop
    ));

    fs::write(
        target.as_std_path(),
        r#"{"mcpServers":{"catalog":{"command":"catalog-mcp"},"foreign":{"command":"two"}},"userField":true}"#,
    )
    .unwrap();
    let unchanged_entry =
        build_mcp_projection_plan(&request, &definitions, &PlannerContext::new(&ledger)).unwrap();
    assert!(matches!(
        unchanged_entry.actions[0].kind,
        ProjectionActionKind::Noop
    ));

    let legacy = MemoryProjectionLedger::default();
    legacy
        .apply_batch(&[LedgerMutation::Upsert(ProjectionRecord {
            id: ProjectionId {
                scope_key: request.scope_key.clone(),
                kind: ai_config_core::model::AssetKind::Mcp,
                name: "catalog".to_owned(),
                surface: ProjectionSurface::Platform(PlatformId::Cursor),
            },
            mode: ProjectionMode::GeneratedJson,
            source_path: source.absolute_path.clone(),
            target_path: target,
            entry_key: Some("mcpServers.catalog".to_owned()),
            source_fingerprint: source.fingerprint.clone(),
            entry_fingerprint: None,
            target_fingerprint: "legacy-container-digest".to_owned(),
            applied_at: chrono::Utc::now(),
        })])
        .unwrap();
    let refused =
        build_mcp_projection_plan(&request, &definitions, &PlannerContext::new(&legacy)).unwrap();
    assert!(matches!(
        refused.actions[0].kind,
        ProjectionActionKind::ReportOnly
    ));
    assert_eq!(
        refused.actions[0].reason_code,
        "generated_ownership_unproven"
    );
}

#[test]
fn load_mcp_definitions_reads_enabled_per_server_sources_with_secret_references() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{
          "transport": "stdio",
          "enabled": true,
          "config": {
            "command": "catalog-mcp",
            "env": {"CATALOG_TOKEN": "${CATALOG_TOKEN}"}
          }
        }"#,
    );

    let definitions = load_mcp_definitions(root).unwrap();

    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].server.name, "catalog");
    assert!(definitions[0].server.enabled);
    assert_eq!(definitions[0].server.secret_keys, vec!["CATALOG_TOKEN"]);
}

#[test]
fn load_mcp_definitions_rejects_literal_env_values_without_echoing_them() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{
          "transport": "stdio",
          "config": {
            "command": "catalog-mcp",
            "env": {"CATALOG_TOKEN": "do-not-log-this-token"}
          }
        }"#,
    );

    let error = load_mcp_definitions(root).unwrap_err().to_string();

    assert!(error.contains("CATALOG_TOKEN"));
    assert!(!error.contains("do-not-log-this-token"));
}

#[test]
fn load_mcp_definitions_accepts_a_valid_secret_reference_with_digits() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{
          "transport": "stdio",
          "config": {
            "command": "catalog-mcp",
            "env": {"CATALOG_TOKEN": "${CATALOG_TOKEN_2}"}
          }
        }"#,
    );

    let definitions = load_mcp_definitions(root).unwrap();

    assert_eq!(definitions[0].server.secret_keys, vec!["CATALOG_TOKEN_2"]);
}

#[test]
fn mcp_definition_filters_by_explicit_platform_targets_and_enabled_state() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{
          "enabled": false,
          "targets": ["cursor", "codex"],
          "config": {"command": "catalog-mcp"}
        }"#,
    );

    let definition = load_mcp_definitions(root).unwrap().pop().unwrap();

    assert!(!definition.enabled_for(PlatformId::Cursor));
    assert!(!definition.enabled_for(PlatformId::Codex));
    assert!(!definition.enabled_for(PlatformId::Claude));
    assert_eq!(
        definition.targets,
        vec![PlatformId::Cursor, PlatformId::Codex]
    );
}

#[test]
fn load_mcp_definitions_rejects_literal_headers_and_url_userinfo_without_echoing_them() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{
          "transport": "http",
          "config": {
            "url": "https://alice:do-not-log-url-secret@example.test/mcp",
            "headers": {"Authorization": "do-not-log-header-secret"}
          }
        }"#,
    );

    let error = load_mcp_definitions(root).unwrap_err().to_string();

    assert!(error.contains("Authorization"));
    assert!(!error.contains("do-not-log-header-secret"));
    assert!(!error.contains("do-not-log-url-secret"));
}

#[test]
fn load_mcp_definitions_rejects_url_userinfo_without_echoing_it() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{
          "transport": "http",
          "config": {"url": "https://alice:do-not-log-url-secret@example.test/mcp"}
        }"#,
    );

    let error = load_mcp_definitions(root).unwrap_err().to_string();

    assert!(error.contains("userinfo"));
    assert!(!error.contains("do-not-log-url-secret"));
}

#[test]
fn load_mcp_definitions_rejects_suspected_credential_arguments_without_echoing_them() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{
          "transport": "stdio",
          "config": {
            "command": "catalog-mcp",
            "args": ["--token", "do-not-log-argument-secret"]
          }
        }"#,
    );

    let error = load_mcp_definitions(root).unwrap_err().to_string();

    assert!(error.contains("credential"));
    assert!(!error.contains("do-not-log-argument-secret"));
}

#[test]
fn resolve_effective_mcp_definitions_overrides_the_whole_server_entry_by_layer() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let global = root.join("global");
    let workspace = root.join("workspace");
    let project = root.join("project");
    write_server(
        &global,
        "catalog",
        r#"{"targets":["cursor"],"config":{"command":"global-catalog"}}"#,
    );
    write_server(
        &workspace,
        "search",
        r#"{"targets":["codex"],"config":{"command":"workspace-search"}}"#,
    );
    write_server(
        &project,
        "catalog",
        r#"{"targets":["claude"],"config":{"command":"project-catalog"}}"#,
    );

    let resolved = resolve_effective_mcp_definitions(&OverlayRoots {
        global: global.clone(),
        workspace: Some(workspace.clone()),
        project: project.clone(),
    })
    .unwrap();

    assert_eq!(resolved.len(), 2);
    assert_eq!(resolved[0].definition.server.name, "catalog");
    assert_eq!(resolved[0].source.layer, SourceLayer::Project);
    assert_eq!(
        resolved[0].source.absolute_path,
        project.join("mcp/servers/catalog.json")
    );
    assert_eq!(resolved[0].definition.targets, vec![PlatformId::Claude]);
    assert_eq!(resolved[1].definition.server.name, "search");
    assert_eq!(resolved[1].source.layer, SourceLayer::Workspace);
}

#[test]
fn cursor_renderer_merges_two_owned_servers_once_and_preserves_foreign_and_top_level_fields() {
    let existing = r#"{
      "mcpServers": {"foreign": {"command": "foreign-mcp"}},
      "unknownTopLevel": {"keep": true}
    }"#;
    let intents = vec![
        JsonServerIntent::new("alpha", serde_json::json!({"command": "alpha-mcp"})),
        JsonServerIntent::new("beta", serde_json::json!({"url": "https://beta.test/mcp"})),
    ];

    let rendered = render_cursor_mcp_json(existing, &intents, &[]).unwrap();
    let rendered: serde_json::Value = serde_json::from_str(&rendered).unwrap();

    assert_eq!(rendered["unknownTopLevel"]["keep"], true);
    assert_eq!(rendered["mcpServers"]["foreign"]["command"], "foreign-mcp");
    assert_eq!(rendered["mcpServers"]["alpha"]["command"], "alpha-mcp");
    assert_eq!(
        rendered["mcpServers"]["beta"]["url"],
        "https://beta.test/mcp"
    );
}

#[test]
fn cursor_renderer_removes_only_the_explicitly_owned_unchanged_server() {
    let existing = r#"{
      "mcpServers": {
        "owned": {"command": "owned-mcp"},
        "foreign": {"command": "foreign-mcp"}
      }
    }"#;

    let rendered = render_cursor_mcp_json(existing, &[], &["owned".to_owned()]).unwrap();
    let rendered: serde_json::Value = serde_json::from_str(&rendered).unwrap();

    assert!(rendered["mcpServers"].get("owned").is_none());
    assert_eq!(rendered["mcpServers"]["foreign"]["command"], "foreign-mcp");
}

#[test]
fn claude_renderer_preserves_projects_settings_and_foreign_servers() {
    let existing = r#"{
      "mcpServers": {"foreign": {"command": "foreign-mcp"}},
      "projects": {"/workspace": {"allowedTools": ["Read"]}},
      "settings": {"theme": "dark"}
    }"#;
    let intents = vec![ClaudeJsonServerIntent::new(
        "alpha",
        serde_json::json!({"command": "alpha-mcp"}),
    )];

    let rendered = render_claude_mcp_json(existing, &intents, &[]).unwrap();
    let rendered: serde_json::Value = serde_json::from_str(&rendered).unwrap();

    assert_eq!(
        rendered["projects"]["/workspace"]["allowedTools"][0],
        "Read"
    );
    assert_eq!(rendered["settings"]["theme"], "dark");
    assert_eq!(rendered["mcpServers"]["foreign"]["command"], "foreign-mcp");
    assert_eq!(rendered["mcpServers"]["alpha"]["command"], "alpha-mcp");
}

#[test]
fn codex_renderer_preserves_comments_unknown_tables_and_foreign_servers() {
    let existing = r#"# user comment
model = "gpt-5"

[features]
keep = true

[mcp_servers.foreign] # foreign comment
command = "foreign-mcp"
"#;
    let intents = vec![
        TomlServerIntent::new(
            "alpha",
            serde_json::json!({
                "command": "alpha-mcp",
                "args": ["--serve", "alpha"],
                "env": {"ALPHA_TOKEN": "${ALPHA_TOKEN}"}
            }),
        ),
        TomlServerIntent::new(
            "beta",
            serde_json::json!({
                "url": "https://beta.test/mcp",
                "headers": {"X-Trace": "trace"}
            }),
        ),
    ];

    let rendered = render_codex_mcp_toml(existing, &intents, &[]).unwrap();
    let parsed: toml::Value = toml::from_str(&rendered).unwrap();

    assert!(rendered.contains("# user comment"));
    assert!(rendered.contains("# foreign comment"));
    assert_eq!(parsed["model"].as_str(), Some("gpt-5"));
    assert_eq!(parsed["features"]["keep"].as_bool(), Some(true));
    assert_eq!(
        parsed["mcp_servers"]["foreign"]["command"].as_str(),
        Some("foreign-mcp")
    );
    assert_eq!(
        parsed["mcp_servers"]["alpha"]["command"].as_str(),
        Some("alpha-mcp")
    );
    assert_eq!(
        parsed["mcp_servers"]["alpha"]["env"]["ALPHA_TOKEN"].as_str(),
        Some("${ALPHA_TOKEN}")
    );
    assert_eq!(
        parsed["mcp_servers"]["beta"]["headers"]["X-Trace"].as_str(),
        Some("trace")
    );
}

#[test]
fn codex_renderer_removes_only_explicit_owned_server_and_rejects_non_table_container() {
    let existing = r#"mcp_servers = "not-a-table""#;
    assert!(render_codex_mcp_toml(existing, &[], &[]).is_err());

    let existing = r#"
[mcp_servers.owned]
command = "owned-mcp"

[mcp_servers.foreign]
command = "foreign-mcp"
"#;
    let rendered = render_codex_mcp_toml(existing, &[], &["owned".to_owned()]).unwrap();
    let parsed: toml::Value = toml::from_str(&rendered).unwrap();

    assert!(parsed["mcp_servers"].get("owned").is_none());
    assert_eq!(
        parsed["mcp_servers"]["foreign"]["command"].as_str(),
        Some("foreign-mcp")
    );
}

#[test]
fn hermes_renderer_preserves_provider_model_comments_and_foreign_server() {
    let existing = r#"# user configuration
provider: openai # preserve provider comment
model: gpt-5

mcp_servers: # managed entries only
  foreign: # external server comment
    command: foreign-mcp
    env:
      FOREIGN_MODE: preserve

skills:
  external_dirs:
    - /opt/skills
"#;
    let intents = vec![YamlServerIntent::new(
        "alpha",
        serde_json::json!({
            "command": "alpha-mcp",
            "args": ["--serve", "alpha"],
            "env": {"ALPHA_TOKEN": "${ALPHA_TOKEN}"}
        }),
    )];

    let rendered = render_hermes_mcp_yaml(existing, &intents, &[]).unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&rendered).unwrap();

    assert!(rendered.contains("# user configuration"));
    assert!(rendered.contains("# preserve provider comment"));
    assert!(rendered.contains("# managed entries only"));
    assert!(rendered.contains("# external server comment"));
    assert_eq!(parsed["provider"].as_str(), Some("openai"));
    assert_eq!(parsed["model"].as_str(), Some("gpt-5"));
    assert_eq!(
        parsed["mcp_servers"]["foreign"]["env"]["FOREIGN_MODE"].as_str(),
        Some("preserve")
    );
    assert_eq!(
        parsed["mcp_servers"]["alpha"]["command"].as_str(),
        Some("alpha-mcp")
    );
    assert_eq!(
        parsed["mcp_servers"]["alpha"]["args"][0].as_str(),
        Some("--serve")
    );
    assert_eq!(
        parsed["mcp_servers"]["alpha"]["env"]["ALPHA_TOKEN"].as_str(),
        Some("${ALPHA_TOKEN}")
    );
    assert_eq!(
        parsed["skills"]["external_dirs"][0].as_str(),
        Some("/opt/skills")
    );
}

#[test]
fn hermes_renderer_removes_only_explicit_owned_server() {
    let existing = r#"provider: openai
mcp_servers:
  owned:
    command: owned-mcp
  foreign: # leave this alone
    command: foreign-mcp
"#;

    let rendered = render_hermes_mcp_yaml(existing, &[], &["owned".to_owned()]).unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&rendered).unwrap();

    assert!(parsed["mcp_servers"].get("owned").is_none());
    assert_eq!(
        parsed["mcp_servers"]["foreign"]["command"].as_str(),
        Some("foreign-mcp")
    );
    assert!(rendered.contains("# leave this alone"));
}

#[test]
fn hermes_renderer_rejects_a_non_mapping_container_without_rendering() {
    let existing = "provider: openai\nmcp_servers: not-a-mapping\n";

    assert!(render_hermes_mcp_yaml(existing, &[], &[]).is_err());
}

#[test]
fn hermes_renderer_does_not_create_an_empty_container_for_a_retract_only_batch() {
    let existing = "provider: openai\nmodel: gpt-5\n";

    assert_eq!(
        render_hermes_mcp_yaml(existing, &[], &["missing-owned".to_owned()]).unwrap(),
        existing
    );
}

#[test]
fn mcp_entry_inspection_fingerprints_named_servers_across_all_four_containers() {
    let cursor = inspect_cursor_mcp_entries(
        r#"{"mcpServers":{"zeta":{"command":"zeta"},"alpha":{"command":"alpha"}}}"#,
    )
    .unwrap();
    let claude = inspect_claude_mcp_entries(
        r#"{"mcpServers":{"alpha":{"command":"alpha"}},"projects":{"/repo":{}}}"#,
    )
    .unwrap();
    let codex = inspect_codex_mcp_entries(
        r#"[mcp_servers.alpha]
command = "alpha"

[mcp_servers.zeta]
command = "zeta"
"#,
    )
    .unwrap();
    let hermes = inspect_hermes_mcp_entries(
        r#"provider: openai
mcp_servers:
  zeta:
    command: zeta
  alpha:
    command: alpha
"#,
    )
    .unwrap();

    assert_eq!(
        cursor
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "zeta"]
    );
    assert_eq!(cursor[0].digest, claude[0].digest);
    assert_eq!(cursor[0].digest, codex[0].digest);
    assert_eq!(cursor[0].digest, hermes[0].digest);
    assert_eq!(cursor[1].digest, codex[1].digest);
    assert_eq!(cursor[1].digest, hermes[1].digest);
}

#[test]
fn mcp_entry_inspection_digest_changes_only_for_the_named_server_configuration() {
    let before = inspect_cursor_mcp_entries(
        r#"{"mcpServers":{"alpha":{"command":"alpha","args":["one"]},"beta":{"command":"beta"}}}"#,
    )
    .unwrap();
    let after = inspect_cursor_mcp_entries(
        r#"{"mcpServers":{"beta":{"command":"beta"},"alpha":{"args":["two"],"command":"alpha"}},"unknown":true}"#,
    )
    .unwrap();

    assert_ne!(before[0].digest, after[0].digest);
    assert_eq!(before[1].digest, after[1].digest);
}

#[test]
fn mcp_entry_inspection_rejects_invalid_container_without_echoing_payload() {
    let sentinel = "do-not-log-entry-secret";

    for error in [
        inspect_cursor_mcp_entries(&format!("{{\"mcpServers\":\"{sentinel}\"}}")),
        inspect_claude_mcp_entries(&format!("{{\"mcpServers\":\"{sentinel}\"}}")),
        inspect_codex_mcp_entries(&format!("mcp_servers = \"{sentinel}\"")),
        inspect_hermes_mcp_entries(&format!("mcp_servers: {sentinel}")),
    ] {
        let error = error.unwrap_err().to_string();
        assert!(error.contains("mcp_servers") || error.contains("mcpServers"));
        assert!(!error.contains(sentinel));
    }
}

#[test]
fn mcp_entry_inspection_redacts_parser_and_entry_shape_failures() {
    let sentinel = "do-not-log-parser-secret";
    let parser_failures = [
        inspect_cursor_mcp_entries(&format!("{{\"mcpServers\":{{\"x\":\"{sentinel}")),
        inspect_claude_mcp_entries(&format!("{{\"mcpServers\":{{\"x\":\"{sentinel}")),
        inspect_codex_mcp_entries(&format!("[mcp_servers.x\ncommand = \"{sentinel}\"")),
        inspect_hermes_mcp_entries(&format!("mcp_servers: [{sentinel}")),
    ];
    let entry_shape_failures = [
        inspect_cursor_mcp_entries(&format!("{{\"mcpServers\":{{\"x\":\"{sentinel}\"}}}}")),
        inspect_claude_mcp_entries(&format!("{{\"mcpServers\":{{\"x\":\"{sentinel}\"}}}}")),
        inspect_codex_mcp_entries(&format!("mcp_servers = {{ x = \"{sentinel}\" }}")),
        inspect_hermes_mcp_entries(&format!("mcp_servers:\n  x: {sentinel}\n")),
    ];

    for result in parser_failures.into_iter().chain(entry_shape_failures) {
        let error = result.unwrap_err().to_string();
        assert!(!error.contains(sentinel));
    }
}
