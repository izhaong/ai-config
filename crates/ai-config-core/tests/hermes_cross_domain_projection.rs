use std::fs;

use ai_config_core::error::CoreError;
use ai_config_core::model::{AssetKind, McpServer, McpTransport, PlatformId};
use ai_config_core::projection::executor::{apply_projection_plan, ApplyOptions, ExecutorContext};
use ai_config_core::projection::fingerprint::path_content_digest;
use ai_config_core::projection::ledger::{MemoryProjectionLedger, ProjectionLedger};
use ai_config_core::projection::mcp::source::{EffectiveMcpDefinition, McpDefinition};
use ai_config_core::projection::model::{DeploymentScope, EffectiveAsset, SourceLayer, SourceRef};
use ai_config_core::projection::planner::{
    build_hermes_cross_domain_projection_plan, build_mcp_projection_plan, build_projection_plan,
    PlannerContext, ProjectionActionKind, ProjectionOperation, ProjectionRequest,
};
use camino::{Utf8Path, Utf8PathBuf};
use tempfile::TempDir;

struct FailingLedger;

impl ProjectionLedger for FailingLedger {
    fn get(
        &self,
        _id: &ai_config_core::projection::model::ProjectionId,
    ) -> Result<Option<ai_config_core::projection::model::ProjectionRecord>, CoreError> {
        Ok(None)
    }

    fn get_many(
        &self,
        _ids: &[ai_config_core::projection::model::ProjectionId],
    ) -> Result<Vec<ai_config_core::projection::model::ProjectionRecord>, CoreError> {
        Ok(Vec::new())
    }

    fn list_scope(
        &self,
        _scope_key: &str,
    ) -> Result<Vec<ai_config_core::projection::model::ProjectionRecord>, CoreError> {
        Ok(Vec::new())
    }

    fn apply_batch(
        &self,
        _mutations: &[ai_config_core::projection::model::LedgerMutation],
    ) -> Result<(), CoreError> {
        Err(CoreError::ProjectionLedger(
            "forced ledger failure".to_owned(),
        ))
    }
}

fn file_asset(root: &Utf8Path, kind: AssetKind, name: &str) -> EffectiveAsset {
    let source_path = root.join("source/hooks").join(name);
    fs::create_dir_all(source_path.parent().unwrap().as_std_path()).unwrap();
    fs::write(source_path.as_std_path(), format!("{kind:?} {name}\n")).unwrap();
    EffectiveAsset {
        kind,
        name: name.to_owned(),
        source_path: source_path.clone(),
        layer: SourceLayer::Project,
        fingerprint: path_content_digest(&source_path).unwrap(),
    }
}

fn skill_asset(root: &Utf8Path, name: &str) -> EffectiveAsset {
    let source_path = root.join("source/skills").join(name);
    fs::create_dir_all(source_path.as_std_path()).unwrap();
    fs::write(
        source_path.join("SKILL.md").as_std_path(),
        "---\nname: demo\n---\n",
    )
    .unwrap();
    EffectiveAsset {
        kind: AssetKind::Skill,
        name: name.to_owned(),
        source_path: source_path.clone(),
        layer: SourceLayer::Project,
        fingerprint: path_content_digest(&source_path).unwrap(),
    }
}

fn mcp_definition(root: &Utf8Path) -> EffectiveMcpDefinition {
    let source_path = root.join("source/mcp/servers/catalog.json");
    fs::create_dir_all(source_path.parent().unwrap().as_std_path()).unwrap();
    let config = serde_json::json!({"command": "catalog-mcp"});
    fs::write(
        source_path.as_std_path(),
        serde_json::to_vec(&serde_json::json!({
            "targets": ["hermes"],
            "config": config,
        }))
        .unwrap(),
    )
    .unwrap();
    EffectiveMcpDefinition {
        definition: McpDefinition {
            server: McpServer {
                project_id: 0,
                name: "catalog".to_owned(),
                transport: McpTransport::Stdio,
                config,
                secret_keys: Vec::new(),
                enabled: true,
            },
            targets: vec![PlatformId::Hermes],
            source_path: source_path.clone(),
        },
        source: SourceRef {
            layer: SourceLayer::Project,
            absolute_path: source_path.clone(),
            fingerprint: path_content_digest(&source_path).unwrap(),
        },
    }
}

