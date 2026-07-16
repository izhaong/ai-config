//! Read-only canonical MCP source loader.
//!
//! Platform containers are never sources here. Each `mcp/servers/<name>.json` is parsed as one
//! server and security-sensitive fields are accepted only as exact `${VAR}` references.

use std::fs;

use camino::Utf8Path;
use serde_json::Value;

use crate::error::CoreError;
use crate::model::{McpServer, PlatformId};
use crate::template::parse_server;

/// One canonical MCP server together with its explicit platform routing policy.
#[derive(Debug, Clone)]
pub struct McpDefinition {
    pub server: McpServer,
    /// Empty is never overloaded as "all": absent `targets` becomes the four deploy targets.
    pub targets: Vec<PlatformId>,
}

impl McpDefinition {
    pub fn enabled_for(&self, platform: PlatformId) -> bool {
        self.server.enabled && self.targets.contains(&platform)
    }
}

/// Load all canonical per-server definitions in a stable filename order.
pub fn load_mcp_definitions(asset_root: &Utf8Path) -> Result<Vec<McpDefinition>, CoreError> {
    let servers_root = asset_root.join("mcp/servers");
    if !servers_root.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = fs::read_dir(servers_root.as_std_path())?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = camino::Utf8PathBuf::from_path_buf(entry.path()).ok()?;
            (path.is_file() && path.extension() == Some("json")).then_some(path)
        })
        .collect::<Vec<_>>();
    paths.sort();

    paths
        .into_iter()
        .map(|path| {
            let server = parse_server(&path)?;
            let raw: Value = serde_json::from_str(&fs::read_to_string(path.as_std_path())?)?;
            validate_secret_references(&server.config, &path)?;
            Ok(McpDefinition {
                server,
                targets: parse_targets(&raw, &path)?,
            })
        })
        .collect()
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

fn validate_secret_references(config: &Value, path: &Utf8Path) -> Result<(), CoreError> {
    if let Some(env) = config.get("env").and_then(Value::as_object) {
        for (key, value) in env {
            require_placeholder(value, key, "env", path)?;
        }
    }
    if let Some(headers) = config.get("headers").and_then(Value::as_object) {
        for (key, value) in headers {
            require_placeholder(value, key, "header", path)?;
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
    Ok(())
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
) -> Result<(), CoreError> {
    let Some(value) = value.as_str() else {
        return Err(source_error(
            path,
            &format!("{field} {key} must be a ${{VAR}} reference"),
            "将敏感字段改为合法的 ${VAR} 引用",
        ));
    };
    if placeholder_name(value).is_none() {
        return Err(source_error(
            path,
            &format!("{field} {key} contains a literal or invalid secret reference"),
            "将敏感字段改为合法的 ${VAR} 引用",
        ));
    }
    Ok(())
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
