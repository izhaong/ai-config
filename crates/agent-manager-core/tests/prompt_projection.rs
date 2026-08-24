use std::fs;

use agent_manager_core::model::{AssetKind, PlatformId};
use agent_manager_core::projection::executor::{
    apply_projection_plan, ApplyActionStatus, ApplyOptions, ExecutorContext,
};
use agent_manager_core::projection::ledger::MemoryProjectionLedger;
use agent_manager_core::projection::model::{DeploymentScope, SourceLayer};
use agent_manager_core::projection::planner::{
    build_projection_plan, PlannerContext, ProjectionActionKind, ProjectionOperation,
    ProjectionRequest,
};
use agent_manager_core::projection::source::{resolve_effective_assets, OverlayRoots};
use camino::{Utf8Path, Utf8PathBuf};
use tempfile::TempDir;

fn write(path: &Utf8Path, contents: &str) {
    fs::create_dir_all(
        path.parent()
            .expect("fixture path has parent")
            .as_std_path(),
    )
    .unwrap();
    fs::write(path.as_std_path(), contents).unwrap();
}

fn project_roots(root: &Utf8Path) -> OverlayRoots {
    OverlayRoots {
        global: root.join("global/.agent-manager"),
        workspace: Some(root.join("workspace/.agent-manager")),
        project: root.join("project/.agent-manager"),
    }
}

fn project_request(root: &Utf8Path) -> ProjectionRequest {
    ProjectionRequest {
        operation: ProjectionOperation::Sync,
        scope_key: "project:/fixture".to_owned(),
        scope: DeploymentScope::Project,
        deploy_base: root.join("project"),
        assets: resolve_effective_assets(&project_roots(root)).unwrap(),
        platforms: vec![
            PlatformId::Cursor,
            PlatformId::Codex,
            PlatformId::Claude,
            PlatformId::Hermes,
        ],
    }
}

fn project_template_root() -> Utf8PathBuf {
    Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("templates/project")
}

#[test]
fn project_template_seeds_the_canonical_agents_prompt() {
    let prompt = project_template_root().join(".agent-manager/prompts/AGENTS.md");

    assert!(
        prompt.is_file(),
        "the project template must seed the complete prompt at .agent-manager/prompts/AGENTS.md"
    );
    assert!(
        !fs::read_to_string(prompt.as_std_path())
            .unwrap()
            .trim()
            .is_empty(),
        "the canonical prompt must contain the project entry body"
    );
}

#[cfg(unix)]
#[test]
fn project_template_root_agents_is_an_exact_canonical_prompt_link() {
    let template_root = project_template_root();
    let agents = template_root.join("AGENTS.md");

    assert!(
        fs::symlink_metadata(agents.as_std_path())
            .expect("template AGENTS entry exists")
            .file_type()
            .is_symlink(),
        "the template root AGENTS.md must not be a second regular-file prompt"
    );
    assert_eq!(
        fs::read_link(agents.as_std_path()).unwrap(),
        std::path::PathBuf::from(".agent-manager/prompts/AGENTS.md"),
        "the project entry must link exactly to the canonical prompt"
    );
}

#[test]
fn project_template_claude_wrapper_only_references_agents() {
    let wrapper = fs::read_to_string(project_template_root().join("CLAUDE.md").as_std_path())
        .expect("template Claude wrapper exists");

    assert_eq!(
        wrapper.trim(),
        "Read [AGENTS.md](./AGENTS.md).",
        "Claude must use a minimal wrapper instead of maintaining prompt body or routing copies"
    );
}

#[test]
fn prompt_plans_one_project_entry_for_all_platforms() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write(
        &root.join("project/.agent-manager/prompts/AGENTS.md"),
        "canonical project entry\n",
    );
    let request = project_request(root);
    let ledger = MemoryProjectionLedger::default();

    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    assert_eq!(
        plan.actions.len(),
        1,
        "four consumers share one entry action"
    );
    assert_eq!(plan.actions[0].kind, ProjectionActionKind::CreateLink);
    assert_eq!(
        plan.actions[0].target.as_ref().unwrap().path,
        request.deploy_base.join("AGENTS.md")
    );
    assert_eq!(
        plan.actions[0].consumers,
        vec![
            PlatformId::Cursor,
            PlatformId::Codex,
            PlatformId::Claude,
            PlatformId::Hermes,
        ]
    );
    assert_eq!(plan.actions[0].members[0].id.kind, AssetKind::Prompt);
    assert_eq!(
        plan.actions[0].members[0].source.layer,
        SourceLayer::Project
    );
}

#[cfg(unix)]
#[test]
fn external_agents_link_is_reported_without_overwrite() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write(
        &root.join("project/.agent-manager/prompts/AGENTS.md"),
        "canonical project entry\n",
    );
    let external = root.join("external/AGENTS.md");
    write(&external, "external entry\n");
    fs::create_dir_all(root.join("project").as_std_path()).unwrap();
    std::os::unix::fs::symlink(
        external.as_std_path(),
        root.join("project/AGENTS.md").as_std_path(),
    )
    .unwrap();
    let request = project_request(root);
    let ledger = MemoryProjectionLedger::default();

    let plan = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();

    assert_eq!(plan.actions.len(), 1);
    assert_eq!(plan.actions[0].kind, ProjectionActionKind::ReportOnly);
    assert!(matches!(
        plan.actions[0].state.as_deref(),
        Some("foreign") | Some("conflict") | Some("equivalent")
    ));

    let result = apply_projection_plan(
        &plan,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&plan),
    );
    assert!(result.is_err(), "a foreign project entry must block apply");
    assert_eq!(
        fs::read_link(root.join("project/AGENTS.md").as_std_path()).unwrap(),
        external,
        "foreign link must remain byte-for-byte untouched"
    );
}

#[cfg(unix)]
#[test]
fn prompt_overlay_then_reapply_is_an_unchanged_noop() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write(
        &root.join("global/.agent-manager/prompts/AGENTS.md"),
        "global entry\n",
    );
    write(
        &root.join("project/.agent-manager/prompts/AGENTS.md"),
        "project entry\n",
    );
    let request = project_request(root);
    let ledger = MemoryProjectionLedger::default();

    assert_eq!(request.assets.len(), 1);
    assert_eq!(request.assets[0].layer, SourceLayer::Project);
    let first = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    let first_report = apply_projection_plan(
        &first,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&first),
    )
    .unwrap();
    assert_eq!(first_report.actions[0].status, ApplyActionStatus::Applied);

    let second = build_projection_plan(&request, &PlannerContext::new(&ledger)).unwrap();
    assert_eq!(second.actions.len(), 1);
    assert_eq!(second.actions[0].kind, ProjectionActionKind::Noop);
    let second_report = apply_projection_plan(
        &second,
        &ExecutorContext::new(&ledger, request.deploy_base.clone(), root.join("backups")),
        ApplyOptions::for_plan(&second),
    )
    .unwrap();
    assert_eq!(
        second_report.actions[0].status,
        ApplyActionStatus::Unchanged
    );
}
