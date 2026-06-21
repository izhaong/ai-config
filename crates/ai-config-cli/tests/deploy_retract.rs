//! 集成测试:整文件 `mcp.json` deploy / retract。

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use tempfile::TempDir;

const BIN: &str = "ai-config";

fn setup() -> (TempDir, TempDir) {
    let home = TempDir::new().expect("home tempdir");
    let root = TempDir::new().expect("root tempdir");
    fs::write(
        root.path().join("mcp.json"),
        r#"{
  "mcpServers": {
    "minio": {
      "command": "docker",
      "env": { "ENDPOINT": "minio.example.com:443" }
    }
  }
}"#,
    )
    .expect("write mcp.json");
    fs::create_dir_all(home.path().join(".cursor")).expect("mkdir .cursor");
    fs::create_dir_all(home.path().join(".codex")).expect("mkdir .codex");
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
fn deploy_copies_mcp_json_to_platform() {
    let (home, root) = setup();
    let cursor_mcp = home.path().join(".cursor").join("mcp.json");
    assert!(!cursor_mcp.exists());

    cmd(home.path(), root.path())
        .args(["mcp", "deploy", "minio", "cursor"])
        .assert()
        .success();

    let content = fs::read_to_string(&cursor_mcp).expect("mcp.json created");
    let v: serde_json::Value = serde_json::from_str(&content).expect("valid JSON");
    let mcp = v["mcpServers"].as_object().expect("mcpServers object");
    assert!(mcp.contains_key("minio"));
    assert_eq!(
        mcp["minio"]["env"]["ENDPOINT"].as_str().unwrap(),
        "minio.example.com:443"
    );
}

#[test]
fn retract_removes_platform_mcp_json() {
    let (home, root) = setup();
    let cursor_mcp = home.path().join(".cursor").join("mcp.json");

    cmd(home.path(), root.path())
        .args(["mcp", "deploy", "minio", "cursor"])
        .assert()
        .success();
    assert!(cursor_mcp.exists());

    cmd(home.path(), root.path())
        .args(["mcp", "retract", "minio", "cursor"])
        .assert()
        .success();

    assert!(
        !cursor_mcp.exists() || {
            let raw = fs::read_to_string(&cursor_mcp).unwrap_or_default();
            let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::json!({}));
            v.get("mcpServers").and_then(|m| m.get("minio")).is_none()
        },
        "retract should remove minio server entry from platform mcp.json"
    );
}

#[test]
fn deploy_does_not_touch_other_platforms() {
    let (home, root) = setup();
    let cursor_mcp = home.path().join(".cursor").join("mcp.json");
    let codex_mcp = home.path().join(".codex").join("mcp.json");

    let codex_before = r#"{
  "mcpServers": { "codex-only": { "command": "echo" } }
}
"#;
    fs::write(&codex_mcp, codex_before).unwrap();

    cmd(home.path(), root.path())
        .args(["mcp", "deploy", "minio", "cursor"])
        .assert()
        .success();

    assert!(cursor_mcp.exists());
    let codex_after = fs::read_to_string(&codex_mcp).unwrap();
    assert_eq!(codex_after, codex_before);
}

#[test]
fn deploy_merges_into_existing_platform_mcp_json() {
    let (home, root) = setup();
    let cursor_mcp = home.path().join(".cursor").join("mcp.json");
    fs::write(
        &cursor_mcp,
        r#"{"mcpServers":{"user-thing":{"command":"echo"}}}"#,
    )
    .unwrap();

    cmd(home.path(), root.path())
        .args(["mcp", "deploy", "minio", "cursor"])
        .assert()
        .success();

    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&cursor_mcp).unwrap()).unwrap();
    let mcp = v["mcpServers"].as_object().unwrap();
    assert!(mcp.contains_key("minio"));
    assert!(
        mcp.contains_key("user-thing"),
        "per-server deploy must preserve other platform entries"
    );
}

#[test]
fn deploy_hermes_writes_config_yaml_preserving_other_keys() {
    let (home, root) = setup();
    let hermes_dir = home.path().join(".hermes");
    fs::create_dir_all(&hermes_dir).unwrap();
    let config_yaml = hermes_dir.join("config.yaml");
    fs::write(&config_yaml, "model: gpt-test\n").unwrap();

    cmd(home.path(), root.path())
        .args(["mcp", "deploy", "minio", "hermes"])
        .assert()
        .success();

    let content = fs::read_to_string(&config_yaml).expect("config.yaml updated");
    assert!(content.contains("model: gpt-test"));
    assert!(content.contains("mcp_servers:"));
    assert!(content.contains("minio:"));
    assert!(content.contains("minio.example.com:443"));
}

#[test]
fn retract_hermes_removes_server_only() {
    let (home, root) = setup();
    let hermes_dir = home.path().join(".hermes");
    fs::create_dir_all(&hermes_dir).unwrap();
    let config_yaml = hermes_dir.join("config.yaml");
    fs::write(&config_yaml, "model: gpt-test\n").unwrap();

    cmd(home.path(), root.path())
        .args(["mcp", "deploy", "minio", "hermes"])
        .assert()
        .success();

    cmd(home.path(), root.path())
        .args(["mcp", "retract", "minio", "hermes"])
        .assert()
        .success();

    let content = fs::read_to_string(&config_yaml).expect("config.yaml still exists");
    assert!(content.contains("model: gpt-test"));
    assert!(!content.contains("minio:"));
}

#[test]
fn migrate_hermes_merges_legacy_mcp_json() {
    let (home, root) = setup();
    let hermes_dir = home.path().join(".hermes");
    fs::create_dir_all(&hermes_dir).unwrap();
    fs::write(
        hermes_dir.join("mcp.json"),
        r#"{"mcpServers":{"legacy-svc":{"command":"echo","args":["legacy"]}}}"#,
    )
    .unwrap();

    cmd(home.path(), root.path())
        .args(["mcp", "migrate-hermes"])
        .assert()
        .success();

    let config_yaml = hermes_dir.join("config.yaml");
    let content = fs::read_to_string(&config_yaml).expect("config.yaml created");
    assert!(content.contains("legacy-svc:"));
    assert!(
        !hermes_dir.join("mcp.json").exists(),
        "legacy mcp.json should be renamed after migrate"
    );
}

#[test]
fn secrets_validate_always_ok_without_mcp_placeholders() {
    let (home, root) = setup();
    cmd(home.path(), root.path())
        .args(["secrets", "validate"])
        .assert()
        .success();
}

#[test]
fn secrets_list_only_keys_no_values() {
    let (home, root) = setup();
    let cfg = home.path().join(".config").join("ai-config");
    fs::create_dir_all(&cfg).unwrap();
    fs::write(cfg.join("secrets.env"), "MINIO_ENDPOINT=secret-value\n").unwrap();

    cmd(home.path(), root.path())
        .args(["--json", "secrets", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("\"keys\""))
        .stdout(predicates::str::contains("\"MINIO_ENDPOINT\""))
        .stdout(predicates::str::contains("secret-value").not());
}
