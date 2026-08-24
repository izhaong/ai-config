//! T009 public lifecycle contract: CLI must expose the core projection plan before it writes.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use agents_manager_core::projection::ledger::ProjectionLedger;
use agents_manager_store::Store;
use assert_cmd::Command;
use camino::Utf8PathBuf;
use serde_json::Value;
use tempfile::TempDir;

const BIN: &str = "agents-manager";
const MISSING_SECRET_KEY: &str = "T009_MISSING_CATALOG_TOKEN";
const SECRET_VALUE_SENTINEL: &str = "t009-secret-value-must-not-leak";

struct Fixture {
    home: TempDir,
    repo: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let home = TempDir::new().expect("temporary HOME");
        let repo = TempDir::new().expect("temporary project");
        let root = repo.path().join(".agents-manager");

        write(
            &root.join("skills/demo/SKILL.md"),
            "---\nname: demo\n---\ncanonical skill\n",
        );
        write(
            &root.join("mcp/servers/catalog.json"),
            r#"{"enabled":true,"targets":["cursor"],"config":{"command":"catalog-mcp"}}"#,
        );
        write(
            &root.join("hooks/lint.sh"),
            "#!/bin/sh\necho canonical hook\n",
        );
        write(
            &root.join("prompts/AGENTS.md"),
            "canonical project instructions\n",
        );

        Self { home, repo }
    }

    fn root(&self) -> &Path {
        self.repo.path()
    }

    fn asset_root(&self) -> PathBuf {
        self.root().join(".agents-manager")
    }

    fn cmd(&self) -> Command {
        let mut command = Command::cargo_bin(BIN).expect("CLI binary");
        command
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .env_remove("AGENTS_MANAGER_ROOT")
            .env_remove("AGENTS_MANAGER_SECRETS_DIR")
            .env_remove("HERMES_SKILLS_DIR")
            .arg("--root")
            .arg(self.root());
        command
    }

    fn source_bytes(&self) -> Vec<(PathBuf, Vec<u8>)> {
        [
            "skills/demo/SKILL.md",
            "mcp/servers/catalog.json",
            "hooks/lint.sh",
            "prompts/AGENTS.md",
        ]
        .into_iter()
        .map(|relative| {
            let path = self.asset_root().join(relative);
            let bytes = fs::read(&path).expect("canonical source remains readable");
            (path, bytes)
        })
        .collect()
    }

    fn assert_no_projection_targets(&self) {
        for path in [
            self.root().join(".agents/skills/demo"),
            self.root().join(".cursor/mcp.json"),
            self.root().join(".cursor/hooks/lint.sh"),
            self.root().join(".cursor/hooks.json"),
            self.root().join("AGENTS.md"),
        ] {
            assert!(
                !path.exists(),
                "plan-only sync must not create projection target {}",
                path.display()
            );
        }
    }

    fn configure_missing_mcp_secret(&self) {
        write(
            &self.asset_root().join("mcp/servers/catalog.json"),
            &format!(
                r#"{{
  "enabled": true,
  "targets": ["cursor"],
  "config": {{
    "command": "catalog-mcp",
    "env": {{ "{MISSING_SECRET_KEY}": "${{{MISSING_SECRET_KEY}}}" }}
  }}
}}"#
            ),
        );
        let secret_path =
            Utf8PathBuf::from_path_buf(self.home.path().join(".config/agents-manager/secrets.env"))
                .expect("temporary secret path is UTF-8");
        agents_manager_core::secrets::save_to(
            &[(
                "UNRELATED_TEST_SECRET".to_owned(),
                SECRET_VALUE_SENTINEL.to_owned(),
            )],
            &secret_path,
        )
        .expect("seed strict temporary secret store");
    }
}

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().expect("fixture path parent")).unwrap();
    fs::write(path, content).unwrap();
}

