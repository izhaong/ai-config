//! 集成测试:整文件 `mcp.json` deploy / retract。

use std::fs;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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
fn deploy_refuses_legacy_write_without_creating_platform_file() {
    let (home, root) = setup();
    let cursor_mcp = home.path().join(".cursor").join("mcp.json");
    let source = root.path().join("mcp.json");
    let source_before = fs::read(&source).expect("read source before");
    assert!(!cursor_mcp.exists());

    cmd(home.path(), root.path())
        .args(["mcp", "deploy", "minio", "cursor"])
        .assert()
        .failure()
        .code(2);

    assert!(
        !cursor_mcp.exists(),
        "deploy must not materialize legacy MCP"
    );
    assert_eq!(fs::read(source).unwrap(), source_before);
}

#[test]
fn retract_preserves_platform_mcp_without_ownership_evidence() {
    let (home, root) = setup();
    let cursor_mcp = home.path().join(".cursor").join("mcp.json");
    let before = r#"{"mcpServers":{"minio":{"command":"docker"}}}"#;
    fs::write(&cursor_mcp, before).unwrap();

    cmd(home.path(), root.path())
        .args(["mcp", "retract", "minio", "cursor"])
        .assert()
        .failure()
        .code(2);

    let raw = fs::read_to_string(&cursor_mcp).expect("platform mcp.json preserved");
    assert_eq!(raw, before);
}

#[test]
fn uninstall_preserves_platform_mcp_file_and_foreign_servers() {
    let (home, root) = setup();
    let cursor_mcp = home.path().join(".cursor").join("mcp.json");
    fs::write(
        &cursor_mcp,
        r#"{"mcpServers":{"minio":{"command":"docker"},"foreign":{"command":"echo"}},"extra":true}"#,
    )
    .unwrap();

    cmd(home.path(), root.path())
        .arg("uninstall")
        .assert()
        .success();

    let doc: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&cursor_mcp).unwrap()).unwrap();
    assert!(doc["mcpServers"].get("minio").is_some());
    assert!(doc["mcpServers"].get("foreign").is_some());
    assert_eq!(doc["extra"], true);
}

#[test]
fn status_does_not_initialize_global_asset_root() {
    let home = TempDir::new().unwrap();
    let mut command = Command::cargo_bin(BIN).unwrap();
    command
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env_remove("AI_CONFIG_ROOT")
        .arg("status")
        .assert()
        .success();

    assert!(
        !home.path().join(".ai-config").exists(),
        "status must not create or seed the global asset root"
    );
}

#[test]
fn project_sync_apply_writes_mcp_under_project_deploy_base() {
    let home = TempDir::new().unwrap();
    let repo = TempDir::new().unwrap();
    let asset_root = repo.path().join(".ai-config");
    fs::create_dir_all(asset_root.join("mcp/servers")).unwrap();
    fs::write(
        asset_root.join("mcp/servers/project-only.json"),
        r#"{"enabled":true,"targets":["cursor"],"config":{"command":"echo"}}"#,
    )
    .unwrap();

    cmd(home.path(), repo.path())
        .args(["sync", "--apply"])
        .assert()
        .success();

    assert!(
        repo.path().join(".cursor/mcp.json").is_file(),
        "project sync must render MCP into the project platform directory"
    );
    assert!(
        !home.path().join(".cursor/mcp.json").exists(),
        "project sync must not write MCP into HOME"
    );
}

#[test]
fn doctor_materialize_is_rejected_without_writing() {
    let (home, root) = setup();
    let cursor_mcp = home.path().join(".cursor").join("mcp.json");
    fs::write(
        &cursor_mcp,
        r#"{"mcpServers":{"foreign":{"command":"echo"}}}"#,
    )
    .unwrap();

    cmd(home.path(), root.path())
        .args(["doctor", "--materialize"])
        .assert()
        .failure();

    assert!(cursor_mcp.is_file());
    assert!(fs::read_to_string(cursor_mcp).unwrap().contains("foreign"));
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
        .failure()
        .code(2);

    assert!(!cursor_mcp.exists());
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
        .failure()
        .code(2);

    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&cursor_mcp).unwrap()).unwrap();
    let mcp = v["mcpServers"].as_object().unwrap();
    assert!(!mcp.contains_key("minio"));
    assert!(mcp.contains_key("user-thing"));
}

