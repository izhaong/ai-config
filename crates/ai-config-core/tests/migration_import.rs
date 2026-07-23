#![cfg(unix)]

use std::fs;

use ai_config_core::model::{AssetKind, PlatformId};
use ai_config_core::paths::SyncRoots;
use ai_config_core::projection::fingerprint::path_fingerprint;
use ai_config_core::projection::migration_action::{
    apply_import_plan, build_import_plan, import_request_for_sync_roots,
    rollback_import_transaction, ImportActionKind, ImportApplyOptions, ImportRequest,
    ImportSecretStatus, RollbackStatus,
};
use ai_config_core::projection::model::SourceLayer;
use camino::Utf8Path;
use tempfile::TempDir;

fn utf8(path: &std::path::Path) -> &Utf8Path {
    Utf8Path::from_path(path).expect("UTF-8 fixture path")
}

fn write(path: &Utf8Path, content: &str) {
    fs::create_dir_all(path.parent().expect("fixture parent").as_std_path()).unwrap();
    fs::write(path.as_std_path(), content).unwrap();
}

fn prompt_request(root: &Utf8Path, replace: bool) -> ImportRequest {
    ImportRequest {
        kind: AssetKind::Prompt,
        name: "AGENTS".to_owned(),
        source_platform: PlatformId::Codex,
        source_path: root.join("repo/AGENTS.md"),
        approved_source_root: root.join("repo"),
        destination_layer: SourceLayer::Project,
        destination_asset_root: root.join("repo/.ai-config"),
        replace,
    }
}

fn codex_mcp_request(root: &Utf8Path, name: &str, replace: bool) -> ImportRequest {
    ImportRequest {
        kind: AssetKind::Mcp,
        name: name.to_owned(),
        source_platform: PlatformId::Codex,
        source_path: root.join("repo/.codex/config.toml"),
        approved_source_root: root.join("repo/.codex"),
        destination_layer: SourceLayer::Project,
        destination_asset_root: root.join("repo/.ai-config"),
        replace,
    }
}

#[test]
fn scope_import_request_uses_only_the_current_platform_target_and_canonical_layer() {
    let temp = TempDir::new().unwrap();
    let root = utf8(temp.path());
    let roots = SyncRoots {
        repo_root: root.join("repo"),
        asset_root: root.join("repo/.ai-config"),
        global_default: root.join("global/.ai-config"),
        deploy_base: root.join("repo"),
    };

    let request = import_request_for_sync_roots(
        AssetKind::Skill,
        "review",
        PlatformId::Cursor,
        &roots,
        false,
    )
    .unwrap();

    assert_eq!(request.source_path, root.join("repo/.cursor/skills/review"));
    assert_eq!(
        request.approved_source_root,
        root.join("repo/.cursor/skills")
    );
    assert_eq!(request.destination_layer, SourceLayer::Project);
    assert_eq!(request.destination_asset_root, root.join("repo/.ai-config"));
}

#[test]
fn prompt_import_plan_is_deterministic_redacted_and_read_only() {
    let temp = TempDir::new().unwrap();
    let root = utf8(temp.path());
    let request = prompt_request(root, false);
    write(&request.source_path, "# Repository instructions\n");
    let before = fs::read(&request.source_path).unwrap();

    let first = build_import_plan(std::slice::from_ref(&request)).unwrap();
    let second = build_import_plan(std::slice::from_ref(&request)).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.actions.len(), 1);
    let action = &first.actions[0];
    assert_eq!(action.action, ImportActionKind::CreateSource);
    assert_eq!(
        action.destination_path,
        root.join("repo/.ai-config/prompts/AGENTS.md")
    );
    assert_eq!(action.destination_layer, SourceLayer::Project);
    assert_eq!(action.secret_preflight.status, ImportSecretStatus::Clear);
    assert!(action.secret_preflight.key_names.is_empty());
    assert_eq!(action.normalized_diff.change, "create");
    assert!(!action.action_id.is_empty());
    assert!(!first.plan_digest.is_empty());
    assert_eq!(fs::read(&request.source_path).unwrap(), before);
    assert!(!serde_json::to_string(&first)
        .unwrap()
        .contains("Repository instructions"));
}

