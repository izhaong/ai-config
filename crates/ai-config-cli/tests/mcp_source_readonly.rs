//! Integration coverage for the source-first read-only MCP commands.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
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
