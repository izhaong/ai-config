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
fn mcp_migrate_extract_secrets_apply_backs_up_legacy_source_and_writes_only_references() {
    let (home, root) = setup();
    let legacy = root.path().join("legacy.json");
    let secret_dir = root.path().join("secret-store");
    let secret = "literal-env-token-must-not-reach-output";
    let header = "Bearer literal-header-token-must-not-reach-output";
    fs::write(
        &legacy,
        format!(
            r#"{{
  "mcpServers": {{
    "migrate-catalog": {{
      "command": "catalog-mcp",
      "env": {{ "CATALOG_TOKEN": "{secret}" }},
      "headers": {{ "Authorization": "{header}" }}
    }}
  }}
}}"#
        ),
    )
    .expect("write legacy source");
    let legacy_before = fs::read(&legacy).expect("read legacy source before migration");

    let assert = cmd(home.path(), root.path())
        .env("AI_CONFIG_SECRETS_DIR", &secret_dir)
        .args([
            "--json",
            "mcp",
            "migrate",
            "legacy.json",
            "--extract-secrets",
            "--apply",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    assert!(
        !stdout.contains(secret) && !stdout.contains(header) && !stderr.contains(secret),
        "migration output must never expose literal credentials"
    );

    assert_eq!(
        fs::read(&legacy).expect("read legacy source after migration"),
        legacy_before,
        "migration must keep the original legacy source"
    );
    assert_eq!(
        fs::read(root.path().join("legacy.json.ai-config-migrate-backup"))
            .expect("read legacy backup"),
        legacy_before,
        "migration must make an immutable backup before introducing canonical source"
    );

    let canonical: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.path().join("mcp/servers/migrate-catalog.json"))
            .expect("read canonical source"),
    )
    .expect("canonical source JSON");
    assert_eq!(
        canonical["config"]["env"]["CATALOG_TOKEN"],
        "${CATALOG_TOKEN}"
    );
    assert_eq!(
        canonical["config"]["headers"]["Authorization"],
        "${MCP_MIGRATE_CATALOG_HEADER_AUTHORIZATION}"
    );

    let secret_path = secret_dir.join("secrets.env");
    let pairs = ai_config_core::secrets::load_from(
        camino::Utf8Path::from_path(&secret_path).expect("utf8 secret path"),
    )
    .expect("read generated secret store");
    assert_eq!(
        pairs,
        vec![
            ("CATALOG_TOKEN".to_owned(), secret.to_owned()),
            (
                "MCP_MIGRATE_CATALOG_HEADER_AUTHORIZATION".to_owned(),
                header.to_owned(),
            ),
        ]
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&secret_path)
                .expect("stat secret store")
                .permissions()
                .mode()
                & 0o777,
            0o600,
            "secret store must be strict 0600"
        );
    }
}

#[test]
fn mcp_migrate_extract_secrets_apply_aborts_on_existing_different_secret_without_mutation() {
    let (home, root) = setup();
    let legacy = root.path().join("legacy.json");
    let secret_dir = root.path().join("secret-store");
    let secret_path = secret_dir.join("secrets.env");
    fs::write(
        &legacy,
        r#"{"mcpServers":{"collision-server":{"command":"catalog-mcp","env":{"CATALOG_TOKEN":"new-secret-value"}}}}"#,
    )
    .expect("write legacy source");
    ai_config_core::secrets::save_to(
        &[("CATALOG_TOKEN".to_owned(), "old-secret-value".to_owned())],
        camino::Utf8Path::from_path(&secret_path).expect("utf8 secret path"),
    )
    .expect("seed strict secret store");
    let secret_before = fs::read(&secret_path).expect("read secret store before migration");

    let assert = cmd(home.path(), root.path())
        .env("AI_CONFIG_SECRETS_DIR", &secret_dir)
        .args([
            "mcp",
            "migrate",
            "legacy.json",
            "--extract-secrets",
            "--apply",
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    assert!(
        !stderr.contains("new-secret-value")
            && !stderr.contains("old-secret-value")
            && !String::from_utf8(assert.get_output().stdout.clone())
                .expect("utf8 stdout")
                .contains("new-secret-value"),
        "collision errors must not echo secret values"
    );
    assert_eq!(
        fs::read(&secret_path).expect("read secret store after migration"),
        secret_before
    );
    assert!(
        !root
            .path()
            .join("mcp/servers/collision-server.json")
            .exists(),
        "secret collision must prevent any canonical source write"
    );
    assert!(
        !root
            .path()
            .join("legacy.json.ai-config-migrate-backup")
            .exists(),
        "validation failure must not create a misleading backup"
    );
}

#[test]
fn mcp_migrate_extract_secrets_apply_rejects_url_userinfo_before_any_write() {
    let (home, root) = setup();
    let legacy = root.path().join("legacy.json");
    let secret_dir = root.path().join("secret-store");
    let url_password = "url-password-must-not-leak";
    fs::write(
        &legacy,
        format!(
            r#"{{"mcpServers":{{"remote":{{"type":"http","url":"https://user:{url_password}@example.test/mcp"}}}}}}"#
        ),
    )
    .expect("write userinfo source");

    let assert = cmd(home.path(), root.path())
        .env("AI_CONFIG_SECRETS_DIR", &secret_dir)
        .args([
            "mcp",
            "migrate",
            "legacy.json",
            "--extract-secrets",
            "--apply",
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(
        !stderr.contains(url_password) && !stdout.contains(url_password),
        "URL credentials must not appear in errors"
    );
    assert!(
        !root.path().join("mcp/servers/remote.json").exists(),
        "URL userinfo must prevent canonical source write"
    );
    assert!(
        !secret_dir.join("secrets.env").exists(),
        "URL userinfo must prevent secret store write"
    );
}

#[cfg(unix)]
#[test]
fn mcp_migrate_extract_secrets_apply_refuses_a_symlinked_secret_store() {
    use std::os::unix::fs::symlink;

    let (home, root) = setup();
    let legacy = root.path().join("legacy.json");
    let secret_dir = root.path().join("secret-store");
    let outside = root.path().join("outside-secrets.env");
    fs::write(
        &legacy,
        r#"{"mcpServers":{"new-catalog":{"command":"catalog-mcp","env":{"TOKEN":"literal-must-stay-outside"}}}}"#,
    )
    .expect("write legacy source");
    fs::write(&outside, "OUTSIDE=must-not-be-read\n").expect("write outside secrets");
    fs::create_dir_all(&secret_dir).expect("create secret directory");
    symlink(&outside, secret_dir.join("secrets.env")).expect("link secret store");

    let assert = cmd(home.path(), root.path())
        .env("AI_CONFIG_SECRETS_DIR", &secret_dir)
        .args([
            "mcp",
            "migrate",
            "legacy.json",
            "--extract-secrets",
            "--apply",
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    assert!(stderr.contains("secret store must not be a symlink"));
    assert!(!stderr.contains("must-not-be-read"));
    assert_eq!(
        fs::read_to_string(&outside).expect("read outside secret store"),
        "OUTSIDE=must-not-be-read\n"
    );
    assert!(
        !root.path().join("mcp/servers/new-catalog.json").exists(),
        "a symlinked secret store must prevent canonical source creation"
    );
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