fn projection_plan(output: &[u8]) -> Value {
    let report: Value = serde_json::from_slice(output).expect("projection report JSON");
    let plan = report
        .get("plan")
        .expect("projection lifecycle JSON must expose a plan");
    assert!(
        plan["schema_version"].is_u64(),
        "plan must include schema_version"
    );
    assert!(
        plan["plan_digest"].is_string(),
        "plan must include deterministic plan_digest"
    );
    let actions = plan["actions"]
        .as_array()
        .expect("plan must include action summaries");
    assert!(
        actions.iter().any(|action| {
            action["members"]
                .as_array()
                .is_some_and(|members| !members.is_empty())
                || action["mcp_members"]
                    .as_array()
                    .is_some_and(|members| !members.is_empty())
        }),
        "at least one action must summarize its managed members"
    );
    plan.clone()
}

#[test]
fn sync_without_apply_returns_a_plan_and_leaves_all_projection_targets_absent() {
    let fixture = Fixture::new();
    let source_before = fixture.source_bytes();

    let output = fixture
        .cmd()
        .args(["--json", "sync"])
        .output()
        .expect("run sync");

    assert!(
        output.status.success(),
        "default sync should plan successfully: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _plan = projection_plan(&output.stdout);
    fixture.assert_no_projection_targets();
    for (path, before) in source_before {
        assert_eq!(
            fs::read(path).unwrap(),
            before,
            "plan must not mutate source"
        );
    }
}

#[test]
fn sync_apply_projects_direct_mcp_hook_and_prompt_through_one_projection_plan() {
    let fixture = Fixture::new();

    let output = fixture
        .cmd()
        .args(["--json", "sync", "--apply"])
        .output()
        .expect("run sync --apply");

    assert!(
        output.status.success(),
        "sync --apply should execute the approved plan: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let _plan = projection_plan(&output.stdout);
    assert!(
        fixture.root().join(".agents/skills/demo").exists(),
        "direct asset must be projected"
    );
    assert!(
        fixture.root().join(".cursor/mcp.json").is_file(),
        "MCP must be rendered as a generated container entry"
    );
    assert!(
        fixture.root().join(".cursor/hooks/lint.sh").exists(),
        "Hook script must be projected as a direct unit"
    );
    assert!(
        fixture.root().join(".cursor/hooks.json").is_file(),
        "Hook binding must be rendered as a generated container entry"
    );
    assert!(
        fixture.root().join("AGENTS.md").exists(),
        "Prompt must be projected once as the project entry"
    );
}

#[test]
fn sync_apply_missing_mcp_secret_exits_four_reports_key_without_value_and_applies_direct_assets() {
    let fixture = Fixture::new();
    fixture.configure_missing_mcp_secret();

    let output = fixture
        .cmd()
        .args(["--json", "sync", "--apply"])
        .output()
        .expect("run sync --apply with a missing MCP secret");

    assert_eq!(
        output.status.code(),
        Some(4),
        "missing MCP secrets must use the secrets exit code: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains(SECRET_VALUE_SENTINEL) && !stderr.contains(SECRET_VALUE_SENTINEL),
        "neither lifecycle report nor diagnostic may expose secret values"
    );

    let report: Value = serde_json::from_slice(&output.stdout).expect("lifecycle report JSON");
    assert_eq!(report["blocking_reason"], "mcp_missing_secret_keys");
    assert!(
        report["apply"]["skipped"].as_u64().unwrap_or_default() > 0,
        "the missing MCP server must be represented as skipped: {report:?}"
    );
    assert_eq!(
        report["apply"]["mcp_skipped_members"][0]["missing_secret_keys"],
        serde_json::json!([MISSING_SECRET_KEY]),
        "the apply report may expose only the missing key name"
    );
    assert!(
        report["plan"]["actions"]
            .as_array()
            .is_some_and(|actions| actions.iter().any(|action| {
                action["reason_code"] == "mcp_missing_secret_keys"
                    && action["state"] == "skipped"
                    && action["mcp_members"][0]["missing_secret_keys"]
                        == serde_json::json!([MISSING_SECRET_KEY])
            })),
        "the reviewed plan must make the MCP-only skip inspectable"
    );
    assert!(
        fixture.root().join(".agents/skills/demo").exists(),
        "a missing MCP secret must not prevent unrelated direct assets from applying"
    );
    assert!(
        !fixture.root().join(".cursor/mcp.json").exists(),
        "the missing MCP server must not create a generated container"
    );
}

#[test]
fn second_sync_apply_reports_unchanged_actions() {
    let fixture = Fixture::new();

    let first = fixture
        .cmd()
        .args(["--json", "sync", "--apply"])
        .output()
        .expect("first sync --apply");
    assert!(
        first.status.success(),
        "first apply must succeed: stdout={} stderr={}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
    );

    let second = fixture
        .cmd()
        .args(["--json", "sync", "--apply"])
        .output()
        .expect("second sync --apply");
    assert!(
        second.status.success(),
        "second apply must succeed: stdout={} stderr={}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr),
    );
    let _plan = projection_plan(&second.stdout);
    let report: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert!(
        report["apply"]["unchanged"].as_u64().unwrap_or_default() > 0,
        "second apply must report unchanged actions instead of rewriting targets"
    );
}

#[test]
fn sync_apply_foreign_project_entry_exits_three_without_partial_writes() {
    let fixture = Fixture::new();
    write(&fixture.root().join("AGENTS.md"), "foreign instructions\n");
    let source_before = fixture.source_bytes();
    let foreign_before = fs::read(fixture.root().join("AGENTS.md")).unwrap();

    let output = fixture
        .cmd()
        .args(["--json", "sync", "--apply"])
        .output()
        .expect("run sync --apply with foreign entry");

    assert_eq!(
        output.status.code(),
        Some(3),
        "a blocking foreign conflict must use the projection conflict exit code"
    );
    assert_eq!(
        fs::read(fixture.root().join("AGENTS.md")).unwrap(),
        foreign_before,
        "foreign entry must be preserved"
    );
    assert!(
        !fixture.root().join(".agents/skills/demo").exists()
            && !fixture.root().join(".cursor/mcp.json").exists()
            && !fixture.root().join(".cursor/hooks/lint.sh").exists(),
        "a blocking conflict must leave the whole plan unapplied"
    );
    for (path, before) in source_before {
        assert_eq!(
            fs::read(path).unwrap(),
            before,
            "conflict must not mutate source"
        );
    }
}

#[test]
fn sync_apply_unopenable_ledger_path_exits_five_with_a_redacted_json_error_envelope() {
    let fixture = Fixture::new();
    fixture.configure_missing_mcp_secret();
    fs::create_dir_all(fixture.root().join(".agents-manager/projection-ledger.sqlite"))
        .expect("make the SQLite ledger path a directory");

    let output = fixture
        .cmd()
        .args(["--json", "sync", "--apply"])
        .output()
        .expect("run sync --apply with an unopenable ledger path");

    assert_eq!(
        output.status.code(),
        Some(5),
        "a filesystem-backed ledger open failure must use exit 5: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains(SECRET_VALUE_SENTINEL) && !stderr.contains(SECRET_VALUE_SENTINEL),
        "filesystem error output must never expose a secret value"
    );
    let envelope: Value = serde_json::from_slice(&output.stdout)
        .expect("filesystem failures must retain the JSON error envelope");
    assert_eq!(envelope["error"]["code"], 5);
    fixture.assert_no_projection_targets();
}

#[test]
fn uninstall_without_apply_returns_a_plan_and_preserves_source_and_foreign_container() {
    let fixture = Fixture::new();
    let source_before = fixture.source_bytes();
    let foreign_container = fixture.root().join(".cursor/mcp.json");
    write(
        &foreign_container,
        r#"{"mcpServers":{"foreign":{"command":"echo"}},"unknown":true}"#,
    );
    let foreign_before = fs::read(&foreign_container).unwrap();

    let output = fixture
        .cmd()
        .args(["--json", "uninstall"])
        .output()
        .expect("run uninstall");

    assert!(
        output.status.success(),
        "default uninstall should plan successfully: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _plan = projection_plan(&output.stdout);
    assert_eq!(
        fs::read(&foreign_container).unwrap(),
        foreign_before,
        "uninstall plan must preserve foreign generated container bytes"
    );
    for (path, before) in source_before {
        assert_eq!(
            fs::read(path).unwrap(),
            before,
            "uninstall plan must preserve canonical source"
        );
    }
}

/// A source change after planning is a real runtime failure: it is not a synthetic executor
/// hook. The normal plan creates many direct targets so the external editor has a deterministic
/// window to save the MCP source before the later MCP-only sub-plan hydrates it.
#[test]
fn later_mcp_source_change_rolls_back_all_preceding_non_hermes_subplans() {
    let fixture = Fixture::new();
    let asset_root = fixture.asset_root();
    for index in 0..768 {
        write(
            &asset_root.join(format!("skills/{index:04}-bulk/SKILL.md")),
            "---\nname: bulk\n---\ncanonical bulk skill\n",
        );
    }

    let first_target = fixture.root().join(".agents/skills/0000-bulk");
    let mcp_source = asset_root.join("mcp/servers/catalog.json");
    let mutation = thread::spawn({
        let first_target = first_target.clone();
        let mcp_source = mcp_source.clone();
        move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if first_target.exists() {
                    fs::write(&mcp_source, "{not valid canonical MCP JSON\n")
                        .expect("simulate external source save after plan creation");
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
        .expect("run sync while an external editor updates the canonical MCP source");
    assert!(
        mutation.join().expect("source editor thread"),
        "the source mutation must occur after the normal sub-plan started"
    );
    assert_eq!(
        output.status.code(),
        Some(3),
        "a later runtime transaction failure must use the partial-failure exit code: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains(SECRET_VALUE_SENTINEL) && !stderr.contains(SECRET_VALUE_SENTINEL),
        "a transaction failure report must not expose secret values"
    );
    let report: Value = serde_json::from_slice(&output.stdout)
        .expect("a runtime transaction failure must still return the lifecycle JSON report");
    assert_eq!(report["blocking_reason"], "transaction_apply_failed");
    assert!(
        report["apply"]["failed"].as_u64().unwrap_or_default() > 0,
        "the failed action must be visible in the lifecycle report: {report:?}"
    );
    assert!(
        report["apply"]["rolled_back"].as_u64().unwrap_or_default() > 0,
        "actions reverted by the global transaction must not remain successful: {report:?}"
    );
    assert_eq!(
        report["apply"]["not_applied"].as_u64(),
        Some(0),
        "this fixture reaches the only failing final MCP action, so its exact report must not invent not_applied actions: {report:?}"
    );

    for target in [
        fixture.root().join(".agents/skills/0000-bulk"),
        fixture.root().join(".agents/skills/demo"),
        fixture.root().join(".cursor/hooks/lint.sh"),
        fixture.root().join(".cursor/hooks.json"),
        fixture.root().join("AGENTS.md"),
        fixture.root().join(".cursor/mcp.json"),
    ] {
        assert!(
            !target.exists(),
            "a later sub-plan failure must globally roll back {}",
            target.display()
        );
    }

    let ledger_path = fixture.root().join(".agents-manager/projection-ledger.sqlite");
    let store = Store::open_at(&ledger_path).expect("open lifecycle ledger after failed apply");
    let scope = format!(
        "project:{}",
        Utf8PathBuf::from_path_buf(fixture.root().to_path_buf()).expect("utf8 project root")
    );
    assert!(
        store
            .projections()
            .list_scope(&scope)
            .expect("read projection ledger")
            .is_empty(),
        "a later sub-plan failure must not leave earlier ownership records"
    );
}
