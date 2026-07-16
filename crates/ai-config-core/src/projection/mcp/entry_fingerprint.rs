//! Pure semantic inspection for named MCP entries in platform containers.
//!
//! The resulting digest deliberately covers one normalized server configuration, not the
//! surrounding container. It lets a future ledger distinguish an externally changed managed
//! entry from unrelated edits to the same JSON/TOML/YAML file. This module performs no IO and
//! never exposes parsed configuration payloads through errors.

use sha2::{Digest, Sha256};

use crate::error::CoreError;

/// Stable semantic digest for one named MCP configuration entry.
///
/// `name` remains separate because ownership is keyed by the platform entry name; the digest
/// itself is only the normalized configuration so it can be compared across platform formats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpEntryFingerprint {
    pub name: String,
    pub digest: String,
}

/// Inspect Cursor's `mcpServers` object without reading or writing a target file.
pub fn inspect_cursor_mcp_entries(existing: &str) -> Result<Vec<McpEntryFingerprint>, CoreError> {
    inspect_json_entries(existing, "Cursor", "mcpServers")
}

/// Inspect Claude's `mcpServers` object without reading or writing a target file.
pub fn inspect_claude_mcp_entries(existing: &str) -> Result<Vec<McpEntryFingerprint>, CoreError> {
    inspect_json_entries(existing, "Claude", "mcpServers")
}

/// Inspect Codex's `[mcp_servers.<name>]` tables without reading or writing a target file.
pub fn inspect_codex_mcp_entries(existing: &str) -> Result<Vec<McpEntryFingerprint>, CoreError> {
    let root: toml::Value = existing
        .parse()
        .map_err(|_| inspection_error("Codex MCP TOML syntax is invalid"))?;
    let root = root
        .as_table()
        .ok_or_else(|| inspection_error("Codex MCP TOML must have a table root"))?;
    let Some(servers) = root.get("mcp_servers") else {
        return Ok(Vec::new());
    };
    let servers = servers
        .as_table()
        .ok_or_else(|| inspection_error("mcp_servers must be a TOML table"))?;

    let mut entries = Vec::with_capacity(servers.len());
    for (name, config) in servers {
        let config = toml_to_json(config)?;
        entries.push(fingerprint_entry(name, config)?);
    }
    Ok(sorted_entries(entries))
}

/// Inspect Hermes' `mcp_servers` mapping without reading or writing a target file.
pub fn inspect_hermes_mcp_entries(existing: &str) -> Result<Vec<McpEntryFingerprint>, CoreError> {
    let root: serde_yaml::Value = serde_yaml::from_str(existing)
        .map_err(|_| inspection_error("Hermes MCP YAML syntax is invalid"))?;
    let serde_yaml::Value::Mapping(root) = root else {
        return match root {
            serde_yaml::Value::Null => Ok(Vec::new()),
            _ => Err(inspection_error("Hermes MCP YAML must have a mapping root")),
        };
    };
    let Some(servers) = root.get(serde_yaml::Value::String("mcp_servers".to_owned())) else {
        return Ok(Vec::new());
    };
    let serde_yaml::Value::Mapping(servers) = servers else {
        return Err(inspection_error("mcp_servers must be a YAML mapping"));
    };

    let mut entries = Vec::with_capacity(servers.len());
    for (name, config) in servers {
        let serde_yaml::Value::String(name) = name else {
            return Err(inspection_error("MCP server names must be strings"));
        };
        entries.push(fingerprint_entry(name, yaml_to_json(config)?)?);
    }
    Ok(sorted_entries(entries))
}

fn inspect_json_entries(
    existing: &str,
    platform: &str,
    container_key: &str,
) -> Result<Vec<McpEntryFingerprint>, CoreError> {
    let root: serde_json::Value = serde_json::from_str(existing)
        .map_err(|_| inspection_error(format!("{platform} MCP JSON syntax is invalid")))?;
    let root = root
        .as_object()
        .ok_or_else(|| inspection_error("MCP JSON must have an object root"))?;
    let Some(servers) = root.get(container_key) else {
        return Ok(Vec::new());
    };
    let servers = servers
        .as_object()
        .ok_or_else(|| inspection_error("mcpServers must be a JSON object"))?;

    let mut entries = Vec::with_capacity(servers.len());
    for (name, config) in servers {
        entries.push(fingerprint_entry(name, config.clone())?);
    }
    Ok(sorted_entries(entries))
}