#[test]
fn selected_prompt_import_creates_canonical_source_and_preserves_platform_original() {
    let temp = TempDir::new().unwrap();
    let root = utf8(temp.path());
    let request = prompt_request(root, false);
    write(&request.source_path, "# Repository instructions\n");
    let original = fs::read(&request.source_path).unwrap();
    let plan = build_import_plan(std::slice::from_ref(&request)).unwrap();
    let selected = plan.actions[0].action_id.clone();
    let transaction_root = root.join("transactions");

    let report = apply_import_plan(
        &plan,
        &ImportApplyOptions::new(&plan.plan_digest, [selected]),
        &transaction_root,
    )
    .unwrap();

    assert_eq!(report.applied, 1);
    assert_eq!(report.skipped, 0);
    assert!(report.transaction_id.is_some());
    assert_eq!(fs::read(&request.source_path).unwrap(), original);
    assert_eq!(
        fs::read(root.join("repo/.ai-config/prompts/AGENTS.md")).unwrap(),
        original
    );
    let transaction_id = report.transaction_id.unwrap();
    assert!(transaction_root
        .join(format!("{transaction_id}.json"))
        .is_file());
}

#[test]
fn import_requires_selection_and_rejects_stale_source_without_writing() {
    let temp = TempDir::new().unwrap();
    let root = utf8(temp.path());
    let request = prompt_request(root, false);
    write(&request.source_path, "reviewed\n");
    let plan = build_import_plan(std::slice::from_ref(&request)).unwrap();
    let destination = root.join("repo/.ai-config/prompts/AGENTS.md");

    let no_selection = apply_import_plan(
        &plan,
        &ImportApplyOptions::new(&plan.plan_digest, Vec::<String>::new()),
        &root.join("transactions"),
    );
    assert!(no_selection.is_err());
    assert!(!destination.exists());

    write(&request.source_path, "changed after review\n");
    let stale = apply_import_plan(
        &plan,
        &ImportApplyOptions::new(&plan.plan_digest, [plan.actions[0].action_id.clone()]),
        &root.join("transactions"),
    );
    assert!(stale.is_err());
    assert!(!destination.exists());
}

#[test]
fn replace_requires_explicit_plan_and_rollback_refuses_post_state_drift() {
    let temp = TempDir::new().unwrap();
    let root = utf8(temp.path());
    let request = prompt_request(root, false);
    write(&request.source_path, "imported\n");
    let destination = root.join("repo/.ai-config/prompts/AGENTS.md");
    write(&destination, "canonical\n");

    assert!(build_import_plan(std::slice::from_ref(&request)).is_err());
    let replacing = ImportRequest {
        replace: true,
        ..request
    };
    let plan = build_import_plan(&[replacing]).unwrap();
    assert_eq!(plan.actions[0].action, ImportActionKind::ReplaceSource);
    let report = apply_import_plan(
        &plan,
        &ImportApplyOptions::new(&plan.plan_digest, [plan.actions[0].action_id.clone()]),
        &root.join("transactions"),
    )
    .unwrap();
    let transaction_id = report.transaction_id.unwrap();
    assert_eq!(fs::read(&destination).unwrap(), b"imported\n");

    write(&destination, "user changed after import\n");
    let rollback = rollback_import_transaction(
        &root.join("transactions"),
        &transaction_id,
        &[root.join("repo/.ai-config")],
    )
    .unwrap();
    assert_eq!(rollback.status, RollbackStatus::Drifted);
    assert_eq!(
        fs::read(&destination).unwrap(),
        b"user changed after import\n"
    );
}