#[test]
fn deploy_hermes_refuses_legacy_write_preserving_other_keys() {
    let (home, root) = setup();
    let hermes_dir = home.path().join(".hermes");
    fs::create_dir_all(&hermes_dir).unwrap();
    let config_yaml = hermes_dir.join("config.yaml");
    fs::write(&config_yaml, "model: gpt-test\n").unwrap();

    cmd(home.path(), root.path())
        .args(["mcp", "deploy", "minio", "hermes"])
        .assert()
        .failure()
        .code(2);

    let content = fs::read_to_string(&config_yaml).expect("config.yaml updated");
    assert_eq!(content, "model: gpt-test\n");
}

#[test]
fn retract_hermes_preserves_server_without_ownership_evidence() {
    let (home, root) = setup();
    let hermes_dir = home.path().join(".hermes");
    fs::create_dir_all(&hermes_dir).unwrap();
    let config_yaml = hermes_dir.join("config.yaml");
    let before = "model: gpt-test\nmcp_servers:\n  minio:\n    command: docker\n";
    fs::write(&config_yaml, before).unwrap();

    cmd(home.path(), root.path())
        .args(["mcp", "retract", "minio", "hermes"])
        .assert()
        .failure()
        .code(2);

    let content = fs::read_to_string(&config_yaml).expect("config.yaml still exists");
    assert_eq!(content, before);
}

#[test]
fn migrate_hermes_refuses_non_dry_run_and_preserves_legacy_files() {
    let (home, root) = setup();
    let hermes_dir = home.path().join(".hermes");
    fs::create_dir_all(&hermes_dir).unwrap();
    let legacy = hermes_dir.join("mcp.json");
    fs::write(
        &legacy,
        r#"{"mcpServers":{"legacy-svc":{"command":"echo","args":["legacy"]}}}"#,
    )
    .unwrap();
    let config_yaml = hermes_dir.join("config.yaml");
    fs::write(&config_yaml, "model: foreign-model\n").unwrap();
    let legacy_before = fs::read(&legacy).unwrap();
    let config_before = fs::read(&config_yaml).unwrap();

    let assert = cmd(home.path(), root.path())
        .args(["mcp", "migrate-hermes"])
        .assert()
        .failure()
        .code(2);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("source-first") && stderr.contains("只读"),
        "expected a source-first read-only refusal, got: {stderr}"
    );

    assert!(
        legacy.is_file(),
        "refusal must not rename or remove legacy mcp.json"
    );
    assert_eq!(fs::read(&legacy).unwrap(), legacy_before);
    assert_eq!(fs::read(&config_yaml).unwrap(), config_before);
}

#[test]
fn migrate_hermes_dry_run_preserves_legacy_files() {
    let (home, root) = setup();
    let hermes_dir = home.path().join(".hermes");
    fs::create_dir_all(&hermes_dir).unwrap();
    let legacy = hermes_dir.join("mcp.json");
    fs::write(
        &legacy,
        r#"{"mcpServers":{"legacy-svc":{"command":"echo"}}}"#,
    )
    .unwrap();
    let legacy_before = fs::read(&legacy).unwrap();
    let config_yaml = hermes_dir.join("config.yaml");

    cmd(home.path(), root.path())
        .args(["mcp", "migrate-hermes", "--dry-run"])
        .assert()
        .success()
        .stdout(predicates::str::contains("dry-run"));

    assert_eq!(fs::read(&legacy).unwrap(), legacy_before);
    assert!(
        !config_yaml.exists(),
        "dry-run must not create a Hermes configuration"
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
    let secrets = cfg.join("secrets.env");
    fs::write(&secrets, "MINIO_ENDPOINT=secret-value\n").unwrap();
    #[cfg(unix)]
    fs::set_permissions(&secrets, fs::Permissions::from_mode(0o600)).unwrap();

    cmd(home.path(), root.path())
        .args(["--json", "secrets", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("\"keys\""))
        .stdout(predicates::str::contains("\"MINIO_ENDPOINT\""))
        .stdout(predicates::str::contains("secret-value").not());
}