fn request(
    root: &Utf8Path,
    scope: DeploymentScope,
    assets: Vec<EffectiveAsset>,
) -> ProjectionRequest {
    ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: format!("{scope:?}:/hermes-cross-domain"),
        scope,
        deploy_base: root.join("deploy"),
        assets,
        platforms: vec![PlatformId::Hermes],
    }
}

#[test]
fn hermes_user_cross_domain_intents_require_one_shared_config_batch() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = request(
        root,
        DeploymentScope::User,
        vec![
            skill_asset(root, "demo"),
            file_asset(root, AssetKind::Hook, "format.sh"),
        ],
    );
    let config = request.deploy_base.join(".hermes/config.yaml");
    fs::create_dir_all(config.parent().unwrap().as_std_path()).unwrap();
    fs::write(
        config.as_std_path(),
        "provider: foreign\nmodel: local\nunknown: preserve\nskills:\n  external_dirs:\n    - /opt/foreign-skills\nhooks:\n  PostToolUse:\n    - command: /opt/foreign-hook.sh\nmcp_servers:\n  foreign:\n    command: foreign-mcp\n",
    )
    .unwrap();
    let before = fs::read_to_string(config.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();

    let plan = build_hermes_cross_domain_projection_plan(
        &request,
        &[mcp_definition(root)],
        &PlannerContext::new(&ledger),
    )
    .unwrap();
    let shared_target_actions = plan
        .actions
        .iter()
        .filter(|action| {
            action
                .target
                .as_ref()
                .is_some_and(|target| target.path == config)
        })
        .collect::<Vec<_>>();

    assert_eq!(
        shared_target_actions.len(),
        1,
        "MCP, skills.external_dirs and Hook must be planned as one Hermes config.yaml batch"
    );
    assert!(
        shared_target_actions[0].kind == ProjectionActionKind::UpsertGeneratedBatch,
        "one shared batch must be executable after one parse/render/replace"
    );
    assert_eq!(
        fs::read_to_string(config.as_std_path()).unwrap(),
        before,
        "planning the shared batch is read-only and preserves foreign provider/model/unknown fields"
    );
}

#[test]
fn hermes_user_cross_domain_apply_replaces_one_container_and_preserves_foreign_fields() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let skill = skill_asset(root, "demo");
    let hook = file_asset(root, AssetKind::Hook, "format.sh");
    let request = request(
        root,
        DeploymentScope::User,
        vec![skill.clone(), hook.clone()],
    );
    let config = request.deploy_base.join(".hermes/config.yaml");
    fs::create_dir_all(config.parent().unwrap().as_std_path()).unwrap();
    fs::write(
        config.as_std_path(),
        "provider: foreign\nmodel: local\nunknown: preserve\nskills:\n  external_dirs:\n    - /opt/foreign-skills\nhooks:\n  PostToolUse:\n    - command: /opt/foreign-hook.sh\nmcp_servers:\n  foreign:\n    command: foreign-mcp\n",
    )
    .unwrap();
    let ledger = MemoryProjectionLedger::default();
    let plan = build_hermes_cross_domain_projection_plan(
        &request,
        &[mcp_definition(root)],
        &PlannerContext::new(&ledger),
    )
    .unwrap();
    assert_eq!(
        plan.actions
            .iter()
            .filter(|action| action
                .target
                .as_ref()
                .is_some_and(|target| target.path == config))
            .count(),
        1,
        "one config target means one parse/render/replace transaction"
    );

    let report = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    )
    .unwrap();
    assert_eq!(
        report.changed, 2,
        "one Hook link plus one shared YAML replacement"
    );
    let rendered: serde_yaml::Value =
        serde_yaml::from_str(&fs::read_to_string(config.as_std_path()).unwrap()).unwrap();
    assert_eq!(rendered["provider"], "foreign");
    assert_eq!(rendered["model"], "local");
    assert_eq!(rendered["unknown"], "preserve");
    assert_eq!(rendered["mcp_servers"]["foreign"]["command"], "foreign-mcp");
    assert_eq!(rendered["mcp_servers"]["catalog"]["command"], "catalog-mcp");
    assert!(rendered["skills"]["external_dirs"]
        .as_sequence()
        .unwrap()
        .iter()
        .any(|entry| entry.as_str() == Some(skill.source_path.as_str())));
    assert!(rendered["hooks"]["PostToolUse"]
        .as_sequence()
        .unwrap()
        .iter()
        .any(|entry| entry["hook"] == "format.sh"));
    assert_eq!(
        fs::read_link(
            request
                .deploy_base
                .join(".hermes/hooks/format.sh")
                .as_std_path()
        )
        .unwrap(),
        hook.source_path
    );
}

