//! Read-only canonical MCP source loader.
//!
//! Platform containers are never sources here. Each `mcp/servers/<name>.json` is parsed as one
//! server and security-sensitive fields are accepted only as exact `${VAR}` references.

use std::collections::BTreeMap;
use std::fs;

use camino::{Utf8Path, Utf8PathBuf};
use serde_json::Value;

use crate::error::CoreError;
use crate::model::{McpServer, PlatformId};
use crate::projection::fingerprint::path_content_digest;
use crate::projection::model::{SourceLayer, SourceRef};
use crate::projection::source::OverlayRoots;
use crate::template::parse_server;

/// One canonical MCP server together with its explicit platform routing policy.
#[derive(Debug, Clone)]
pub struct McpDefinition {
    pub server: McpServer,
    /// Empty is never overloaded as "all": absent `targets` becomes the four deploy targets.
    pub targets: Vec<PlatformId>,
    /// Exact canonical per-server file that supplied this definition.
    pub source_path: Utf8PathBuf,
}

/// An effective MCP definition retains the canonical layer and exact path that won overlay.
#[derive(Debug, Clone)]
pub struct EffectiveMcpDefinition {
    pub definition: McpDefinition,
    pub source: SourceRef,
}

impl McpDefinition {
    pub fn enabled_for(&self, platform: PlatformId) -> bool {
        self.server.enabled && self.targets.contains(&platform)
    }
}

/// Load all canonical per-server definitions in a stable filename order.
pub fn load_mcp_definitions(asset_root: &Utf8Path) -> Result<Vec<McpDefinition>, CoreError> {
    definition_paths(asset_root)?
        .into_iter()
        .map(|path| parse_definition(&path))
        .collect()
}

/// Re-parse one plan-bound canonical server after the executor has verified its content digest.
/// This deliberately takes an explicit path rather than deriving a location from HOME or a
/// platform container, keeping source authority outside the executor.
pub fn load_mcp_definition_at(path: &Utf8Path) -> Result<McpDefinition, CoreError> {
    parse_definition(path)
}

/// Resolve complete server entries by `project > workspace > global`; no field-level merging is
/// allowed because a lower-layer credential or target policy must not leak into an override.
pub fn resolve_effective_mcp_definitions(
    roots: &OverlayRoots,
) -> Result<Vec<EffectiveMcpDefinition>, CoreError> {
    let mut resolved = BTreeMap::new();
    for (root, layer) in [
        (Some(&roots.global), SourceLayer::Global),
        (roots.workspace.as_ref(), SourceLayer::Workspace),
        (Some(&roots.project), SourceLayer::Project),
    ] {
        let Some(root) = root else {
            continue;
        };
        for path in definition_paths(root)? {
            let definition = parse_definition(&path)?;
            let source = SourceRef {
                layer,
                absolute_path: path.clone(),
                fingerprint: path_content_digest(&path)?,
            };
            resolved.insert(
                definition.server.name.clone(),
                EffectiveMcpDefinition { definition, source },
            );
        }
    }
    Ok(resolved.into_values().collect())
}

fn definition_paths(asset_root: &Utf8Path) -> Result<Vec<camino::Utf8PathBuf>, CoreError> {
    let servers_root = asset_root.join("mcp/servers");
    if !servers_root.is_dir() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(servers_root.as_std_path())?;
    let mut paths = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = camino::Utf8PathBuf::from_path_buf(entry.path()).ok()?;
            (path.is_file() && path.extension() == Some("json")).then_some(path)
        })
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn parse_definition(path: &Utf8Path) -> Result<McpDefinition, CoreError> {
    let mut server = parse_server(path)?;
    let raw: Value = serde_json::from_str(&fs::read_to_string(path.as_std_path())?)?;
    // `template::parse_server` historically collected placeholders from every config field.
    // Source-first MCP only permits secret references in env/header values, so overwrite that
    // legacy metadata with the validated, deployable key set here.
    server.secret_keys = validate_secret_references(&server.config, path)?;
    Ok(McpDefinition {
        server,
        targets: parse_targets(&raw, path)?,
        source_path: path.to_path_buf(),
    })
}

fn parse_targets(raw: &Value, path: &Utf8Path) -> Result<Vec<PlatformId>, CoreError> {
    let Some(targets) = raw.get("targets") else {
        return Ok(vec![
            PlatformId::Cursor,
            PlatformId::Codex,
            PlatformId::Claude,
            PlatformId::Hermes,
        ]);
    };
    let values = targets.as_array().ok_or_else(|| {
        source_error(
            path,
            "targets must be an array of supported platform names",
            "使用 cursor、codex、claude 或 hermes 的数组",
        )
    })?;
    let mut parsed = Vec::new();
    for value in values {
        let platform = match value.as_str() {
            Some("cursor") => PlatformId::Cursor,
            Some("codex") => PlatformId::Codex,
            Some("claude") => PlatformId::Claude,
            Some("hermes") => PlatformId::Hermes,
            _ => {
                return Err(source_error(
                    path,
                    "targets contains an unsupported platform",
                    "使用 cursor、codex、claude 或 hermes",
                ));
            }
        };
        if !parsed.contains(&platform) {
            parsed.push(platform);
        }
    }
    Ok(parsed)
}