fn fingerprint_entry(
    name: &str,
    config: serde_json::Value,
) -> Result<McpEntryFingerprint, CoreError> {
    if name.is_empty() {
        return Err(inspection_error("MCP server name cannot be empty"));
    }
    if !config.is_object() {
        return Err(inspection_error("each MCP server config must be an object"));
    }
    let encoded = serde_json::to_vec(&config)
        .map_err(|_| inspection_error("MCP server config cannot be normalized"))?;
    let mut hasher = Sha256::new();
    hasher.update(b"ai-config:mcp-entry-fingerprint:v1\0");
    hasher.update(encoded);
    Ok(McpEntryFingerprint {
        name: name.to_owned(),
        digest: hex::encode(hasher.finalize()),
    })
}

fn sorted_entries(mut entries: Vec<McpEntryFingerprint>) -> Vec<McpEntryFingerprint> {
    entries.sort_unstable_by(|left, right| left.name.cmp(&right.name));
    entries
}

fn toml_to_json(value: &toml::Value) -> Result<serde_json::Value, CoreError> {
    match value {
        toml::Value::String(value) => Ok(serde_json::Value::String(value.clone())),
        toml::Value::Integer(value) => Ok(serde_json::Value::Number((*value).into())),
        toml::Value::Float(value) => serde_json::Number::from_f64(*value)
            .map(serde_json::Value::Number)
            .ok_or_else(|| inspection_error("MCP TOML contains a non-finite number")),
        toml::Value::Boolean(value) => Ok(serde_json::Value::Bool(*value)),
        toml::Value::Datetime(_) => Err(inspection_error(
            "MCP TOML datetime values cannot be represented safely",
        )),
        toml::Value::Array(values) => values
            .iter()
            .map(toml_to_json)
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array),
        toml::Value::Table(values) => {
            let mut object = serde_json::Map::new();
            for (key, value) in values {
                object.insert(key.clone(), toml_to_json(value)?);
            }
            Ok(serde_json::Value::Object(object))
        }
    }
}

fn yaml_to_json(value: &serde_yaml::Value) -> Result<serde_json::Value, CoreError> {
    match value {
        serde_yaml::Value::Null => Ok(serde_json::Value::Null),
        serde_yaml::Value::Bool(value) => Ok(serde_json::Value::Bool(*value)),
        serde_yaml::Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                Ok(serde_json::Value::Number(value.into()))
            } else if let Some(value) = value.as_u64() {
                Ok(serde_json::Value::Number(value.into()))
            } else if let Some(value) = value.as_f64() {
                serde_json::Number::from_f64(value)
                    .map(serde_json::Value::Number)
                    .ok_or_else(|| inspection_error("MCP YAML contains a non-finite number"))
            } else {
                Err(inspection_error("MCP YAML contains an unsupported number"))
            }
        }
        serde_yaml::Value::String(value) => Ok(serde_json::Value::String(value.clone())),
        serde_yaml::Value::Sequence(values) => values
            .iter()
            .map(yaml_to_json)
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array),
        serde_yaml::Value::Mapping(values) => {
            let mut object = serde_json::Map::new();
            for (key, value) in values {
                let serde_yaml::Value::String(key) = key else {
                    return Err(inspection_error("MCP YAML object keys must be strings"));
                };
                if object.insert(key.clone(), yaml_to_json(value)?).is_some() {
                    return Err(inspection_error("MCP YAML object keys must be unique"));
                }
            }
            Ok(serde_json::Value::Object(object))
        }
        serde_yaml::Value::Tagged(_) => Err(inspection_error(
            "MCP YAML tagged values cannot be represented safely",
        )),
    }
}

fn inspection_error(reason: impl Into<String>) -> CoreError {
    CoreError::TemplateRender {
        template: "MCP entry inspection".to_owned(),
        reason: reason.into(),
        hint: "修复平台 MCP 配置后重新生成计划".to_owned(),
    }
}