#[test]
fn hermes_user_cross_domain_refuses_a_same_name_foreign_mcp_without_writing() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let request = request(
        root,
        DeploymentScope::User,
        vec![
            skill_asset(root, "demo"),
            file_asset(root, AssetKind::Hook, "format.sh"),
        ],
    );
    let config = request.deploy_base.join(".hermes/config.yaml");
    fs::create_dir_all(config.parent().unwrap().as_std_path()).unwrap();
    fs::write(
        config.as_std_path(),
        "provider: foreign\nmcp_servers:\n  catalog:\n    command: third-party-catalog\n",
    )
    .unwrap();
    let before = fs::read_to_string(config.as_std_path()).unwrap();
    let ledger = MemoryProjectionLedger::default();
    let plan = build_hermes_cross_domain_projection_plan(
        &request,
        &[mcp_definition(root)],
        &PlannerContext::new(&ledger),
    )
    .unwrap();
    assert!(plan.actions.iter().any(|action| {
        action.kind == ProjectionActionKind::ReportOnly
            && action.state.as_deref() == Some("foreign")
    }));

    assert!(apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    )
    .is_err());
    assert_eq!(fs::read_to_string(config.as_std_path()).unwrap(), before);
    assert!(!request.deploy_base.join(".hermes/hooks/format.sh").exists());
}

#[test]
fn hermes_user_cross_domain_ledger_failure_restores_config_and_hook_link() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let hook = file_asset(root, AssetKind::Hook, "format.sh");
    let request = request(
        root,
        DeploymentScope::User,
        vec![skill_asset(root, "demo"), hook],
    );
    let config = request.deploy_base.join(".hermes/config.yaml");
    fs::create_dir_all(config.parent().unwrap().as_std_path()).unwrap();
    fs::write(config.as_std_path(), "provider: foreign\nmodel: local\n").unwrap();
    let before = fs::read_to_string(config.as_std_path()).unwrap();
    let ledger = FailingLedger;
    let plan = build_hermes_cross_domain_projection_plan(
        &request,
        &[mcp_definition(root)],
        &PlannerContext::new(&ledger),
    )
    .unwrap();

    let failure = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    );

    assert!(failure.is_err());
    assert_eq!(fs::read_to_string(config.as_std_path()).unwrap(), before);
    assert!(!request.deploy_base.join(".hermes/hooks/format.sh").exists());
}

#[test]
fn hermes_workspace_and_project_cross_domain_requests_are_unsupported_without_global_writes() {
    for scope in [DeploymentScope::Workspace, DeploymentScope::Project] {
        let temp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let request = request(
            root,
            scope,
            vec![file_asset(root, AssetKind::Hook, "format.sh")],
        );
        let user_global =
            Utf8PathBuf::from_path_buf(temp.path().join("user-global/.hermes/config.yaml"))
                .unwrap();
        fs::create_dir_all(user_global.parent().unwrap().as_std_path()).unwrap();
        fs::write(user_global.as_std_path(), "provider: foreign\n").unwrap();
        let before = fs::read_to_string(user_global.as_std_path()).unwrap();
        let ledger = MemoryProjectionLedger::default();

        let hook_plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
        let mcp_plan = build_mcp_projection_plan(
            &request,
            &[mcp_definition(root)],
            &PlannerContext::new(&ledger),
        )
        .unwrap();

        assert!(hook_plan.actions.iter().all(|action| {
            action.kind == ProjectionActionKind::ReportOnly
                && action.state.as_deref() == Some("unsupported")
        }));
        assert!(mcp_plan.actions.iter().all(|action| {
            action.kind == ProjectionActionKind::ReportOnly
                && action.state.as_deref() == Some("unsupported")
        }));
        assert!(!request.deploy_base.join(".hermes/config.yaml").exists());
        assert_eq!(
            fs::read_to_string(user_global.as_std_path()).unwrap(),
            before
        );
    }
}
