//! T010.2 migration/adoption CLI contract.
//!
//! These tests intentionally exercise the public CLI only.  Every fixture uses an isolated
//! HOME so a future migration executor cannot inspect or modify the developer's environment.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

const BIN: &str = "agent-manager";

struct Fixture {
    home: TempDir,
    asset_root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let home = TempDir::new().expect("temporary HOME");
        let asset_root = home.path().join(".agent-manager");
        fs::create_dir_all(&asset_root).expect("canonical asset root");
        Self { home, asset_root }
    }

    fn home(&self) -> &Path {
        self.home.path()
    }

    fn canonical_skill(&self, name: &str, content: &str) -> PathBuf {
        let skill = self.asset_root.join("skills").join(name);
        write(&skill.join("SKILL.md"), content);
        skill
    }

    fn platform_skill_copy(&self, name: &str, content: &str) -> PathBuf {
        let skill = self.home().join(".agents/skills").join(name);
        write(&skill.join("SKILL.md"), content);
        skill
    }

    fn command(&self) -> Command {
        let mut command = Command::cargo_bin(BIN).expect("CLI binary");
        command
            .env("HOME", self.home())
            .env("USERPROFILE", self.home())
            .env_remove("AGENT_MANAGER_ROOT")
            .env_remove("AGENT_MANAGER_SECRETS_DIR")
            .env_remove("HERMES_SKILLS_DIR")
            .arg("--root")
            .arg(&self.asset_root)
            .arg("--json");
        command
    }

    fn migration_plan_output(&self) -> std::process::Output {
        self.command()
            .args(["migrate", "plan"])
            .output()
            .expect("run migration plan")
    }

    fn migration_plan(&self) -> Value {
        let output = self.migration_plan_output();
        assert!(
            output.status.success(),
            "migration plan must be a successful read-only command: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        let plan: Value = serde_json::from_slice(&output.stdout).expect("migration plan JSON");
        assert!(plan["schema_version"].is_u64(), "plan={plan:?}");
        assert!(
            plan["plan_digest"]
                .as_str()
                .is_some_and(|digest| !digest.is_empty()),
            "plan must expose a review digest: {plan:?}"
        );
        assert!(plan["actions"].is_array(), "plan={plan:?}");
        plan
    }

    fn migration_source_first(
        &self,
        plan: &Value,
        action_ids: &[&str],
        apply: bool,
    ) -> std::process::Output {
        let mut reviewed = tempfile::NamedTempFile::new().expect("reviewed migration plan");
        serde_json::to_writer(&mut reviewed, plan).expect("serialize reviewed migration plan");
        reviewed.flush().expect("flush reviewed migration plan");
        let mut command = self.command();
        command
            .args(["migrate", "source-first", "--plan"])
            .arg(reviewed.path())
            .args(action_ids.iter().flat_map(|id| ["--select", *id]));
        if apply {
            command.arg("--apply");
        }
        command.output().expect("run migration apply")
    }

    fn migration_rollback(&self, transaction_id: &str) -> std::process::Output {
        self.command()
            .args(["migrate", "rollback", transaction_id])
            .output()
            .expect("run migration rollback")
    }
}

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture parent");
    fs::write(path, content).expect("write fixture");
}

fn tree_snapshot(root: &Path) -> BTreeMap<PathBuf, SnapshotPayload> {
    let mut snapshot = BTreeMap::new();
    snapshot_tree(root, root, &mut snapshot);
    snapshot
}

