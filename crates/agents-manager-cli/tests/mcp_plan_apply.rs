//! CLI coverage for the source-first MCP plan/apply boundary.
//!
//! Every command runs with a temporary HOME.  These tests must never depend on, or write to,
//! the developer's real platform configuration.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const BIN: &str = "agents-manager";

fn setup() -> (TempDir, TempDir) {
    let home = TempDir::new().expect("temporary home");
    let root = TempDir::new().expect("temporary asset root");
    fs::create_dir_all(root.path().join("skills")).expect("mark canonical asset root");
    fs::create_dir_all(root.path().join("mcp/servers")).expect("create MCP source directory");
    fs::write(
        root.path().join("mcp/servers/catalog.json"),
        r#"{
          "targets": ["cursor"],
          "config": {"command": "catalog-mcp"}
        }"#,
    )
    .expect("write canonical catalog source");
    (home, root)
}

fn command(home: &Path, root: &Path) -> Command {
    let mut command = Command::cargo_bin(BIN).expect("agents-manager binary");
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("AGENTS_MANAGER_ROOT")
        .env_remove("AGENTS_MANAGER_SECRETS_DIR")
        .arg("--root")
        .arg(root);
    command
}

#[test]
fn deploy_uses_canonical_server_plan_and_preserves_foreign_cursor_entries() {
    let (home, root) = setup();
    let target = home.path().join(".cursor/mcp.json");
    fs::create_dir_all(target.parent().expect("cursor parent")).expect("create cursor parent");
    fs::write(
        &target,
        r#"{"mcpServers":{"foreign":{"command":"external"}},"userField":true}"#,
    )
    .expect("write foreign target");
    let source = root.path().join("mcp/servers/catalog.json");
    let source_before = fs::read(&source).expect("read source before");

    command(home.path(), root.path())
        .args(["mcp", "deploy", "catalog", "cursor"])
        .assert()
        .success();

    let rendered: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&target).expect("read rendered target"))
            .expect("valid MCP JSON");
    assert_eq!(rendered["userField"], true);
    assert_eq!(rendered["mcpServers"]["foreign"]["command"], "external");
    assert_eq!(rendered["mcpServers"]["catalog"]["command"], "catalog-mcp");
    assert_eq!(fs::read(&source).expect("read source after"), source_before);
    assert!(
        !home.path().join(".codex/config.toml").exists(),
        "single-platform deploy must not touch other platform targets"
    );
}

#[test]
fn deploy_from_a_project_root_writes_only_the_project_platform_target() {
    let home = TempDir::new().expect("temporary home");
    let project = TempDir::new().expect("temporary project");
    let assets = project.path().join(".agents");
    fs::create_dir_all(assets.join("skills")).expect("mark project asset root");
    fs::create_dir_all(assets.join("mcp/servers")).expect("create project MCP source directory");
    fs::write(
        assets.join("mcp/servers/project-catalog.json"),
        r#"{"targets":["cursor"],"config":{"command":"project-catalog"}}"#,
    )
    .expect("write project canonical source");

    command(home.path(), project.path())
        .args(["mcp", "deploy", "project-catalog", "cursor"])
        .assert()
        .success();

    assert!(
        project.path().join(".cursor/mcp.json").is_file(),
        "project root must be the project deploy base"
    );
    assert!(
        !home.path().join(".cursor/mcp.json").exists(),
        "project deploy must not write the user HOME target"
    );
}

#[test]
fn deploy_skips_only_the_server_when_a_declared_secret_is_missing() {
    let (home, root) = setup();
    fs::write(
        root.path().join("mcp/servers/catalog.json"),
        r#"{
          "targets": ["cursor"],
          "config": {
            "command": "catalog-mcp",
            "env": {"CATALOG_TOKEN": "${CATALOG_TOKEN}"}
          }
        }"#,
    )
    .expect("write secret-bearing canonical source");
    let target = home.path().join(".cursor/mcp.json");

    command(home.path(), root.path())
        .args(["--json", "mcp", "deploy", "catalog", "cursor"])
        .assert()
        .success()
        .stdout(predicate::str::contains("CATALOG_TOKEN"))
        .stdout(predicate::str::contains("catalog-mcp").not());

    assert!(
        !target.exists(),
        "a missing secret must skip this server before any platform file is created"
    );
}

#[cfg(unix)]
#[test]
fn deploy_hydrates_a_strict_secret_store_without_echoing_its_value() {
    let (home, root) = setup();
    fs::write(
        root.path().join("mcp/servers/catalog.json"),
        r#"{
          "targets": ["cursor"],
          "config": {
            "command": "catalog-mcp",
            "env": {"CATALOG_TOKEN": "${CATALOG_TOKEN}"}
          }
        }"#,
    )
    .expect("write secret-bearing canonical source");
    let secret_dir = home.path().join(".config/agents-manager");
    fs::create_dir_all(&secret_dir).expect("create secret directory");
    let secret_store = secret_dir.join("secrets.env");
    let secret = "catalog-secret-must-not-be-echoed";
    fs::write(&secret_store, format!("CATALOG_TOKEN={secret}\n")).expect("write secret store");
    fs::set_permissions(&secret_store, fs::Permissions::from_mode(0o600))
        .expect("make secret store strict");

    command(home.path(), root.path())
        .args(["--json", "mcp", "deploy", "catalog", "cursor"])
        .assert()
        .success()
        .stdout(predicate::str::contains(secret).not());

    let rendered: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(home.path().join(".cursor/mcp.json")).expect("read target"),
    )
    .expect("valid target JSON");
    assert_eq!(
        rendered["mcpServers"]["catalog"]["env"]["CATALOG_TOKEN"],
        secret
    );
}

#[test]
fn retract_without_persistent_ownership_proof_is_a_zero_write_conflict() {
    let (home, root) = setup();
    let target = home.path().join(".cursor/mcp.json");
    fs::create_dir_all(target.parent().expect("cursor parent")).expect("create cursor parent");
    let before = r#"{"mcpServers":{"catalog":{"command":"foreign-value"},"foreign":{"command":"external"}}}"#;
    fs::write(&target, before).expect("write unproven target");

    command(home.path(), root.path())
        .args(["mcp", "retract", "catalog", "cursor"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "mcp_projection_plan_conflict:generated_ownership_unproven",
        ));

    assert_eq!(
        fs::read_to_string(&target).expect("read target after"),
        before
    );
}

#[cfg(unix)]
#[test]
fn deploy_rejects_a_non_0600_secret_store_before_any_platform_write() {
    let (home, root) = setup();
    fs::write(
        root.path().join("mcp/servers/catalog.json"),
        r#"{
          "targets": ["cursor"],
          "config": {
            "command": "catalog-mcp",
            "env": {"CATALOG_TOKEN": "${CATALOG_TOKEN}"}
          }
        }"#,
    )
    .expect("write secret-bearing canonical source");
    let secret_dir = home.path().join(".config/agents-manager");
    fs::create_dir_all(&secret_dir).expect("create secret directory");
    let secret_store = secret_dir.join("secrets.env");
    fs::write(&secret_store, "CATALOG_TOKEN=must-not-leak\n").expect("write secret store");
    fs::set_permissions(&secret_store, fs::Permissions::from_mode(0o644))
        .expect("make secret store insecure");

    command(home.path(), root.path())
        .args(["mcp", "deploy", "catalog", "cursor"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("must-not-leak").not());

    assert!(
        !home.path().join(".cursor/mcp.json").exists(),
        "the 0600 check must happen before a platform target is created"
    );
}