#[test]
fn mcp_literal_secret_preflight_blocks_apply_without_serializing_the_value() {
    let temp = TempDir::new().unwrap();
    let root = utf8(temp.path());
    let sentinel = "sk-import-SHOULD-NEVER-APPEAR-IN-PLAN";
    let source = root.join("repo/.cursor/mcp.json");
    write(
        &source,
        &format!(
            r#"{{"mcpServers":{{"demo":{{"command":"demo","env":{{"TOKEN":"{sentinel}"}}}}}}}}"#
        ),
    );
    let request = ImportRequest {
        kind: AssetKind::Mcp,
        name: "demo".to_owned(),
        source_platform: PlatformId::Cursor,
        source_path: source,
        approved_source_root: root.join("repo"),
        destination_layer: SourceLayer::Project,
        destination_asset_root: root.join("repo/.ai-config"),
        replace: false,
    };

    let plan = build_import_plan(&[request]).unwrap();
    assert_eq!(
        plan.actions[0].secret_preflight.status,
        ImportSecretStatus::Blocked
    );
    assert_eq!(plan.actions[0].secret_preflight.key_names, ["TOKEN"]);
    assert!(!serde_json::to_string(&plan).unwrap().contains(sentinel));
    assert!(apply_import_plan(
        &plan,
        &ImportApplyOptions::new(&plan.plan_digest, [plan.actions[0].action_id.clone()],),
        &root.join("transactions"),
    )
    .is_err());
    assert!(!root.join("repo/.ai-config/mcp/servers/demo.json").exists());
}