fn snapshot_tree(root: &Path, path: &Path, snapshot: &mut BTreeMap<PathBuf, SnapshotPayload>) {
    let metadata = fs::symlink_metadata(path).expect("lstat fixture entry");
    let relative = path
        .strip_prefix(root)
        .expect("fixture entry below root")
        .to_path_buf();
    let payload = if metadata.file_type().is_symlink() {
        SnapshotPayload::Symlink(fs::read_link(path).expect("read symlink"))
    } else if metadata.is_file() {
        SnapshotPayload::File(fs::read(path).expect("read fixture file"))
    } else if metadata.is_dir() {
        SnapshotPayload::Directory
    } else {
        SnapshotPayload::Other
    };
    snapshot.insert(relative, payload);

    if !metadata.is_dir() {
        return;
    }
    let mut children = fs::read_dir(path)
        .expect("read fixture directory")
        .map(|entry| entry.expect("fixture child").path())
        .collect::<Vec<_>>();
    children.sort();
    for child in children {
        snapshot_tree(root, &child, snapshot);
    }
}

#[derive(Debug, PartialEq, Eq)]
enum SnapshotPayload {
    Directory,
    File(Vec<u8>),
    Symlink(PathBuf),
    Other,
}

fn action_id_for(plan: &Value, name: &str) -> String {
    plan["actions"]
        .as_array()
        .expect("plan actions")
        .iter()
        .find(|action| {
            action["members"]
                .as_array()
                .is_some_and(|members| members.iter().any(|member| member["id"]["name"] == name))
        })
        .and_then(|action| action["action_id"].as_str())
        .unwrap_or_else(|| panic!("missing action_id for {name:?}: {plan:?}"))
        .to_owned()
}

#[test]
fn migration_plan_is_deterministic_and_inventory_and_plan_are_zero_write() {
    let fixture = Fixture::new();
    fixture.canonical_skill("review", "canonical review\n");
    fixture.platform_skill_copy("review", "canonical review\n");

    let before = tree_snapshot(fixture.home());
    let inventory = fixture
        .command()
        .args(["migrate", "inventory"])
        .output()
        .expect("run migration inventory");
    assert!(
        inventory.status.success(),
        "inventory must remain available before migration actions: {}",
        String::from_utf8_lossy(&inventory.stderr)
    );
    assert_eq!(
        tree_snapshot(fixture.home()),
        before,
        "inventory must not write"
    );

    let first = fixture.migration_plan();
    assert_eq!(tree_snapshot(fixture.home()), before, "plan must not write");
    let second = fixture.migration_plan();
    assert_eq!(
        second, first,
        "unchanged migration inventory must produce deterministic actions and digest"
    );
    assert_eq!(
        tree_snapshot(fixture.home()),
        before,
        "repeated plan must not write"
    );
}

#[test]
fn migration_apply_requires_a_plan_digest_and_explicit_selected_action_ids() {
    let fixture = Fixture::new();
    fixture.canonical_skill("review", "canonical review\n");
    fixture.platform_skill_copy("review", "canonical review\n");
    let plan = fixture.migration_plan();
    let action_id = action_id_for(&plan, "review");
    let before = tree_snapshot(fixture.home());

    let mut plan_without_digest = plan.clone();
    plan_without_digest
        .as_object_mut()
        .expect("plan object")
        .remove("plan_digest");
    let missing_digest = fixture.migration_source_first(&plan_without_digest, &[&action_id], true);
    assert!(
        !missing_digest.status.success(),
        "apply must reject an action selection without the reviewed plan digest"
    );
    assert_eq!(
        tree_snapshot(fixture.home()),
        before,
        "missing digest must be zero-write"
    );

    let missing_selection = fixture.migration_source_first(&plan, &[], true);
    assert!(
        !missing_selection.status.success(),
        "apply must reject a plan digest without explicit action IDs"
    );
    assert_eq!(
        tree_snapshot(fixture.home()),
        before,
        "missing selection must be zero-write"
    );
}

