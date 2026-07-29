//! Lossless-at-the-object-level Cursor/Claude MCP JSON container merge.
//!
//! This module is intentionally pure: ownership checks, secrets injection, backup/swap and
//! ledger updates remain executor responsibilities. The caller supplies only entries already
//! proven eligible for upsert/removal.

use std::collections::HashSet;

use serde_json::{Map, Value};

use crate::error::CoreError;

#[derive(Debug, Clone, PartialEq)]
pub struct JsonServerIntent {
    pub name: String,
    pub config: Value,
}

impl JsonServerIntent {
    pub fn new(name: impl Into<String>, config: Value) -> Self {
        Self {
            name: name.into(),
            config,
        }
    }
}

/// Merge named server entries while retaining foreign servers and all unknown top-level fields.
pub fn render_cursor_mcp_json(
    existing: &str,
    upserts: &[JsonServerIntent],
    owned_removals: &[String],
) -> Result<String, CoreError> {
    let mut root: Value = serde_json::from_str(existing)?;
    let root_object = root
        .as_object_mut()
        .ok_or_else(|| CoreError::TemplateRender {
            template: "cursor mcp container".to_owned(),
            reason: "top-level MCP JSON must be an object".to_owned(),
            hint: "修复平台 MCP 配置后重新生成计划".to_owned(),
        })?;
    let servers = root_object
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| CoreError::TemplateRender {
            template: "cursor mcp container".to_owned(),
            reason: "mcpServers must be an object".to_owned(),
            hint: "修复平台 MCP 配置后重新生成计划".to_owned(),
        })?;
    let mut seen = HashSet::new();
    for intent in upserts {
        if intent.name.is_empty() || !seen.insert(&intent.name) {
            return Err(CoreError::InvalidPath(
                "MCP JSON batch contains an empty or duplicate server name".to_owned(),
            ));
        }
        servers.insert(intent.name.clone(), intent.config.clone());
    }
    for name in owned_removals {
        servers.remove(name);
    }
    Ok(serde_json::to_string_pretty(&root)?)
}