fn validate_secret_references(config: &Value, path: &Utf8Path) -> Result<Vec<String>, CoreError> {
    let mut secret_keys = Vec::new();
    if let Some(env) = config.get("env").and_then(Value::as_object) {
        for (key, value) in env {
            secret_keys.push(require_placeholder(value, key, "env", path)?);
        }
    }
    if let Some(headers) = config.get("headers").and_then(Value::as_object) {
        for (key, value) in headers {
            secret_keys.push(require_placeholder(value, key, "header", path)?);
        }
    }
    if let Some(url) = config.get("url").and_then(Value::as_str) {
        if url_userinfo_present(url) {
            return Err(source_error(
                path,
                "URL contains userinfo credentials",
                "移除 URL userinfo，并改用 headers/env 的 ${VAR} 引用",
            ));
        }
    }
    if let Some(args) = config.get("args").and_then(Value::as_array) {
        reject_suspected_credential_arguments(args, path)?;
    }
    reject_placeholders_outside_secret_fields(config, path)?;
    secret_keys.sort();
    secret_keys.dedup();
    Ok(secret_keys)
}

/// Values substituted into command/args/url are indistinguishable from credentials in process
/// arguments or URLs. Keep secret interpolation at the explicit env/header boundary only.
fn reject_placeholders_outside_secret_fields(
    config: &Value,
    path: &Utf8Path,
) -> Result<(), CoreError> {
    let Some(fields) = config.as_object() else {
        return Ok(());
    };
    for (key, value) in fields {
        if key == "env" || key == "headers" {
            continue;
        }
        if contains_placeholder(value) {
            return Err(source_error(
                path,
                &format!("{key} contains a secret placeholder outside env/header"),
                "仅在 env 或 headers value 中使用 ${VAR}；command/args/url 必须是普通字面量",
            ));
        }
    }
    Ok(())
}

fn contains_placeholder(value: &Value) -> bool {
    match value {
        Value::String(value) => value.contains("${"),
        Value::Array(values) => values.iter().any(contains_placeholder),
        Value::Object(values) => values.values().any(contains_placeholder),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

fn reject_suspected_credential_arguments(args: &[Value], path: &Utf8Path) -> Result<(), CoreError> {
    for (index, argument) in args.iter().enumerate() {
        let Some(argument) = argument.as_str() else {
            continue;
        };
        let normalized = argument.to_ascii_lowercase();
        let credential_flag = [
            "token",
            "password",
            "secret",
            "api-key",
            "apikey",
            "credential",
        ]
        .iter()
        .any(|needle| normalized.contains(needle));
        if !credential_flag {
            continue;
        }
        let has_inline_value = normalized
            .split_once('=')
            .is_some_and(|(_, value)| !value.is_empty());
        let has_following_value = args
            .get(index + 1)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.starts_with('-'));
        if has_inline_value || has_following_value {
            return Err(source_error(
                path,
                "credential-like command argument contains a literal value",
                "将凭据改为 env/header 中的 ${VAR} 引用",
            ));
        }
    }
    Ok(())
}

fn require_placeholder(
    value: &Value,
    key: &str,
    field: &str,
    path: &Utf8Path,
) -> Result<String, CoreError> {
    let Some(value) = value.as_str() else {
        return Err(source_error(
            path,
            &format!("{field} {key} must be a ${{VAR}} reference"),
            "将敏感字段改为合法的 ${VAR} 引用",
        ));
    };
    let Some(name) = placeholder_name(value) else {
        return Err(source_error(
            path,
            &format!("{field} {key} contains a literal or invalid secret reference"),
            "将敏感字段改为合法的 ${VAR} 引用",
        ));
    };
    Ok(name.to_owned())
}

fn placeholder_name(value: &str) -> Option<&str> {
    let name = value.strip_prefix("${")?.strip_suffix('}')?;
    (!name.is_empty()
        && name.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_uppercase() || (index != 0 && byte.is_ascii_digit())
        }))
    .then_some(name)
}

fn url_userinfo_present(url: &str) -> bool {
    let Some((_, remainder)) = url.split_once("://") else {
        return false;
    };
    remainder
        .split('/')
        .next()
        .is_some_and(|authority| authority.contains('@'))
}

fn source_error(path: &Utf8Path, reason: &str, hint: &str) -> CoreError {
    CoreError::TemplateRender {
        template: path.to_string(),
        reason: reason.to_owned(),
        hint: hint.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use camino::Utf8Path;
    use serde_json::json;

    use super::validate_secret_references;

    #[test]
    fn validated_secret_keys_come_only_from_exact_env_or_header_references() {
        let config = json!({
            "command": "catalog-mcp",
            "env": {"CATALOG_TOKEN": "${CATALOG_TOKEN}"},
            "headers": {"X-Trace": "${TRACE_KEY}"}
        });

        assert_eq!(
            validate_secret_references(&config, Utf8Path::new("mcp/servers/catalog.json")).unwrap(),
            vec!["CATALOG_TOKEN", "TRACE_KEY"]
        );
    }

    #[test]
    fn secret_placeholders_in_command_args_or_url_are_rejected_without_echoing_values() {
        for config in [
            json!({"command": "catalog-${super-secret-sentinel}", "env": {}}),
            json!({"command": "catalog", "args": ["--token=${super-secret-sentinel}"], "env": {}}),
            json!({"url": "https://example.test/${super-secret-sentinel}", "headers": {}}),
        ] {
            let error =
                validate_secret_references(&config, Utf8Path::new("mcp/servers/catalog.json"))
                    .unwrap_err()
                    .to_string();
            assert!(error.contains("mcp/servers/catalog.json"));
            assert!(!error.contains("super-secret-sentinel"));
        }
    }
}
