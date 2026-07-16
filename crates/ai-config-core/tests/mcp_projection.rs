use std::fs;

use ai_config_core::model::PlatformId;
use ai_config_core::projection::mcp::source::load_mcp_definitions;
use camino::Utf8Path;
use tempfile::TempDir;

fn write_server(root: &Utf8Path, name: &str, body: &str) {
    let path = root.join("mcp/servers").join(format!("{name}.json"));
    fs::create_dir_all(path.parent().unwrap().as_std_path()).unwrap();
    fs::write(path.as_std_path(), body).unwrap();
}

#[test]
fn load_mcp_definitions_reads_enabled_per_server_sources_with_secret_references() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{
          "transport": "stdio",
          "enabled": true,
          "config": {
            "command": "catalog-mcp",
            "env": {"CATALOG_TOKEN": "${CATALOG_TOKEN}"}
          }
        }"#,
    );

    let definitions = load_mcp_definitions(root).unwrap();

    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].server.name, "catalog");
    assert!(definitions[0].server.enabled);
    assert_eq!(definitions[0].server.secret_keys, vec!["CATALOG_TOKEN"]);
}

#[test]
fn load_mcp_definitions_rejects_literal_env_values_without_echoing_them() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{
          "transport": "stdio",
          "config": {
            "command": "catalog-mcp",
            "env": {"CATALOG_TOKEN": "do-not-log-this-token"}
          }
        }"#,
    );

    let error = load_mcp_definitions(root).unwrap_err().to_string();

    assert!(error.contains("CATALOG_TOKEN"));
    assert!(!error.contains("do-not-log-this-token"));
}

#[test]
fn load_mcp_definitions_accepts_a_valid_secret_reference_with_digits() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{
          "transport": "stdio",
          "config": {
            "command": "catalog-mcp",
            "env": {"CATALOG_TOKEN": "${CATALOG_TOKEN_2}"}
          }
        }"#,
    );

    let definitions = load_mcp_definitions(root).unwrap();

    assert_eq!(definitions[0].server.secret_keys, vec!["CATALOG_TOKEN_2"]);
}

#[test]
fn mcp_definition_filters_by_explicit_platform_targets_and_enabled_state() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{
          "enabled": false,
          "targets": ["cursor", "codex"],
          "config": {"command": "catalog-mcp"}
        }"#,
    );

    let definition = load_mcp_definitions(root).unwrap().pop().unwrap();

    assert!(!definition.enabled_for(PlatformId::Cursor));
    assert!(!definition.enabled_for(PlatformId::Codex));
    assert!(!definition.enabled_for(PlatformId::Claude));
    assert_eq!(
        definition.targets,
        vec![PlatformId::Cursor, PlatformId::Codex]
    );
}

#[test]
fn load_mcp_definitions_rejects_literal_headers_and_url_userinfo_without_echoing_them() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{
          "transport": "http",
          "config": {
            "url": "https://alice:do-not-log-url-secret@example.test/mcp",
            "headers": {"Authorization": "do-not-log-header-secret"}
          }
        }"#,
    );

    let error = load_mcp_definitions(root).unwrap_err().to_string();

    assert!(error.contains("Authorization"));
    assert!(!error.contains("do-not-log-header-secret"));
    assert!(!error.contains("do-not-log-url-secret"));
}

#[test]
fn load_mcp_definitions_rejects_url_userinfo_without_echoing_it() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{
          "transport": "http",
          "config": {"url": "https://alice:do-not-log-url-secret@example.test/mcp"}
        }"#,
    );

    let error = load_mcp_definitions(root).unwrap_err().to_string();

    assert!(error.contains("userinfo"));
    assert!(!error.contains("do-not-log-url-secret"));
}

#[test]
fn load_mcp_definitions_rejects_suspected_credential_arguments_without_echoing_them() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    write_server(
        root,
        "catalog",
        r#"{
          "transport": "stdio",
          "config": {
            "command": "catalog-mcp",
            "args": ["--token", "do-not-log-argument-secret"]
          }
        }"#,
    );

    let error = load_mcp_definitions(root).unwrap_err().to_string();

    assert!(error.contains("credential"));
    assert!(!error.contains("do-not-log-argument-secret"));
}
