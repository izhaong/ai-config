//! Claude user/project MCP JSON container merge.
//!
//! Claude uses the same named `mcpServers` object shape as Cursor, but this explicit façade keeps
//! its user `projects` and settings preservation contract visible at the platform boundary.

use crate::error::CoreError;
use crate::projection::mcp::cursor_json::{render_cursor_mcp_json, JsonServerIntent};

pub use crate::projection::mcp::cursor_json::JsonServerIntent as ClaudeJsonServerIntent;

/// Render only named MCP entries; project-local state and all other top-level Claude fields stay
/// untouched. Ownership proof for removals belongs to the planner/executor.
pub fn render_claude_mcp_json(
    existing: &str,
    upserts: &[JsonServerIntent],
    owned_removals: &[String],
) -> Result<String, CoreError> {
    render_cursor_mcp_json(existing, upserts, owned_removals)
}
