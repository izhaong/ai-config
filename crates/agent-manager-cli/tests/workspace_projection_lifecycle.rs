//! T009 workspace lifecycle contract: members are planned together and applied atomically.

use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use agent_manager_core::projection::fingerprint::path_content_digest;
use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

const BIN: &str = "agents-manager";

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
            let asset_root = root.join(format!("members/{member}/.agents-manager"));
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
            .env_remove("AGENT_MANAGER_ROOT")
            .env_remove("AGENT_MANAGER_SECRETS_DIR")
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

    fn configure_three_layer_overlay(&self) {
        for member in ["one", "two"] {
            let assets = self.member(member).join(".agents-manager");
            fs::remove_dir_all(&assets).expect("remove default member assets");
        }

        write(
            &self
                .home
                .path()
                .join(".agents-manager/skills/global-default/SKILL.md"),
            "---\nname: global default\n---\nglobal default\n",
        );
        write(
            &self
                .workspace
                .path()
                .join(".agents-manager/skills/shared/SKILL.md"),
            "---\nname: workspace shared\n---\nworkspace shared\n",
        );
        write(
            &self
                .workspace
                .path()
                .join(".agents-manager/skills/workspace-default/SKILL.md"),
            "---\nname: workspace default\n---\nworkspace default\n",
        );
        write(
            &self.member("one").join(".agents-manager/skills/shared/SKILL.md"),
            "---\nname: project shared\n---\nproject shared\n",
        );
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

fn action_for_skill<'a>(member: &'a Value, skill: &str) -> &'a Value {
    member["plan"]["actions"]
        .as_array()
        .expect("plan actions")
        .iter()
        .find(|action| {
            action["members"].as_array().is_some_and(|members| {
                members
                    .iter()
                    .any(|entry| entry["id"]["kind"] == "skill" && entry["id"]["name"] == skill)
            })
        })
        .unwrap_or_else(|| panic!("member plan must include skill {skill}: {member:?}"))
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
fn workspace_plan_overlays_workspace_defaults_for_local_and_empty_members_without_home_targets() {
    let fixture = WorkspaceFixture::new();
    fixture.configure_three_layer_overlay();

    let output = fixture
        .cmd()
        .args(["--json", "sync"])
        .output()
        .expect("workspace overlay sync plan");

    assert!(
        output.status.success(),
        "workspace overlay plan must succeed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let report = workspace_report(&output.stdout);
    let members = report["members"].as_array().expect("workspace members");
    let member = |name: &str| {
        let expected_member = fixture.member(name).to_string_lossy().into_owned();
        members
            .iter()
            .find(|member| member["member"].as_str() == Some(expected_member.as_str()))
            .unwrap_or_else(|| panic!("workspace plan must include member {name}: {report:?}"))
    };
    let assert_source_and_member_target =
        |member: &Value, member_name: &str, skill: &str, layer: &str, source: &Path| {
            let action = action_for_skill(member, skill);
            let expected_source = source.to_string_lossy().into_owned();
            let member_root = fixture.member(member_name).to_string_lossy().into_owned();
            let source_ref = action["members"]
                .as_array()
                .expect("action members")
                .iter()
                .find(|entry| entry["id"]["name"] == skill)
                .expect("skill source member");
            assert_eq!(source_ref["source"]["layer"], layer, "action={action:?}");
            assert_eq!(
                source_ref["source"]["absolute_path"].as_str(),
                Some(expected_source.as_str()),
                "action={action:?}"
            );
            assert!(
                action["target"]["path"]
                    .as_str()
                    .is_some_and(|target| target.starts_with(&member_root)),
                "workspace source must still deploy into member {member_name}: {action:?}"
            );
        };

    let one = member("one");
    assert_source_and_member_target(
        one,
        "one",
        "shared",
        "project",
        &fixture.member("one").join(".agents-manager/skills/shared"),
    );
    assert_source_and_member_target(
        one,
        "one",
        "workspace-default",
        "workspace",
        &fixture
            .workspace
            .path()
            .join(".agents-manager/skills/workspace-default"),
    );
    assert_source_and_member_target(
        one,
        "one",
        "global-default",
        "global",
        &fixture.home.path().join(".agents-manager/skills/global-default"),
    );

    let two = member("two");
    assert_source_and_member_target(
        two,
        "two",
        "shared",
        "workspace",
        &fixture.workspace.path().join(".agents-manager/skills/shared"),
    );
    assert_source_and_member_target(
        two,
        "two",
        "workspace-default",
        "workspace",
        &fixture
            .workspace
            .path()
            .join(".agents-manager/skills/workspace-default"),
    );
    assert_source_and_member_target(
        two,
        "two",
        "global-default",
        "global",
        &fixture.home.path().join(".agents-manager/skills/global-default"),
    );
    assert!(
        !fixture.home.path().join(".agents").exists()
            && !fixture.home.path().join("AGENTS.md").exists()
            && !fixture
                .home
                .path()
                .join(".agents-manager/projection-ledger.sqlite")
                .exists(),
        "plan-only workspace overlay must not write HOME targets or a ledger"
    );
    fixture.assert_no_member_targets();
}

#[test]
fn workspace_apply_uses_three_layer_overlay_for_each_member_without_writing_home() {
    let fixture = WorkspaceFixture::new();
    fixture.configure_three_layer_overlay();
    let home = camino::Utf8Path::from_path(fixture.home.path()).expect("UTF-8 temporary HOME");
    let home_before = path_content_digest(home).expect("digest HOME before apply");

    let output = fixture
        .cmd()
        .args(["--json", "sync", "--apply"])
        .output()
        .expect("workspace overlay sync apply");

    assert!(
        output.status.success(),
        "workspace overlay apply must succeed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    workspace_report(&output.stdout);

    let assert_member_skill_link = |member: &str, skill: &str, source: &Path| {
        let target = fixture
            .member(member)
            .join(format!(".agents/skills/{skill}"));
        assert_eq!(
            fs::read_link(&target).expect("projected skill must be a direct link"),
            source,
            "member {member} must project {skill} from its effective source"
        );
    };
    let workspace_assets = fixture.workspace.path().join(".agents-manager/skills");
    let global_assets = fixture.home.path().join(".agents-manager/skills");
    assert_member_skill_link(
        "one",
        "shared",
        &fixture.member("one").join(".agents-manager/skills/shared"),
    );
    assert_member_skill_link("two", "shared", &workspace_assets.join("shared"));
    for member in ["one", "two"] {
        assert_member_skill_link(
            member,
            "workspace-default",
            &workspace_assets.join("workspace-default"),
        );
        assert_member_skill_link(
            member,
            "global-default",
            &global_assets.join("global-default"),
        );
    }

    assert_eq!(
        path_content_digest(home).expect("digest HOME after apply"),
        home_before,
        "workspace apply must not modify HOME sources, targets, or ledger"
    );
    assert!(
        !fixture.home.path().join(".agents").exists()
            && !fixture.home.path().join("AGENTS.md").exists()
            && !fixture
                .home
                .path()
                .join(".agents-manager/projection-ledger.sqlite")
                .exists(),
        "workspace apply must not create HOME targets or a ledger"
    );
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
                .join(format!(".agents-manager/skills/{index:04}-bulk/SKILL.md")),
            "---\nname: bulk\n---\ncanonical bulk skill\n",
        );
    }
    let first_target = fixture.member("one").join(".agents/skills/0000-bulk");
    let second_member_source = fixture.member("two").join(".agents-manager/skills/two/SKILL.md");
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
