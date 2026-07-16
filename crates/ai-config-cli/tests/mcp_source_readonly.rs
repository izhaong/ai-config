//! Integration coverage for the source-first read-only MCP commands.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use tempfile::TempDir;

const BIN: &str = "ai-config";
const LEGACY_SECRET_SENTINEL: &str = "legacy-secret-sentinel-must-not-leak";

fn setup() -> (TempDir, TempDir) {
    let home = TempDir::new().expect("home tempdir");
    let root = TempDir::new().expect("root tempdir");
    let servers = root.path().join("mcp/servers");
    fs::create_dir_all(&servers).expect("create canonical MCP source directory");
    fs::write(
        servers.join("catalog.json"),
        r#"{
  "name": "catalog",
  "transport": "stdio",
  "enabled": true,
  "targets": ["cursor"],
  "config": {
    "command": "catalog-mcp",
    "env": { "CATALOG_TOKEN": "${CATALOG_TOKEN}" }
  }
}"#,
    )
    .expect("write canonical catalog source");
    fs::write(
        root.path().join("mcp.json"),
        format!(
            r#"{{
  "mcpServers": {{
    "catalog": {{
      "command": "legacy-catalog",
      "env": {{ "CATALOG_TOKEN": "{LEGACY_SECRET_SENTINEL}" }}
    }}
  }}
}}"#
        ),
    )
    .expect("write ignored legacy MCP container");

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
fn mcp_list_reads_canonical_per_server_source_instead_of_legacy_container() {
    let (home, root) = setup();
    let expected_source = root.path().join("mcp/servers/catalog.json");

    let assert = cmd(home.path(), root.path())
        .args(["--json", "mcp", "list"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let output: serde_json::Value = serde_json::from_str(&stdout).expect("JSON list output");
    let items = output["items"].as_array().expect("items array");

    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "catalog");
    assert_eq!(items[0]["transport"], "stdio");
    assert_eq!(items[0]["enabled"], true);
    assert_eq!(
        items[0]["source_path"],
        serde_json::Value::String(expected_source.to_string_lossy().into_owned())
    );
    assert_eq!(
        items[0]["secret_keys"],
        serde_json::json!(["CATALOG_TOKEN"])
    );
    assert!(
        !stdout.contains(LEGACY_SECRET_SENTINEL),
        "list must not read or echo the legacy container: {stdout}"
    );
}

#[test]
fn mcp_show_exposes_only_canonical_metadata_and_secret_key_names() {
    let (home, root) = setup();
    let expected_source = root.path().join("mcp/servers/catalog.json");

    let assert = cmd(home.path(), root.path())
        .args(["--json", "mcp", "show", "catalog"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let output: serde_json::Value = serde_json::from_str(&stdout).expect("JSON show output");

    assert_eq!(output["name"], "catalog");
    assert_eq!(output["transport"], "stdio");
    assert_eq!(output["enabled"], true);
    assert_eq!(
        output["source_path"],
        serde_json::Value::String(expected_source.to_string_lossy().into_owned())
    );
    assert_eq!(output["secret_keys"], serde_json::json!(["CATALOG_TOKEN"]));
    assert!(
        output.get("config").is_none(),
        "show must not expose config"
    );
    assert!(
        !stdout.contains(LEGACY_SECRET_SENTINEL),
        "show must not read or echo the legacy container: {stdout}"
    );
}

#[test]
fn mcp_domain_crud_only_mutates_validated_per_server_sources() {
    let (home, root) = setup();
    let input = root.path().join("input.json");
    fs::write(
        &input,
        r#"{
  "name": "weather",
  "transport": "http",
  "enabled": true,
  "targets": ["codex"],
  "config": {
    "url": "https://weather.example/mcp",
    "headers": { "Authorization": "${WEATHER_TOKEN}" }
  }
}"#,
    )
    .expect("write validated source input");

    cmd(home.path(), root.path())
        .args(["mcp", "add", input.to_str().expect("utf8 path")])
        .assert()
        .success();
    let stored = root.path().join("mcp/servers/weather.json");
    assert!(stored.is_file(), "add creates the canonical source only");

    cmd(home.path(), root.path())
        .args(["mcp", "disable", "weather"])
        .assert()
        .success();
    let disabled: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&stored).expect("read disabled source"))
            .expect("disabled source is JSON");
    assert_eq!(disabled["enabled"], false);
    assert_eq!(
        disabled["config"]["headers"]["Authorization"],
        "${WEATHER_TOKEN}"
    );

    cmd(home.path(), root.path())
        .args(["mcp", "enable", "weather"])
        .assert()
        .success();
    cmd(home.path(), root.path())
        .args(["mcp", "remove", "weather"])
        .assert()
        .success();
    assert!(
        !stored.exists(),
        "remove only removes its canonical server file"
    );
    assert!(
        root.path().join("mcp.json").is_file(),
        "legacy container is untouched"
    );
}

#[test]
fn mcp_add_refuses_literal_credentials_without_creating_a_source() {
    let (home, root) = setup();
    let input = root.path().join("unsafe.json");
    fs::write(
        &input,
        r#"{
  "name": "unsafe",
  "config": {
    "command": "unsafe-mcp",
    "env": { "TOKEN": "literal-secret-must-not-be-written" }
  }
}"#,
    )
    .expect("write unsafe source input");

    cmd(home.path(), root.path())
        .args(["mcp", "add", input.to_str().expect("utf8 path")])
        .assert()
        .failure()
        .stderr(predicates::str::contains("literal-secret-must-not-be-written").not());
    assert!(
        !root.path().join("mcp/servers/unsafe.json").exists(),
        "unsafe input must not create a canonical source"
    );
}

#[test]
fn mcp_migrate_extract_secrets_apply_is_explicitly_fail_closed_until_secret_store_is_wired() {
    let (home, root) = setup();
    cmd(home.path(), root.path())
        .args(["mcp", "migrate", "--extract-secrets", "--apply"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("secret extraction"));
}

#[cfg(unix)]
#[test]
fn mcp_enable_refuses_a_symlinked_canonical_source_without_touching_its_target() {
    use std::os::unix::fs::symlink;

    let (home, root) = setup();
    let outside = root.path().join("outside.json");
    let original = r#"{
  "name": "linked",
  "enabled": false,
  "config": { "command": "safe-mcp" }
}"#;
    fs::write(&outside, original).expect("write outside source");
    symlink(&outside, root.path().join("mcp/servers/linked.json"))
        .expect("create malicious canonical symlink");

    cmd(home.path(), root.path())
        .args(["mcp", "enable", "linked"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("must not be a symlink"));
    assert_eq!(
        fs::read_to_string(&outside).expect("read outside source"),
        original,
        "source CRUD must not follow a canonical symlink"
    );
}
