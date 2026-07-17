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

const BIN: &str = "ai-config";

struct Fixture {
    home: TempDir,
    asset_root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let home = TempDir::new().expect("temporary HOME");
        let asset_root = home.path().join(".ai-config");
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
            .env_remove("AI_CONFIG_ROOT")
            .env_remove("AI_CONFIG_SECRETS_DIR")
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
fn migration_apply_is_zero_write_when_any_sibling_action_is_blocking() {
    let fixture = Fixture::new();
    fixture.canonical_skill("selected", "canonical selected\n");
    fixture.platform_skill_copy("selected", "canonical selected\n");
    fixture.canonical_skill("foreign", "canonical foreign\n");
    fixture.platform_skill_copy("foreign", "different platform content\n");

    let plan = fixture.migration_plan();
    let selected_action = action_id_for(&plan, "selected");
    let before = tree_snapshot(fixture.home());
    let output = fixture.migration_source_first(&plan, &[&selected_action], true);

    assert!(
        !output.status.success(),
        "a blocking sibling must reject the whole reviewed transaction: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        tree_snapshot(fixture.home()),
        before,
        "a blocking sibling must keep every selected and unselected target unchanged"
    );
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

struct ProjectFixture {
    home: TempDir,
    project: PathBuf,
    asset_root: PathBuf,
}

impl ProjectFixture {
    fn new() -> Self {
        let home = TempDir::new().expect("temporary HOME");
        let project = home.path().join("project");
        let asset_root = project.join(".ai-config");
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
            .env_remove("AI_CONFIG_ROOT")
            .env_remove("AI_CONFIG_SECRETS_DIR")
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
            .join(".ai-config/prompts/AGENTS.md")
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
        fs::read(fixture.project.join(".ai-config/prompts/AGENTS.md")).unwrap(),
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
    let canonical = fixture.project.join(".ai-config/prompts/AGENTS.md");
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
