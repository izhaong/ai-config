//! Lossless-at-the-container-level Codex MCP TOML merge.
//!
//! The renderer owns only `[mcp_servers.<name>]` tables. It deliberately does no filesystem
//! work and never decides whether a server is eligible for removal; the planner/executor must
//! establish that ownership before calling it.

use std::collections::HashSet;

use serde_json::Value as JsonValue;
use toml_edit::{Array, DocumentMut, Item, Table, Value};

use crate::error::CoreError;

#[derive(Debug, Clone, PartialEq)]
pub struct TomlServerIntent {
    pub name: String,
    pub config: JsonValue,
}

impl TomlServerIntent {
    pub fn new(name: impl Into<String>, config: JsonValue) -> Self {
        Self {
            name: name.into(),
            config,
        }
    }
}

/// Merge owned MCP server tables while preserving TOML comments, other config tables and foreign
/// server entries. `owned_removals` is intentionally explicit: no inference from table shape is
/// permitted.
pub fn render_codex_mcp_toml(
    existing: &str,
    upserts: &[TomlServerIntent],
    owned_removals: &[String],
) -> Result<String, CoreError> {
    let mut document = existing
        .parse::<DocumentMut>()
        .map_err(|error| renderer_error(error.to_string()))?;
    let root = document.as_table_mut();
    if !root.contains_key("mcp_servers") {
        root.insert("mcp_servers", Item::Table(Table::new()));
    }
    let servers = root
        .get_mut("mcp_servers")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| renderer_error("mcp_servers must be a TOML table"))?;

    let mut seen = HashSet::new();
    for intent in upserts {
        if intent.name.is_empty() || !seen.insert(&intent.name) {
            return Err(CoreError::InvalidPath(
                "MCP TOML batch contains an empty or duplicate server name".to_owned(),
            ));
        }
        let config = intent
            .config
            .as_object()
            .ok_or_else(|| renderer_error("each MCP server config must be a JSON object"))?;
        servers.insert(&intent.name, Item::Table(json_object_to_table(config)?));
    }
    for name in owned_removals {
        servers.remove(name);
    }
    Ok(document.to_string())
}

fn json_object_to_table(object: &serde_json::Map<String, JsonValue>) -> Result<Table, CoreError> {
    let mut table = Table::new();
    for (key, value) in object {
        table.insert(key, json_to_item(value)?);
    }
    Ok(table)
}

fn json_to_item(value: &JsonValue) -> Result<Item, CoreError> {
    match value {
        JsonValue::Null => Err(renderer_error("MCP TOML cannot represent JSON null")),
        JsonValue::Bool(value) => Ok(Item::Value(Value::from(*value))),
        JsonValue::Number(value) => {
            if let Some(value) = value.as_i64() {
                Ok(Item::Value(Value::from(value)))
            } else if let Some(value) = value.as_u64() {
                let value = i64::try_from(value)
                    .map_err(|_| renderer_error("MCP TOML integer exceeds signed TOML range"))?;
                Ok(Item::Value(Value::from(value)))
            } else if let Some(value) = value.as_f64() {
                Ok(Item::Value(Value::from(value)))
            } else {
                Err(renderer_error("MCP TOML cannot represent JSON number"))
            }
        }
        JsonValue::String(value) => Ok(Item::Value(Value::from(value.as_str()))),
        JsonValue::Array(values) => {
            let mut array = Array::new();
            for value in values {
                array.push(json_to_value(value)?);
            }
            Ok(Item::Value(Value::Array(array)))
        }
        JsonValue::Object(object) => Ok(Item::Table(json_object_to_table(object)?)),
    }
}

fn json_to_value(value: &JsonValue) -> Result<Value, CoreError> {
    match json_to_item(value)? {
        Item::Value(value) => Ok(value),
        Item::Table(_) | Item::ArrayOfTables(_) | Item::None => Err(renderer_error(
            "MCP TOML arrays cannot contain JSON objects or null values",
        )),
    }
}

fn renderer_error(reason: impl Into<String>) -> CoreError {
    CoreError::TemplateRender {
        template: "codex mcp container".to_owned(),
        reason: reason.into(),
        hint: "修复 Codex config.toml 后重新生成计划".to_owned(),
    }
}
