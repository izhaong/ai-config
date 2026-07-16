//! 集成测试：agent API 路径（`list` / `status` / `doctor` / `sync` --json）与 `serve` 子命令。

use std::fs;
use std::path::Path;
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

use assert_cmd::Command;
use tempfile::TempDir;

const BIN: &str = "ai-config";

fn setup_project() -> (TempDir, TempDir) {
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
    c.env_remove("AI_CONFIG_SECRETS_DIR");
    c.env_remove("HERMES_SKILLS_DIR");
    c.arg("--root").arg(root);
    c
}

#[test]
fn status_json_has_summary_and_assets() {
    let (home, root) = setup_project();
    let out = cmd(home.path(), root.path())
        .args(["--json", "status"])
        .output()
        .expect("status");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert!(v["summary"]["total_assets"].as_u64().unwrap() >= 3);
    assert!(v["projects"][0]["assets"].is_array());
}

#[test]
fn sync_json_returns_a_plan_without_applying() {
    let (home, root) = setup_project();
    let out = cmd(home.path(), root.path())
        .args(["--json", "sync"])
        .output()
        .expect("sync");
    assert!(out.status.success() || out.status.code() == Some(3));

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert!(v["plan"]["schema_version"].is_u64(), "sync report={v:?}");
    assert!(v["plan"]["plan_digest"].is_string(), "sync report={v:?}");
    assert!(v["plan"]["actions"].is_array(), "sync report={v:?}");
    assert!(
        v.get("apply").is_none(),
        "sync must remain plan-only without --apply: {v:?}"
    );
    assert!(
        !home.path().join(".agents/skills/foo").exists()
            && !home.path().join(".cursor/mcp.json").exists(),
        "plan-only sync must not create platform targets"
    );
}

#[test]
fn serve_subcommand_is_registered() {
    let out = Command::cargo_bin(BIN)
        .expect("binary")
        .arg("serve")
        .arg("--help")
        .output()
        .expect("serve --help");
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("MCP") || help.contains("stdio"));
}

#[test]
fn serve_stdio_exits_when_stdin_closed() {
    let (home, root) = setup_project();
    let bin = assert_cmd::cargo::cargo_bin(BIN);
    let mut child = std::process::Command::new(bin)
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .arg("--root")
        .arg(root.path())
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn serve");

    drop(child.stdin.take());
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        if Instant::now() >= deadline {
            panic!("serve did not exit within 5s");
        }
        thread::sleep(Duration::from_millis(50));
    };
    assert!(
        status.success() || status.code() == Some(0) || status.code() == Some(5),
        "serve should exit when stdin closes (status={status:?})"
    );
}