#[test]
fn migration_apply_changes_only_the_explicitly_selected_equivalent_action() {
    let fixture = Fixture::new();
    let selected_source = fixture.canonical_skill("selected", "canonical selected\n");
    fixture.canonical_skill("unselected", "canonical unselected\n");
    fixture.platform_skill_copy("selected", "canonical selected\n");
    let unselected_target = fixture.platform_skill_copy("unselected", "canonical unselected\n");
    let unselected_before = tree_snapshot(&unselected_target);

    let plan = fixture.migration_plan();
    let selected_action = action_id_for(&plan, "selected");
    let output = fixture.migration_source_first(&plan, &[&selected_action], true);
    assert!(
        output.status.success(),
        "a reviewed selected equivalent action must apply: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let selected_target = fixture.home().join(".agents/skills/selected");
    assert_eq!(
        fs::read_link(&selected_target).expect("selected target becomes managed link"),
        selected_source,
        "selected adoption must point to canonical source"
    );
    assert_eq!(
        tree_snapshot(&unselected_target),
        unselected_before,
        "an equivalent action without its action ID must remain byte-for-byte untouched"
    );
}

#[test]
fn migration_apply_is_scoped_to_selected_equivalent_action_despite_blocking_sibling() {
    let fixture = Fixture::new();
    fixture.canonical_skill("selected", "canonical selected\n");
    fixture.platform_skill_copy("selected", "canonical selected\n");
    fixture.canonical_skill("foreign", "canonical foreign\n");
    fixture.platform_skill_copy("foreign", "different platform content\n");

    let plan = fixture.migration_plan();
    let selected_action = action_id_for(&plan, "selected");
    let foreign_target = fixture.home().join(".agents/skills/foreign");
    let foreign_before = tree_snapshot(&foreign_target);
    let output = fixture.migration_source_first(&plan, &[&selected_action], true);

    assert!(
        output.status.success(),
        "an unrelated blocking sibling must not reject the selected reviewed action: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        tree_snapshot(&foreign_target),
        foreign_before,
        "the unselected blocking sibling must remain unchanged"
    );
    assert!(fixture.home().join(".agents/skills/selected").is_symlink());
}

#[test]
fn content_different_foreign_target_cannot_be_directly_adopted_and_requires_import() {
    let fixture = Fixture::new();
    fixture.canonical_skill("foreign", "canonical content\n");
    let target = fixture.platform_skill_copy("foreign", "foreign platform content\n");
    let before = tree_snapshot(fixture.home());

    let plan = fixture.migration_plan();
    let action_id = action_id_for(&plan, "foreign");
    let action = plan["actions"]
        .as_array()
        .expect("plan actions")
        .iter()
        .find(|candidate| candidate["action_id"] == action_id)
        .expect("foreign action");
    assert_eq!(
        action["reason_code"], "import_required",
        "different foreign content must be sent through explicit import, never direct adopt: {action:?}"
    );

    let output = fixture.migration_source_first(&plan, &[&action_id], true);
    assert!(
        !output.status.success(),
        "different foreign content must reject direct adoption: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        tree_snapshot(fixture.home()),
        before,
        "rejected adopt must not write"
    );
    assert_eq!(
        fs::read(target.join("SKILL.md")).unwrap(),
        b"foreign platform content\n",
        "direct adoption must never overwrite the foreign target"
    );
}

#[test]
fn migration_apply_rejects_stale_digest_and_unknown_action_id_without_writing() {
    let fixture = Fixture::new();
    fixture.canonical_skill("review", "canonical review\n");
    let target = fixture.platform_skill_copy("review", "canonical review\n");
    let reviewed = fixture.migration_plan();
    let reviewed_action = action_id_for(&reviewed, "review");

    write(&target.join("SKILL.md"), "changed after review\n");
    let changed_snapshot = tree_snapshot(fixture.home());
    let stale = fixture.migration_source_first(&reviewed, &[&reviewed_action], true);
    assert!(
        !stale.status.success(),
        "apply must reject a reviewed digest after its target precondition changes"
    );
    assert_eq!(
        tree_snapshot(fixture.home()),
        changed_snapshot,
        "stale-plan rejection must not overwrite the changed target"
    );

    let current = fixture.migration_plan();
    let unknown = fixture.migration_source_first(&current, &["not-an-action-in-this-plan"], true);
    assert!(
        !unknown.status.success(),
        "apply must reject selected action IDs that are absent from the current plan"
    );
    assert_eq!(
        tree_snapshot(fixture.home()),
        changed_snapshot,
        "unknown action rejection must be zero-write"
    );
}

#[test]
fn migration_adoption_returns_one_transaction_and_rollback_restores_all_original_targets() {
    let fixture = Fixture::new();
    let alpha_source = fixture.canonical_skill("alpha", "canonical alpha\n");
    let beta_source = fixture.canonical_skill("beta", "canonical beta\n");
    let alpha_target = fixture.platform_skill_copy("alpha", "canonical alpha\n");
    let beta_target = fixture.platform_skill_copy("beta", "canonical beta\n");
    let alpha_before = tree_snapshot(&alpha_target);
    let beta_before = tree_snapshot(&beta_target);

    let plan = fixture.migration_plan();
    let alpha_action = action_id_for(&plan, "alpha");
    let beta_action = action_id_for(&plan, "beta");
    let applied = fixture.migration_source_first(&plan, &[&alpha_action, &beta_action], true);
    assert!(
        applied.status.success(),
        "two reviewed equivalent actions must apply atomically: stdout={} stderr={}",
        String::from_utf8_lossy(&applied.stdout),
        String::from_utf8_lossy(&applied.stderr),
    );
    let report: Value = serde_json::from_slice(&applied.stdout).expect("adoption apply JSON");
    let transaction_id = report["transaction_id"]
        .as_str()
        .expect("adoption apply must return one durable transaction_id");
    assert!(
        transaction_id.starts_with("adopt-"),
        "adoption transaction must be distinctly addressable: {report:?}"
    );
    assert_eq!(
        fs::read_link(&alpha_target).expect("alpha became managed link"),
        alpha_source
    );
    assert_eq!(
        fs::read_link(&beta_target).expect("beta became managed link"),
        beta_source
    );

    let rollback = fixture.migration_rollback(transaction_id);
    assert!(
        rollback.status.success(),
        "rollback must restore every selected adoption: stdout={} stderr={}",
        String::from_utf8_lossy(&rollback.stdout),
        String::from_utf8_lossy(&rollback.stderr),
    );
    assert_eq!(tree_snapshot(&alpha_target), alpha_before);
    assert_eq!(tree_snapshot(&beta_target), beta_before);

    let after_rollback = fixture.migration_plan();
    for name in ["alpha", "beta"] {
        let action = after_rollback["actions"]
            .as_array()
            .expect("plan actions")
            .iter()
            .find(|action| {
                action["members"].as_array().is_some_and(|members| {
                    members.iter().any(|member| member["id"]["name"] == name)
                })
            })
            .unwrap_or_else(|| panic!("rollback plan missing {name:?}: {after_rollback:?}"));
        assert_eq!(
            action["kind"], "adopt_equivalent",
            "rollback must revoke managed-ledger ownership and return {name:?} to an explicit equivalent adoption candidate: {action:?}"
        );
    }
}

#[test]
fn migration_adoption_rollback_refuses_one_drift_without_touching_other_selected_targets() {
    let fixture = Fixture::new();
    fixture.canonical_skill("drifted", "canonical drifted\n");
    fixture.canonical_skill("untouched", "canonical untouched\n");
    let drifted_target = fixture.platform_skill_copy("drifted", "canonical drifted\n");
    let untouched_target = fixture.platform_skill_copy("untouched", "canonical untouched\n");
    let plan = fixture.migration_plan();
    let drifted_action = action_id_for(&plan, "drifted");
    let untouched_action = action_id_for(&plan, "untouched");
    let applied =
        fixture.migration_source_first(&plan, &[&drifted_action, &untouched_action], true);
    assert!(applied.status.success());
    let report: Value = serde_json::from_slice(&applied.stdout).expect("adoption apply JSON");
    let transaction_id = report["transaction_id"]
        .as_str()
        .expect("adoption apply must return a transaction_id");

    fs::remove_file(&drifted_target).expect("replace managed link with user drift");
    write(
        &drifted_target.join("SKILL.md"),
        "user changed this target after adoption\n",
    );
    let untouched_before_rollback = tree_snapshot(&untouched_target);

    let rollback = fixture.migration_rollback(transaction_id);
    assert!(
        !rollback.status.success(),
        "one drifted adopted target must fail the entire rollback: stdout={} stderr={}",
        String::from_utf8_lossy(&rollback.stdout),
        String::from_utf8_lossy(&rollback.stderr),
    );
    assert_eq!(
        tree_snapshot(&untouched_target),
        untouched_before_rollback,
        "rollback must preflight every selected target and leave non-drifted siblings untouched when any target drifted"
    );
    assert_eq!(
        fs::read(drifted_target.join("SKILL.md")).unwrap(),
        b"user changed this target after adoption\n"
    );
}

#[test]
fn migration_adoption_plan_only_and_stale_paths_are_read_only_while_unrelated_blockers_are_ignored()
{
    let plan_only = Fixture::new();
    plan_only.canonical_skill("review", "canonical review\n");
    plan_only.platform_skill_copy("review", "canonical review\n");
    let reviewed = plan_only.migration_plan();
    let review_action = action_id_for(&reviewed, "review");
    let before_plan_only = tree_snapshot(plan_only.home());
    let dry_run = plan_only.migration_source_first(&reviewed, &[&review_action], false);
    assert!(dry_run.status.success());
    let dry_run_report: Value = serde_json::from_slice(&dry_run.stdout).expect("dry-run JSON");
    assert!(
        dry_run_report.get("transaction_id").is_none(),
        "plan-only adoption must not reserve a durable transaction: {dry_run_report:?}"
    );
    assert_eq!(tree_snapshot(plan_only.home()), before_plan_only);

    let stale = Fixture::new();
    stale.canonical_skill("review", "canonical review\n");
    let stale_target = stale.platform_skill_copy("review", "canonical review\n");
    let reviewed = stale.migration_plan();
    let stale_action = action_id_for(&reviewed, "review");
    write(&stale_target.join("SKILL.md"), "changed after review\n");
    let before_stale = tree_snapshot(stale.home());
    let stale_apply = stale.migration_source_first(&reviewed, &[&stale_action], true);
    assert!(!stale_apply.status.success());
    assert_eq!(tree_snapshot(stale.home()), before_stale);

    let blocking = Fixture::new();
    blocking.canonical_skill("selected", "canonical selected\n");
    blocking.platform_skill_copy("selected", "canonical selected\n");
    blocking.canonical_skill("foreign", "canonical foreign\n");
    blocking.platform_skill_copy("foreign", "different platform content\n");
    let reviewed = blocking.migration_plan();
    let selected_action = action_id_for(&reviewed, "selected");
    let foreign_before = tree_snapshot(&blocking.home().join(".agents/skills/foreign"));
    let blocking_apply = blocking.migration_source_first(&reviewed, &[&selected_action], true);
    assert!(
        blocking_apply.status.success(),
        "an unrelated foreign action must not block an explicitly selected equivalent adoption: stdout={} stderr={}",
        String::from_utf8_lossy(&blocking_apply.stdout),
        String::from_utf8_lossy(&blocking_apply.stderr),
    );
    assert_eq!(
        tree_snapshot(&blocking.home().join(".agents/skills/foreign")),
        foreign_before,
        "unselected foreign targets must remain untouched"
    );
}

struct ProjectFixture {
    home: TempDir,
    project: PathBuf,
    asset_root: PathBuf,
}

impl ProjectFixture {
    fn new() -> Self {
        let home = TempDir::new().expect("temporary HOME");
        let project = home.path().join("project");
        let asset_root = project.join(".agent-manager");
        fs::create_dir_all(&project).expect("project root");
        Self {
            home,
            project,
            asset_root,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::cargo_bin(BIN).expect("CLI binary");
        command
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .env_remove("AGENT_MANAGER_ROOT")
            .env_remove("AGENT_MANAGER_SECRETS_DIR")
            .env_remove("HERMES_SKILLS_DIR")
            .arg("--root")
            .arg(&self.asset_root)
            .arg("--json");
        command
    }

    fn prompt_import_plan(&self) -> Value {
        let output = self
            .command()
            .args([
                "import", "prompt", "AGENTS", "--from", "codex", "--to", "project",
            ])
            .output()
            .expect("plan prompt import");
        assert!(
            output.status.success(),
            "prompt import plan failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        serde_json::from_slice(&output.stdout).expect("prompt import plan JSON")
    }

    fn apply_prompt_import(&self, plan: &Value) -> std::process::Output {
        let mut reviewed = tempfile::NamedTempFile::new().expect("reviewed import plan");
        serde_json::to_writer(&mut reviewed, plan).expect("serialize import plan");
        reviewed.flush().expect("flush import plan");
        self.command()
            .args([
                "import", "prompt", "AGENTS", "--from", "codex", "--to", "project", "--plan",
            ])
            .arg(reviewed.path())
            .arg("--apply")
            .output()
            .expect("apply prompt import")
    }

    fn mcp_import_plan(&self, name: &str) -> std::process::Output {
        self.command()
            .args(["import", "mcp", name, "--from", "codex", "--to", "project"])
            .output()
            .expect("plan Codex MCP import")
    }

    fn apply_mcp_import(&self, name: &str, plan: &Value) -> std::process::Output {
        let mut reviewed = tempfile::NamedTempFile::new().expect("reviewed MCP import plan");
        serde_json::to_writer(&mut reviewed, plan).expect("serialize MCP import plan");
        reviewed.flush().expect("flush MCP import plan");
        self.command()
            .args([
                "import", "mcp", name, "--from", "codex", "--to", "project", "--plan",
            ])
            .arg(reviewed.path())
            .arg("--apply")
            .output()
            .expect("apply Codex MCP import")
    }
}

#[test]
fn codex_mcp_import_is_reviewed_reversible_and_preserves_the_platform_target() {
    let fixture = ProjectFixture::new();
    let target = fixture.project.join(".codex/config.toml");
    write(
        &target,
        r#"model = "gpt-test"

[mcp_servers.gitea]
url = "https://mcp.example.test/mcp"
bearer_token_env_var = "PROJECT_GITEA_TOKEN"
"#,
    );
    let before = fs::read(&target).unwrap();

    let planned = fixture.mcp_import_plan("gitea");
    assert!(
        planned.status.success(),
        "Codex MCP import plan failed: stdout={} stderr={}",
        String::from_utf8_lossy(&planned.stdout),
        String::from_utf8_lossy(&planned.stderr),
    );
    let plan: Value = serde_json::from_slice(&planned.stdout).unwrap();
    assert_eq!(plan["actions"][0]["kind"], "mcp");
    assert_eq!(plan["actions"][0]["source_platform"], "codex");
    assert_eq!(fs::read(&target).unwrap(), before);

    let applied = fixture.apply_mcp_import("gitea", &plan);
    assert!(
        applied.status.success(),
        "Codex MCP import apply failed: stdout={} stderr={}",
        String::from_utf8_lossy(&applied.stdout),
        String::from_utf8_lossy(&applied.stderr),
    );
    let report: Value = serde_json::from_slice(&applied.stdout).unwrap();
    let transaction_id = report["transaction_id"].as_str().unwrap();
    let canonical = fixture.project.join(".agent-manager/mcp/servers/gitea.json");
    let canonical_json: Value = serde_json::from_slice(&fs::read(&canonical).unwrap()).unwrap();
    assert_eq!(canonical_json["targets"], serde_json::json!(["codex"]));
    assert_eq!(
        canonical_json["config"]["bearer_token_env_var"],
        "PROJECT_GITEA_TOKEN"
    );
    assert_eq!(fs::read(&target).unwrap(), before);
    assert!(report["projection_plan"]["actions"]
        .as_array()
        .is_some_and(|actions| actions.iter().any(|action| {
            action["kind"] == "adopt_equivalent"
                && action["reason_code"] == "equivalent_mcp_entries_unmanaged"
        })));

    let rolled_back = fixture
        .command()
        .args(["migrate", "rollback", transaction_id])
        .output()
        .expect("rollback Codex MCP import");
    assert!(
        rolled_back.status.success(),
        "Codex MCP rollback failed: stdout={} stderr={}",
        String::from_utf8_lossy(&rolled_back.stdout),
        String::from_utf8_lossy(&rolled_back.stderr),
    );
    assert!(!canonical.exists());
    assert_eq!(fs::read(&target).unwrap(), before);
}

#[test]
fn prompt_import_requires_reviewed_plan_and_preserves_platform_original() {
    let fixture = ProjectFixture::new();
    let agents = fixture.project.join("AGENTS.md");
    write(&agents, "# Project instructions\n");
    let before = tree_snapshot(&fixture.project);

    let plan = fixture.prompt_import_plan();
    assert_eq!(
        tree_snapshot(&fixture.project),
        before,
        "plan must not write"
    );
    assert_eq!(plan["actions"][0]["kind"], "prompt");
    assert_eq!(plan["actions"][0]["destination_layer"], "project");
    assert_eq!(
        plan["actions"][0]["destination_path"].as_str(),
        fixture
            .project
            .join(".agent-manager/prompts/AGENTS.md")
            .to_str()
    );
    assert_eq!(plan["actions"][0]["secret_preflight"]["status"], "clear");

    let unreviewed = fixture
        .command()
        .args([
            "import", "prompt", "AGENTS", "--from", "codex", "--to", "project", "--apply",
        ])
        .output()
        .expect("reject unreviewed import");
    assert!(!unreviewed.status.success());
    assert_eq!(tree_snapshot(&fixture.project), before);

    let applied = fixture.apply_prompt_import(&plan);
    assert!(
        applied.status.success(),
        "reviewed import failed: stdout={} stderr={}",
        String::from_utf8_lossy(&applied.stdout),
        String::from_utf8_lossy(&applied.stderr),
    );
    let report: Value = serde_json::from_slice(&applied.stdout).unwrap();
    assert!(report["transaction_id"].is_string(), "report={report:?}");
    assert!(
        report["projection_plan"]["plan_digest"]
            .as_str()
            .is_some_and(|digest| !digest.is_empty()),
        "a successful canonical import must return the next reviewed projection plan: {report:?}"
    );
    assert!(
        report["projection_plan"]["actions"]
            .as_array()
            .is_some_and(|actions| !actions.is_empty()),
        "the next plan must expose the foreign project entry convergence action: {report:?}"
    );
    assert_eq!(fs::read(&agents).unwrap(), b"# Project instructions\n");
    assert_eq!(
        fs::read(fixture.project.join(".agent-manager/prompts/AGENTS.md")).unwrap(),
        b"# Project instructions\n"
    );
}

#[test]
fn migration_rollback_restores_unchanged_import_and_refuses_drift() {
    let fixture = ProjectFixture::new();
    let agents = fixture.project.join("AGENTS.md");
    write(&agents, "# Project instructions\n");
    let plan = fixture.prompt_import_plan();
    let applied = fixture.apply_prompt_import(&plan);
    assert!(applied.status.success());
    let report: Value = serde_json::from_slice(&applied.stdout).unwrap();
    let transaction_id = report["transaction_id"].as_str().unwrap();
    let canonical = fixture.project.join(".agent-manager/prompts/AGENTS.md");
    write(&canonical, "user changed canonical after import\n");

    let drifted = fixture
        .command()
        .args(["migrate", "rollback", transaction_id])
        .output()
        .expect("reject drifted rollback");
    assert!(!drifted.status.success());
    assert_eq!(
        fs::read(&canonical).unwrap(),
        b"user changed canonical after import\n"
    );
    assert_eq!(fs::read(&agents).unwrap(), b"# Project instructions\n");
}
