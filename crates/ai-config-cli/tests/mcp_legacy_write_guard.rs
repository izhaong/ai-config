//! T007 safety boundary: legacy MCP commands must not mutate user assets.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use tempfile::TempDir;

const BIN: &str = "ai-config";

fn setup() -> (TempDir, TempDir) {
    let home = TempDir::new().expect("home tempdir");
    let root = TempDir::new().expect("root tempdir");
    fs::write(
        root.path().join("mcp.json"),
        r#"{"mcpServers":{"legacy":{"command":"echo"}}}"#,
    )
    .expect("write legacy mcp.json");
    fs::create_dir_all(home.path().join(".cursor")).expect("mkdir cursor");
    (home, root)
}

fn cmd(home: &Path, root: &Path) -> Command {
    let mut command = Command::cargo_bin(BIN).expect("binary");
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("AI_CONFIG_SECRETS_DIR")
        .env_remove("HERMES_SKILLS_DIR")
        .arg("--root")
        .arg(root);
    command
}

#[test]
fn legacy_mcp_platform_write_commands_refuse_without_touching_source_or_platform() {
    let (home, root) = setup();
    let source = root.path().join("mcp.json");
    let source_before = fs::read(&source).expect("read source before");
    let target = home.path().join(".cursor/mcp.json");

    for args in [
        vec!["mcp", "deploy", "legacy", "cursor"],
        vec!["mcp", "retract", "legacy", "cursor"],
    ] {
        let assert = cmd(home.path(), root.path())
            .args(&args)
            .assert()
            .failure()
            .code(2);
        let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
        assert!(
            stderr.contains("source-first") && stderr.contains("拒绝写入"),
            "{args:?} should fail closed, got: {stderr}"
        );
    }

    assert_eq!(fs::read(&source).expect("read source after"), source_before);
    assert!(
        !target.exists(),
        "legacy commands must not materialize platform mcp.json"
    );
}
