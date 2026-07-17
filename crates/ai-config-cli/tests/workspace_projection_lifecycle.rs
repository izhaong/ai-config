//! T009 workspace lifecycle contract: members are planned together and applied atomically.

use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

const BIN: &str = "ai-config";

struct WorkspaceFixture {
    home: TempDir,
    workspace: TempDir,
}

impl WorkspaceFixture {
    fn new() -> Self {
        let home = TempDir::new().expect("temporary HOME");
        let workspace = TempDir::new().expect("temporary workspace");
        let root = workspace.path();
        fs::write(
            root.join(".gitmodules"),
            "[submodule \"one\"]\n\tpath = members/one\n[submodule \"two\"]\n\tpath = members/two\n",
        )
        .expect("workspace members");
        for member in ["one", "two"] {
            let asset_root = root.join(format!("members/{member}/.ai-config"));
            write(
                &asset_root.join(format!("skills/{member}/SKILL.md")),
                "---\nname: workspace skill\n---\ncanonical member skill\n",
            );
            write(
                &asset_root.join("prompts/AGENTS.md"),
                &format!("canonical instructions for {member}\n"),
            );
        }
        Self { home, workspace }
    }

    fn member(&self, name: &str) -> std::path::PathBuf {
        self.workspace.path().join("members").join(name)
    }

    fn cmd(&self) -> Command {
        let mut command = Command::cargo_bin(BIN).expect("CLI binary");
        command
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .env_remove("AI_CONFIG_ROOT")
            .env_remove("AI_CONFIG_SECRETS_DIR")
            .env_remove("HERMES_SKILLS_DIR")
            .arg("--root")
            .arg(self.workspace.path())
            .arg("--workspace");
        command
    }

    fn assert_no_member_targets(&self) {
        for member in ["one", "two"] {
            for target in [
                self.member(member).join(format!(".agents/skills/{member}")),
                self.member(member).join("AGENTS.md"),
                self.member(member).join(".cursor/mcp.json"),
            ] {
                assert!(
                    !target.exists(),
                    "workspace lifecycle must not leave target {}",
                    target.display()
                );
            }
        }
    }
}

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture parent");
    fs::write(path, content).expect("write fixture source");
}

fn workspace_report(report: &[u8]) -> Value {
    let report: Value = serde_json::from_slice(report).expect("workspace lifecycle JSON");
    let members = report["members"]
        .as_array()
        .expect("workspace lifecycle must return one report per member");
    assert_eq!(members.len(), 2, "workspace report={report:?}");
    for member in members {
        assert!(member["member"].is_string(), "member report={member:?}");
        assert!(
            member["plan"]["schema_version"].is_u64() && member["plan"]["plan_digest"].is_string(),
            "member plan must be independently reviewable: {member:?}"
        );
    }
    report
}