#[test]
fn codex_mcp_env_indirection_imports_without_secret_values_or_target_writes() {
    let temp = TempDir::new().unwrap();
    let root = utf8(temp.path());
    let source = root.join("repo/.codex/config.toml");
    write(
        &source,
        r#"# preserved comment
[mcp_servers.gitea]
type = "http"
url = "https://gitea.example/mcp"
bearer_token_env_var = "PROJECT_GITEA_TOKEN"
http_headers = { Accept = "application/json, text/event-stream", Content-Type = "application/json" }

[mcp_servers.jenkins]
url = "https://jenkins.example/mcp"
env_http_headers = { Authorization = "PROJECT_JENKINS_AUTHORIZATION" }
http_headers = { Accept = "application/json", Content-Type = "application/json" }
"#,
    );
    let before = fs::read(&source).unwrap();
    let requests = [
        codex_mcp_request(root, "gitea", false),
        codex_mcp_request(root, "jenkins", false),
    ];

    let plan = build_import_plan(&requests).unwrap();

    assert_eq!(plan.actions.len(), 2);
    assert!(plan
        .actions
        .iter()
        .all(|action| action.secret_preflight.status == ImportSecretStatus::Clear));
    let selected = plan
        .actions
        .iter()
        .map(|action| action.action_id.clone())
        .collect::<Vec<_>>();
    apply_import_plan(
        &plan,
        &ImportApplyOptions::new(&plan.plan_digest, selected),
        &root.join("transactions"),
    )
    .unwrap();

    assert_eq!(fs::read(&source).unwrap(), before);
    let gitea: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("repo/.ai-config/mcp/servers/gitea.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(gitea["targets"], serde_json::json!(["codex"]));
    assert_eq!(gitea["transport"], "http");
    assert_eq!(
        gitea["config"]["bearer_token_env_var"],
        "PROJECT_GITEA_TOKEN"
    );
    assert!(gitea["config"].get("type").is_none());
    let jenkins: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("repo/.ai-config/mcp/servers/jenkins.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        jenkins["config"]["env_http_headers"]["Authorization"],
        "PROJECT_JENKINS_AUTHORIZATION"
    );
}

#[test]
fn codex_mcp_literal_authorization_blocks_import_without_leaking_the_value() {
    let temp = TempDir::new().unwrap();
    let root = utf8(temp.path());
    let sentinel = "Bearer MUST-NOT-LEAK-CODEX-AUTH";
    write(
        &root.join("repo/.codex/config.toml"),
        &format!(
            r#"[mcp_servers.gitea]
url = "https://gitea.example/mcp"
http_headers = {{ Authorization = "{sentinel}" }}
"#
        ),
    );

    let plan = build_import_plan(&[codex_mcp_request(root, "gitea", false)]).unwrap();
    let serialized = serde_json::to_string(&plan).unwrap();

    assert_eq!(
        plan.actions[0].secret_preflight.status,
        ImportSecretStatus::Blocked
    );
    assert!(plan.actions[0]
        .secret_preflight
        .key_names
        .contains(&"Authorization".to_owned()));
    assert!(!serialized.contains(sentinel));
    assert!(apply_import_plan(
        &plan,
        &ImportApplyOptions::new(&plan.plan_digest, [plan.actions[0].action_id.clone()]),
        &root.join("transactions"),
    )
    .is_err());
    assert!(!root.join("repo/.ai-config/mcp/servers/gitea.json").exists());
}

#[test]
fn codex_mcp_missing_named_entry_is_read_only_error() {
    let temp = TempDir::new().unwrap();
    let root = utf8(temp.path());
    let source = root.join("repo/.codex/config.toml");
    write(&source, "[mcp_servers.present]\ncommand = \"present\"\n");
    let before = fs::read(&source).unwrap();

    let result = build_import_plan(&[codex_mcp_request(root, "missing", false)]);

    assert!(result.is_err());
    assert_eq!(fs::read(&source).unwrap(), before);
    assert!(!root
        .join("repo/.ai-config/mcp/servers/missing.json")
        .exists());
}

#[test]
fn second_action_validation_failure_rolls_back_prior_and_current_writes() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new().unwrap();
    let root = utf8(temp.path());
    let agent_source = root.join("repo/.cursor/agents/helper.md");
    write(
        &agent_source,
        "---\nname: helper\ndescription: helper\n---\nbody\n",
    );
    let mcp_source = root.join("repo/.cursor/mcp.json");
    write(
        &mcp_source,
        r#"{"mcpServers":{"broken":{"command":"demo"}}}"#,
    );
    let destination_asset_root = root.join("repo/.ai-config");
    let blocked_parent = destination_asset_root.join("mcp/servers");
    fs::create_dir_all(blocked_parent.as_std_path()).unwrap();
    fs::set_permissions(
        blocked_parent.as_std_path(),
        fs::Permissions::from_mode(0o500),
    )
    .unwrap();
    let requests = [
        ImportRequest {
            kind: AssetKind::Agent,
            name: "helper".to_owned(),
            source_platform: PlatformId::Cursor,
            source_path: agent_source,
            approved_source_root: root.join("repo"),
            destination_layer: SourceLayer::Project,
            destination_asset_root: destination_asset_root.clone(),
            replace: false,
        },
        ImportRequest {
            kind: AssetKind::Mcp,
            name: "broken".to_owned(),
            source_platform: PlatformId::Cursor,
            source_path: mcp_source,
            approved_source_root: root.join("repo"),
            destination_layer: SourceLayer::Project,
            destination_asset_root: destination_asset_root.clone(),
            replace: false,
        },
    ];
    let plan = build_import_plan(&requests).unwrap();
    let selected = plan
        .actions
        .iter()
        .map(|action| action.action_id.clone())
        .collect::<Vec<_>>();

    let result = apply_import_plan(
        &plan,
        &ImportApplyOptions::new(&plan.plan_digest, selected),
        &root.join("transactions"),
    );
    fs::set_permissions(
        blocked_parent.as_std_path(),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    assert!(result.is_err());
    assert!(!destination_asset_root.join("agents/helper.md").exists());
    assert!(!destination_asset_root
        .join("mcp/servers/broken.json")
        .exists());
}

#[test]
fn rollback_restores_an_unchanged_replaced_canonical_source() {
    let temp = TempDir::new().unwrap();
    let root = utf8(temp.path());
    let request = prompt_request(root, true);
    write(&request.source_path, "imported\n");
    let destination = root.join("repo/.ai-config/prompts/AGENTS.md");
    write(&destination, "canonical before import\n");
    let plan = build_import_plan(&[request]).unwrap();
    let report = apply_import_plan(
        &plan,
        &ImportApplyOptions::new(&plan.plan_digest, [plan.actions[0].action_id.clone()]),
        &root.join("transactions"),
    )
    .unwrap();

    let rollback = rollback_import_transaction(
        &root.join("transactions"),
        report.transaction_id.as_deref().unwrap(),
        &[root.join("repo/.ai-config")],
    )
    .unwrap();

    assert_eq!(rollback.status, RollbackStatus::RolledBack);
    assert_eq!(rollback.restored, 1);
    assert_eq!(fs::read(destination).unwrap(), b"canonical before import\n");
}

#[test]
fn rollback_refuses_a_concurrent_import_lock_without_changing_the_canonical_source() {
    let temp = TempDir::new().unwrap();
    let root = utf8(temp.path());
    let request = prompt_request(root, true);
    write(&request.source_path, "imported\n");
    let destination = root.join("repo/.ai-config/prompts/AGENTS.md");
    write(&destination, "canonical before import\n");
    let plan = build_import_plan(&[request]).unwrap();
    let transaction_root = root.join("transactions");
    let report = apply_import_plan(
        &plan,
        &ImportApplyOptions::new(&plan.plan_digest, [plan.actions[0].action_id.clone()]),
        &transaction_root,
    )
    .unwrap();
    write(
        &transaction_root.join(".ai-config-import.lock"),
        "concurrent apply\n",
    );
    let before = fs::read(&destination).unwrap();

    let result = rollback_import_transaction(
        &transaction_root,
        report.transaction_id.as_deref().unwrap(),
        &[root.join("repo/.ai-config")],
    );

    assert!(result.is_err());
    assert_eq!(fs::read(destination).unwrap(), before);
}

#[test]
fn rollback_recovers_a_write_completed_before_the_applied_flag_was_persisted() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new().unwrap();
    let root = utf8(temp.path());
    let transaction_root = root.join("transactions");
    fs::create_dir(&transaction_root).unwrap();
    fs::set_permissions(
        transaction_root.as_std_path(),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let approved = root.join("repo/.ai-config");
    let destination = approved.join("prompts/AGENTS.md");
    write(&destination, "write completed before manifest update\n");
    let transaction_id = "migration-interrupted-create";
    let manifest = serde_json::json!({
        "schema_version": 1,
        "transaction_id": transaction_id,
        "plan_digest": "reviewed-plan",
        "status": "applying",
        "actions": [{
            "action_id": "create-prompt",
            "kind": "prompt",
            "destination_asset_root": approved,
            "destination_path": destination,
            "before": {
                "entry_type": "missing",
                "digest": null,
                "link_target": null,
                "mode": null
            },
            "after": path_fingerprint(&destination).unwrap(),
            "backup_path": null,
            "applied": false
        }]
    });
    fs::write(
        transaction_root.join(format!("{transaction_id}.json")),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let rollback = rollback_import_transaction(
        &transaction_root,
        transaction_id,
        std::slice::from_ref(&approved),
    )
    .unwrap();

    assert_eq!(rollback.status, RollbackStatus::RolledBack);
    assert_eq!(rollback.restored, 1);
    assert!(!destination.exists());
    let persisted: serde_json::Value = serde_json::from_slice(
        &fs::read(transaction_root.join(format!("{transaction_id}.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(persisted["status"], "rolled_back");
}

#[test]
fn mcp_credential_like_unknown_field_is_redacted_and_blocks_import() {
    let temp = TempDir::new().unwrap();
    let root = utf8(temp.path());
    let sentinel = "TOP-SECRET-UNKNOWN-FIELD";
    let source = root.join("repo/.cursor/mcp.json");
    write(
        &source,
        &format!(r#"{{"mcpServers":{{"demo":{{"command":"demo","token":"{sentinel}"}}}}}}"#),
    );
    let request = ImportRequest {
        kind: AssetKind::Mcp,
        name: "demo".to_owned(),
        source_platform: PlatformId::Cursor,
        source_path: source,
        approved_source_root: root.join("repo"),
        destination_layer: SourceLayer::Project,
        destination_asset_root: root.join("repo/.ai-config"),
        replace: false,
    };

    let plan = build_import_plan(&[request]).unwrap();

    assert_eq!(
        plan.actions[0].secret_preflight.status,
        ImportSecretStatus::Blocked
    );
    assert_eq!(plan.actions[0].secret_preflight.key_names, ["token"]);
    assert!(!serde_json::to_string(&plan).unwrap().contains(sentinel));
}

#[test]
fn mcp_nested_credential_like_field_is_redacted_and_blocks_import() {
    let temp = TempDir::new().unwrap();
    let root = utf8(temp.path());
    let sentinel = "TOP-SECRET-NESTED-FIELD";
    let source = root.join("repo/.cursor/mcp.json");
    write(
        &source,
        &format!(
            r#"{{"mcpServers":{{"demo":{{"command":"demo","config":{{"token":"{sentinel}"}}}}}}}}"#
        ),
    );
    let request = ImportRequest {
        kind: AssetKind::Mcp,
        name: "demo".to_owned(),
        source_platform: PlatformId::Cursor,
        source_path: source,
        approved_source_root: root.join("repo"),
        destination_layer: SourceLayer::Project,
        destination_asset_root: root.join("repo/.ai-config"),
        replace: false,
    };

    let plan = build_import_plan(&[request]).unwrap();
    let serialized = serde_json::to_string(&plan).unwrap();

    assert_eq!(
        plan.actions[0].secret_preflight.status,
        ImportSecretStatus::Blocked
    );
    assert_eq!(plan.actions[0].secret_preflight.key_names, ["token"]);
    assert!(!serialized.contains(sentinel));
    let selected = plan.actions[0].action_id.clone();
    assert!(apply_import_plan(
        &plan,
        &ImportApplyOptions::new(&plan.plan_digest, [selected]),
        &root.join("transactions"),
    )
    .is_err());
    assert!(!root.join("repo/.ai-config/mcp/servers/demo.json").exists());
}

#[test]
fn mcp_credential_query_parameter_is_redacted_and_blocks_import() {
    let temp = TempDir::new().unwrap();
    let root = utf8(temp.path());
    let sentinel = "TOP-SECRET-URL-QUERY";
    let source = root.join("repo/.cursor/mcp.json");
    write(
        &source,
        &format!(
            r#"{{"mcpServers":{{"demo":{{"url":"https://example.test/mcp?%74oken={sentinel}","config":{{"endpoint":"https://nested.test/path?api_key={sentinel}"}}}}}}}}"#
        ),
    );
    let request = ImportRequest {
        kind: AssetKind::Mcp,
        name: "demo".to_owned(),
        source_platform: PlatformId::Cursor,
        source_path: source,
        approved_source_root: root.join("repo"),
        destination_layer: SourceLayer::Project,
        destination_asset_root: root.join("repo/.ai-config"),
        replace: false,
    };

    let plan = build_import_plan(&[request]).unwrap();
    let serialized = serde_json::to_string(&plan).unwrap();

    assert_eq!(
        plan.actions[0].secret_preflight.status,
        ImportSecretStatus::Blocked
    );
    assert!(plan.actions[0]
        .secret_preflight
        .key_names
        .contains(&"URL_QUERY".to_owned()));
    assert!(!serialized.contains(sentinel));
}

#[test]
fn import_apply_refuses_a_preexisting_transaction_lock_without_writing() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new().unwrap();
    let root = utf8(temp.path());
    let request = prompt_request(root, false);
    write(&request.source_path, "locked import\n");
    let plan = build_import_plan(&[request]).unwrap();
    let transaction_root = root.join("transactions");
    fs::create_dir(&transaction_root).unwrap();
    fs::set_permissions(
        transaction_root.as_std_path(),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    write(
        &transaction_root.join(".ai-config-import.lock"),
        "active or requires inspection\n",
    );

    let result = apply_import_plan(
        &plan,
        &ImportApplyOptions::new(&plan.plan_digest, [plan.actions[0].action_id.clone()]),
        &transaction_root,
    );

    assert!(result.is_err());
    assert!(!root.join("repo/.ai-config/prompts/AGENTS.md").exists());
    assert_eq!(
        fs::read(transaction_root.join(".ai-config-import.lock")).unwrap(),
        b"active or requires inspection\n"
    );
}

#[test]
fn import_apply_is_zero_write_when_an_unselected_sibling_is_secret_blocked() {
    let temp = TempDir::new().unwrap();
    let root = utf8(temp.path());
    let prompt = prompt_request(root, false);
    write(&prompt.source_path, "safe selected prompt\n");
    let mcp_source = root.join("repo/.cursor/mcp.json");
    write(
        &mcp_source,
        r#"{"mcpServers":{"blocked":{"command":"demo","env":{"TOKEN":"literal"}}}}"#,
    );
    let blocked = ImportRequest {
        kind: AssetKind::Mcp,
        name: "blocked".to_owned(),
        source_platform: PlatformId::Cursor,
        source_path: mcp_source,
        approved_source_root: root.join("repo"),
        destination_layer: SourceLayer::Project,
        destination_asset_root: root.join("repo/.ai-config"),
        replace: false,
    };
    let plan = build_import_plan(&[prompt, blocked]).unwrap();
    let selected = plan
        .actions
        .iter()
        .find(|action| action.kind == AssetKind::Prompt)
        .unwrap()
        .action_id
        .clone();

    let result = apply_import_plan(
        &plan,
        &ImportApplyOptions::new(&plan.plan_digest, [selected]),
        &root.join("transactions"),
    );

    assert!(result.is_err());
    assert!(!root.join("repo/.ai-config/prompts/AGENTS.md").exists());
    assert!(!root
        .join("repo/.ai-config/mcp/servers/blocked.json")
        .exists());
}

#[test]
fn skill_import_preserves_executable_mode_and_mode_changes_stale_the_plan() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new().unwrap();
    let root = utf8(temp.path());
    let source = root.join("repo/.agents/skills/demo");
    write(&source.join("SKILL.md"), "# Demo\n");
    write(&source.join("scripts/run.sh"), "#!/bin/sh\nexit 0\n");
    fs::set_permissions(
        source.join("scripts/run.sh").as_std_path(),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let request = ImportRequest {
        kind: AssetKind::Skill,
        name: "demo".to_owned(),
        source_platform: PlatformId::Codex,
        source_path: source.clone(),
        approved_source_root: root.join("repo"),
        destination_layer: SourceLayer::Project,
        destination_asset_root: root.join("repo/.ai-config"),
        replace: false,
    };
    let plan = build_import_plan(std::slice::from_ref(&request)).unwrap();

    fs::set_permissions(
        source.join("scripts/run.sh").as_std_path(),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(apply_import_plan(
        &plan,
        &ImportApplyOptions::new(&plan.plan_digest, [plan.actions[0].action_id.clone()],),
        &root.join("transactions"),
    )
    .is_err());

    fs::set_permissions(
        source.join("scripts/run.sh").as_std_path(),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let current = build_import_plan(&[request]).unwrap();
    apply_import_plan(
        &current,
        &ImportApplyOptions::new(&current.plan_digest, [current.actions[0].action_id.clone()]),
        &root.join("transactions"),
    )
    .unwrap();
    let mode = fs::metadata(
        root.join("repo/.ai-config/skills/demo/scripts/run.sh")
            .as_std_path(),
    )
    .unwrap()
    .permissions()
    .mode();
    assert_ne!(mode & 0o111, 0);
}

#[test]
fn rollback_rejects_manifest_paths_outside_caller_approved_canonical_roots() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new().unwrap();
    let root = utf8(temp.path());
    let transaction_root = root.join("transactions");
    fs::create_dir(&transaction_root).unwrap();
    fs::set_permissions(
        transaction_root.as_std_path(),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let approved = root.join("repo/.ai-config");
    fs::create_dir_all(approved.as_std_path()).unwrap();
    let victim = root.join("victim.txt");
    write(&victim, "must survive\n");
    let transaction_id = "migration-forged";
    let manifest = serde_json::json!({
        "schema_version": 1,
        "transaction_id": transaction_id,
        "plan_digest": "forged",
        "status": "applied",
        "actions": [{
            "action_id": "forged-action",
            "kind": "prompt",
            "destination_asset_root": root,
            "destination_path": victim,
            "before": {
                "entry_type": "missing",
                "digest": null,
                "link_target": null,
                "mode": null
            },
            "after": path_fingerprint(&victim).unwrap(),
            "backup_path": null,
            "applied": true
        }]
    });
    fs::write(
        transaction_root.join(format!("{transaction_id}.json")),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    assert!(rollback_import_transaction(&transaction_root, transaction_id, &[approved],).is_err());
    assert_eq!(fs::read(victim).unwrap(), b"must survive\n");
}
