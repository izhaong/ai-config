//! 集成测试：agent API 路径（`list` / `status` / `doctor` / `sync` --json）与 `serve` 子命令。

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::Stdio;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use assert_cmd::Command;
use tempfile::TempDir;

const BIN: &str = "agents-manager";

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
    c.env_remove("AGENT_MANAGER_SECRETS_DIR");
    c.env_remove("HERMES_SKILLS_DIR");
    c.arg("--root").arg(root);
    c
}

fn mcp_sync_plan(home: &Path, root: &Path) -> serde_json::Value {
    let bin = assert_cmd::cargo::cargo_bin(BIN);
    let mut child = std::process::Command::new(bin)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("AGENT_MANAGER_SECRETS_DIR")
        .env_remove("HERMES_SKILLS_DIR")
        .arg("--root")
        .arg(root)
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn MCP bridge");
    let mut stdin = child.stdin.take().expect("MCP stdin");
    let stdout = child.stdout.take().expect("MCP stdout");
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let line = line.expect("MCP stdout line");
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                sender.send(value).expect("MCP response receiver");
            }
        }
    });

    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "contract-test", "version": "1.0" }
            }
        })
    )
    .expect("send MCP initialize");
    stdin.flush().expect("flush MCP initialize");
    let initialized = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("MCP initialize response");
    assert_eq!(initialized["id"], 1, "initialize response={initialized:?}");

    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        })
    )
    .expect("send MCP initialized notification");
    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "agent_manager_sync",
                "arguments": { "root": root.to_string_lossy(), "apply": false }
            }
        })
    )
    .expect("send MCP sync call");
    stdin.flush().expect("flush MCP sync call");
    let called = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("MCP sync response");
    assert_eq!(called["id"], 2, "sync response={called:?}");
    assert!(called.get("error").is_none(), "sync response={called:?}");

    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if child.try_wait().expect("poll MCP bridge").is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().expect("stop timed-out MCP bridge");
            panic!("MCP bridge did not stop after stdin closed");
        }
        thread::sleep(Duration::from_millis(25));
    }
    reader.join().expect("join MCP stdout reader");

    let text = called["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("MCP sync result has no text content: {called:?}"));
    let report: serde_json::Value = serde_json::from_str(text).expect("MCP lifecycle report JSON");
    report["plan"].clone()
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
fn cli_and_mcp_sync_return_the_same_projection_plan_identity() {
    let (home, root) = setup_project();
    let cli = cmd(home.path(), root.path())
        .args(["--json", "sync"])
        .output()
        .expect("CLI sync plan");
    assert!(
        cli.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&cli.stderr)
    );
    let cli_report: serde_json::Value =
        serde_json::from_slice(&cli.stdout).expect("CLI lifecycle report JSON");
    let cli_plan = &cli_report["plan"];
    let mcp_plan = mcp_sync_plan(home.path(), root.path());

    assert_eq!(mcp_plan["schema_version"], cli_plan["schema_version"]);
    assert_eq!(mcp_plan["plan_digest"], cli_plan["plan_digest"]);
    assert_eq!(
        mcp_plan["actions"], cli_plan["actions"],
        "the bridge must preserve action and managed-member identity"
    );
}

#[test]
fn install_is_plan_only_by_default_and_requires_explicit_apply() {
    let (home, root) = setup_project();
    fs::remove_file(root.path().join("rules/r1.mdc"))
        .expect("install contract uses only an independently supported skill");
    let target = home.path().join(".agents/skills/foo");
    let ledger = home.path().join(".agents-manager/projection-ledger.sqlite");

    let planned = cmd(home.path(), root.path())
        .args(["--json", "install"])
        .output()
        .expect("install plan");
    assert!(
        planned.status.success(),
        "install plan stderr={}",
        String::from_utf8_lossy(&planned.stderr)
    );
    let plan_report: serde_json::Value =
        serde_json::from_slice(&planned.stdout).expect("install plan JSON");
    assert!(plan_report["plan"]["plan_digest"].is_string());
    assert!(plan_report.get("apply").is_none());
    assert!(!target.exists(), "default install must not project assets");
    assert!(
        !ledger.exists(),
        "default install must not initialize ownership state"
    );

    let applied = cmd(home.path(), root.path())
        .args(["--json", "install", "--apply"])
        .output()
        .expect("install apply");
    assert!(
        applied.status.success(),
        "install apply stdout={} stderr={}",
        String::from_utf8_lossy(&applied.stdout),
        String::from_utf8_lossy(&applied.stderr)
    );
    let apply_report: serde_json::Value =
        serde_json::from_slice(&applied.stdout).expect("install apply JSON");
    assert!(
        apply_report["apply"]["changed"]
            .as_u64()
            .unwrap_or_default()
            > 0
    );
    assert!(
        target.exists(),
        "install --apply must project the reviewed plan"
    );
    assert!(
        ledger.is_file(),
        "install --apply must persist ownership state"
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