#[test]
fn workspace_sync_without_apply_returns_member_plans_and_writes_nothing() {
    let fixture = WorkspaceFixture::new();
    let output = fixture
        .cmd()
        .args(["--json", "sync"])
        .output()
        .expect("workspace sync plan");

    assert!(
        output.status.success(),
        "workspace plan must succeed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let report = workspace_report(&output.stdout);
    let members = report["members"].as_array().expect("workspace members");
    assert!(
        members.iter().all(|member| member.get("apply").is_none()),
        "workspace sync without --apply must remain plan-only"
    );
    fixture.assert_no_member_targets();
}

#[test]
fn workspace_sync_apply_projects_each_member_without_writing_home() {
    let fixture = WorkspaceFixture::new();
    let output = fixture
        .cmd()
        .args(["--json", "sync", "--apply"])
        .output()
        .expect("workspace sync apply");

    assert!(
        output.status.success(),
        "workspace apply must succeed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let report = workspace_report(&output.stdout);
    let members = report["members"].as_array().expect("workspace members");
    assert!(
        members
            .iter()
            .all(|member| member["apply"]["changed"].is_u64()),
        "each member must disclose its apply report"
    );
    for member in ["one", "two"] {
        assert!(
            fixture
                .member(member)
                .join(format!(".agents/skills/{member}"))
                .exists()
                && fixture.member(member).join("AGENTS.md").exists(),
            "--apply must project member {member} into its own repository"
        );
    }
    assert!(
        !fixture.home.path().join(".agents").exists()
            && !fixture.home.path().join("AGENTS.md").exists(),
        "workspace member projection must never fall back to HOME"
    );
}

#[test]
fn workspace_apply_foreign_member_conflict_rolls_back_all_members() {
    let fixture = WorkspaceFixture::new();
    let foreign = fixture.member("two").join("AGENTS.md");
    fs::write(&foreign, "foreign member instructions\n").expect("foreign member entry");
    let foreign_before = fs::read(&foreign).expect("read foreign member entry");

    let output = fixture
        .cmd()
        .args(["--json", "sync", "--apply"])
        .output()
        .expect("workspace sync conflict");

    assert_eq!(
        output.status.code(),
        Some(3),
        "any member foreign conflict must fail the workspace transaction: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(fs::read(&foreign).unwrap(), foreign_before);
    for member in ["one", "two"] {
        assert!(
            !fixture
                .member(member)
                .join(format!(".agents/skills/{member}"))
                .exists()
                && !fixture.member(member).join(".cursor/mcp.json").exists(),
            "foreign conflict must leave member {member} without managed targets"
        );
    }
    assert!(
        !fixture.member("one").join("AGENTS.md").exists(),
        "foreign conflict in another member must prevent workspace prompt writes"
    );
}

#[test]
fn workspace_runtime_failure_reports_rollback_and_not_applied_members_with_exit_three() {
    let fixture = WorkspaceFixture::new();
    for index in 0..256 {
        write(
            &fixture
                .member("one")
                .join(format!(".ai-config/skills/{index:04}-bulk/SKILL.md")),
            "---\nname: bulk\n---\ncanonical bulk skill\n",
        );
    }
    let first_target = fixture.member("one").join(".agents/skills/0000-bulk");
    let second_member_source = fixture.member("two").join(".ai-config/skills/two/SKILL.md");
    let mutation = thread::spawn({
        let first_target = first_target.clone();
        let second_member_source = second_member_source.clone();
        move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if first_target.exists() {
                    fs::write(&second_member_source, "source changed after planning\n")
                        .expect("simulate a member-two canonical source update");
                    return true;
                }
                thread::yield_now();
            }
            false
        }
    });

    let output = fixture
        .cmd()
        .args(["--json", "sync", "--apply"])
        .output()
        .expect("workspace sync with a later source change");
    assert!(
        mutation.join().expect("source editor thread"),
        "the source update must happen after the first workspace member starts staging"
    );

    assert_eq!(
        output.status.code(),
        Some(3),
        "a workspace runtime transaction failure must use exit 3: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let report = workspace_report(&output.stdout);
    let members = report["members"].as_array().expect("workspace members");
    assert!(
        members.iter().all(|member| {
            member["blocking_reason"] == "transaction_apply_failed"
                && member["apply"].is_object()
        }),
        "a global transaction failure must preserve a structured report for every member: {report:?}"
    );
    assert!(
        members[0]["apply"]["rolled_back"]
            .as_u64()
            .unwrap_or_default()
            > 0,
        "member one writes must be reported as rolled back: {report:?}"
    );
    assert!(
        members[1]["apply"]["failed"].as_u64().unwrap_or_default() > 0,
        "member two source mismatch must be reported as failed: {report:?}"
    );
    assert!(
        members[1]["apply"]["not_applied"]
            .as_u64()
            .unwrap_or_default()
            > 0,
        "later member-two actions must remain explicitly not_applied: {report:?}"
    );
    fixture.assert_no_member_targets();
}
