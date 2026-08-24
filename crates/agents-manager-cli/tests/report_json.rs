//! 集成测试：`list` / `status` / `sync` / `doctor` 的 `--json` 报告结构。

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use tempfile::TempDir;

const BIN: &str = "agents-manager";

fn setup() -> (TempDir, TempDir) {
    let home = TempDir::new().expect("home");
    let root = TempDir::new().expect("root");
    fs::create_dir_all(root.path().join("skills/foo")).unwrap();
    fs::write(root.path().join("skills/foo/SKILL.md"), "# foo\n").unwrap();
    fs::create_dir_all(root.path().join("rules")).unwrap();
    fs::write(root.path().join("rules/r1.mdc"), "---\n").unwrap();
    fs::write(
        root.path().join("mcp.json"),
        r#"{"mcpServers":{"echo":{"command":"echo","args":["hi"]}}}"#,
    )
    .unwrap();
    fs::create_dir_all(home.path().join(".cursor")).unwrap();
    (home, root)
}

fn cmd(home: &Path, root: &Path) -> Command {
    let mut c = Command::cargo_bin(BIN).expect("binary");
    c.env("HOME", home);
    c.env("USERPROFILE", home);
    c.env_remove("AGENTS_MANAGER_SECRETS_DIR");
    c.env_remove("HERMES_SKILLS_DIR");
    c.arg("--root").arg(root);
    c
}

#[test]
fn list_json_has_count_and_assets() {
    let (home, root) = setup();
    let out = cmd(home.path(), root.path())
        .args(["--json", "list"])
        .output()
        .expect("list");
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["count"].as_u64(), Some(3));
    assert_eq!(v["assets"].as_array().map(|a| a.len()), Some(3));
}

#[test]
fn status_json_has_summary_and_platform_states() {
    let (home, root) = setup();
    let out = cmd(home.path(), root.path())
        .args(["--json", "status"])
        .output()
        .expect("status");
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["summary"]["total_assets"].as_u64(), Some(3));
    let assets = v["projects"][0]["assets"].as_array().unwrap();
    let mcp = assets.iter().find(|a| a["kind"] == "mcp").expect("mcp row");
    assert_eq!(mcp["platforms"].as_array().map(|p| p.len()), Some(4));
}

#[test]
fn doctor_json_has_issues_array() {
    let (home, root) = setup();
    let out = cmd(home.path(), root.path())
        .args(["--json", "doctor"])
        .output()
        .expect("doctor");
    assert!(out.status.success() || out.status.code() == Some(3));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["missing_secrets"].is_array());
    assert!(v["platform_capability_issues"].is_array());
    assert!(v["broken"].is_number());
}

#[test]
fn sync_json_has_plan_and_no_apply_without_explicit_flag() {
    let (home, root) = setup();
    let out = cmd(home.path(), root.path())
        .args(["--json", "sync"])
        .output()
        .expect("sync");
    assert!(out.status.success() || out.status.code() == Some(3));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["plan"]["schema_version"].is_u64());
    assert!(v["plan"]["plan_digest"].is_string());
    assert!(v["plan"]["actions"].is_array());
    assert!(v.get("apply").is_none());
    assert!(
        !home.path().join(".agents/skills/foo").exists()
            && !home.path().join(".cursor/mcp.json").exists(),
        "default sync must not materialize targets"
    );
}
