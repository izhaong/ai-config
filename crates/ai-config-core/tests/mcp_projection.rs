use std::fs;

use ai_config_core::model::PlatformId;
use ai_config_core::projection::mcp::codex_toml::{render_codex_mcp_toml, TomlServerIntent};
use ai_config_core::projection::mcp::cursor_json::{render_cursor_mcp_json, JsonServerIntent};
use ai_config_core::projection::mcp::source::{
    load_mcp_definitions, resolve_effective_mcp_definitions,
};
use ai_config_core::projection::model::SourceLayer;
use ai_config_core::projection::source::OverlayRoots;
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

#[test]
fn resolve_effective_mcp_definitions_overrides_the_whole_server_entry_by_layer() {
    let temp = TempDir::new().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let global = root.join("global");
    let workspace = root.join("workspace");
    let project = root.join("project");
    write_server(
        &global,
        "catalog",
        r#"{"targets":["cursor"],"config":{"command":"global-catalog"}}"#,
    );
    write_server(
        &workspace,
        "search",
        r#"{"targets":["codex"],"config":{"command":"workspace-search"}}"#,
    );
    write_server(
        &project,
        "catalog",
        r#"{"targets":["claude"],"config":{"command":"project-catalog"}}"#,
    );

    let resolved = resolve_effective_mcp_definitions(&OverlayRoots {
        global: global.clone(),
        workspace: Some(workspace.clone()),
        project: project.clone(),
    })
    .unwrap();

    assert_eq!(resolved.len(), 2);
    assert_eq!(resolved[0].definition.server.name, "catalog");
    assert_eq!(resolved[0].source.layer, SourceLayer::Project);
    assert_eq!(
        resolved[0].source.absolute_path,
        project.join("mcp/servers/catalog.json")
    );
    assert_eq!(resolved[0].definition.targets, vec![PlatformId::Claude]);
    assert_eq!(resolved[1].definition.server.name, "search");
    assert_eq!(resolved[1].source.layer, SourceLayer::Workspace);
}

#[test]
fn cursor_renderer_merges_two_owned_servers_once_and_preserves_foreign_and_top_level_fields() {
    let existing = r#"{
      "mcpServers": {"foreign": {"command": "foreign-mcp"}},
      "unknownTopLevel": {"keep": true}
    }"#;
    let intents = vec![
        JsonServerIntent::new("alpha", serde_json::json!({"command": "alpha-mcp"})),
        JsonServerIntent::new("beta", serde_json::json!({"url": "https://beta.test/mcp"})),
    ];

    let rendered = render_cursor_mcp_json(existing, &intents, &[]).unwrap();
    let rendered: serde_json::Value = serde_json::from_str(&rendered).unwrap();

    assert_eq!(rendered["unknownTopLevel"]["keep"], true);
    assert_eq!(rendered["mcpServers"]["foreign"]["command"], "foreign-mcp");
    assert_eq!(rendered["mcpServers"]["alpha"]["command"], "alpha-mcp");
    assert_eq!(
        rendered["mcpServers"]["beta"]["url"],
        "https://beta.test/mcp"
    );
}

#[test]
fn cursor_renderer_removes_only_the_explicitly_owned_unchanged_server() {
    let existing = r#"{
      "mcpServers": {
        "owned": {"command": "owned-mcp"},
        "foreign": {"command": "foreign-mcp"}
      }
    }"#;

    let rendered = render_cursor_mcp_json(existing, &[], &["owned".to_owned()]).unwrap();
    let rendered: serde_json::Value = serde_json::from_str(&rendered).unwrap();

    assert!(rendered["mcpServers"].get("owned").is_none());
    assert_eq!(rendered["mcpServers"]["foreign"]["command"], "foreign-mcp");
}

#[test]
fn codex_renderer_preserves_comments_unknown_tables_and_foreign_servers() {
    let existing = r#"# user comment
model = "gpt-5"

[features]
keep = true

[mcp_servers.foreign] # foreign comment
command = "foreign-mcp"
"#;
    let intents = vec![
        TomlServerIntent::new(
            "alpha",
            serde_json::json!({
                "command": "alpha-mcp",
                "args": ["--serve", "alpha"],
                "env": {"ALPHA_TOKEN": "${ALPHA_TOKEN}"}
            }),
        ),
        TomlServerIntent::new(
            "beta",
            serde_json::json!({
                "url": "https://beta.test/mcp",
                "headers": {"X-Trace": "trace"}
            }),
        ),
    ];

    let rendered = render_codex_mcp_toml(existing, &intents, &[]).unwrap();
    let parsed: toml::Value = toml::from_str(&rendered).unwrap();

    assert!(rendered.contains("# user comment"));
    assert!(rendered.contains("# foreign comment"));
    assert_eq!(parsed["model"].as_str(), Some("gpt-5"));
    assert_eq!(parsed["features"]["keep"].as_bool(), Some(true));
    assert_eq!(
        parsed["mcp_servers"]["foreign"]["command"].as_str(),
        Some("foreign-mcp")
    );
    assert_eq!(
        parsed["mcp_servers"]["alpha"]["command"].as_str(),
        Some("alpha-mcp")
    );
    assert_eq!(
        parsed["mcp_servers"]["alpha"]["env"]["ALPHA_TOKEN"].as_str(),
        Some("${ALPHA_TOKEN}")
    );
    assert_eq!(
        parsed["mcp_servers"]["beta"]["headers"]["X-Trace"].as_str(),
        Some("trace")
    );
}

#[test]
fn codex_renderer_removes_only_explicit_owned_server_and_rejects_non_table_container() {
    let existing = r#"mcp_servers = "not-a-table""#;
    assert!(render_codex_mcp_toml(existing, &[], &[]).is_err());

    let existing = r#"
[mcp_servers.owned]
command = "owned-mcp"

[mcp_servers.foreign]
command = "foreign-mcp"
"#;
    let rendered = render_codex_mcp_toml(existing, &[], &["owned".to_owned()]).unwrap();
    let parsed: toml::Value = toml::from_str(&rendered).unwrap();

    assert!(parsed["mcp_servers"].get("owned").is_none());
    assert_eq!(
        parsed["mcp_servers"]["foreign"]["command"].as_str(),
        Some("foreign-mcp")
    );
}
