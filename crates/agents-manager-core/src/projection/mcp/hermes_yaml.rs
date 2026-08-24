//! Lossless-at-the-container-level Hermes MCP YAML merge.
//!
//! `serde_yaml` validates the document but cannot retain comments. This renderer therefore
//! validates the YAML tree first, then replaces only direct children of the block-style
//! `mcp_servers:` mapping. Foreign children and every byte outside that mapping stay intact.
//! It is deliberately pure: ownership validation, filesystem writes, backups and secrets stay
//! in the projection executor.

use std::collections::HashSet;

use serde_json::Value as JsonValue;
use serde_yaml::Value as YamlValue;

use crate::error::CoreError;

#[derive(Debug, Clone, PartialEq)]
pub struct YamlServerIntent {
    pub name: String,
    pub config: JsonValue,
}

impl YamlServerIntent {
    pub fn new(name: impl Into<String>, config: JsonValue) -> Self {
        Self {
            name: name.into(),
            config,
        }
    }
}

/// Merge named MCP entries into a Hermes `config.yaml` without reserializing unrelated YAML.
///
/// `owned_removals` is intentionally explicit. A caller must establish ownership before it may
/// name an entry here; this function never infers ownership from YAML shape or content.
pub fn render_hermes_mcp_yaml(
    existing: &str,
    upserts: &[YamlServerIntent],
    owned_removals: &[String],
) -> Result<String, CoreError> {
    validate_container(existing)?;
    let upsert_names = validate_intents(upserts, owned_removals)?;
    let generated = render_generated_servers(upserts, 2)?;

    if existing.trim().is_empty() {
        if upserts.is_empty() {
            return Ok(existing.to_owned());
        }
        return Ok(format!("mcp_servers:\n{generated}"));
    }

    let lines: Vec<&str> = existing.split_inclusive('\n').collect();
    let Some(header_index) = find_root_mcp_servers_header(&lines) else {
        if upserts.is_empty() {
            return Ok(existing.to_owned());
        }
        let mut rendered = existing.to_owned();
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }
        rendered.push_str("mcp_servers:\n");
        rendered.push_str(&generated);
        return Ok(rendered);
    };

    ensure_block_style_header(lines[header_index])?;
    let section_end = find_section_end(&lines, header_index);
    let server_indent = first_child_indent(&lines, header_index + 1, section_end).unwrap_or(2);
    let server_starts = find_server_starts(&lines, header_index + 1, section_end, server_indent);
    let generated = render_generated_servers(upserts, server_indent)?;

    let mut rendered = join_lines(&lines[..=header_index]);
    if server_starts.is_empty() {
        rendered.push_str(&join_lines(&lines[header_index + 1..section_end]));
    } else {
        let first_start =
            owned_comment_start(&lines, server_starts[0], header_index + 1, server_indent);
        rendered.push_str(&join_lines(&lines[header_index + 1..first_start]));

        let trailer_start = trailing_comment_start(
            &lines,
            *server_starts.last().unwrap(),
            section_end,
            server_indent,
        );
        for (position, start) in server_starts.iter().enumerate() {
            let end = if position + 1 == server_starts.len() {
                trailer_start
            } else {
                owned_comment_start(
                    &lines,
                    server_starts[position + 1],
                    *start + 1,
                    server_indent,
                )
            };
            let name = mapping_key_at_indent(lines[*start], server_indent)
                .expect("server start is derived from a mapping key");
            if !upsert_names.contains(&name) && !owned_removals.contains(&name) {
                rendered.push_str(&join_lines(&lines[*start..end]));
            }
        }
        rendered.push_str(&generated);
        rendered.push_str(&join_lines(&lines[trailer_start..section_end]));
    }
    if server_starts.is_empty() {
        rendered.push_str(&generated);
    }
    rendered.push_str(&join_lines(&lines[section_end..]));
    Ok(rendered)
}

fn validate_container(existing: &str) -> Result<(), CoreError> {
    let document: YamlValue =
        serde_yaml::from_str(existing).map_err(|error| renderer_error(error.to_string()))?;
    match document {
        YamlValue::Null => Ok(()),
        YamlValue::Mapping(root) => match root.get(YamlValue::String("mcp_servers".to_owned())) {
            None | Some(YamlValue::Mapping(_)) => Ok(()),
            Some(_) => Err(renderer_error("mcp_servers must be a YAML mapping")),
        },
        _ => Err(renderer_error("top-level Hermes YAML must be a mapping")),
    }
}

fn validate_intents(
    upserts: &[YamlServerIntent],
    owned_removals: &[String],
) -> Result<HashSet<String>, CoreError> {
    let mut names = HashSet::new();
    for intent in upserts {
        if intent.name.is_empty() || !names.insert(intent.name.clone()) {
            return Err(CoreError::InvalidPath(
                "Hermes MCP batch contains an empty or duplicate server name".to_owned(),
            ));
        }
        if !intent.config.is_object() {
            return Err(renderer_error(
                "each MCP server config must be a JSON object",
            ));
        }
    }
    if owned_removals.iter().any(|name| names.contains(name)) {
        return Err(CoreError::InvalidPath(
            "Hermes MCP batch cannot upsert and remove the same server".to_owned(),
        ));
    }
    Ok(names)
}

fn render_generated_servers(
    upserts: &[YamlServerIntent],
    server_indent: usize,
) -> Result<String, CoreError> {
    let mut rendered = String::new();
    for intent in upserts {
        let yaml = serde_yaml::to_value(&intent.config)
            .map_err(|error| renderer_error(error.to_string()))?;
        let YamlValue::Mapping(_) = yaml else {
            return Err(renderer_error(
                "each MCP server config must be a JSON object",
            ));
        };
        let serialized =
            serde_yaml::to_string(&yaml).map_err(|error| renderer_error(error.to_string()))?;
        let body = serialized.strip_prefix("---\n").unwrap_or(&serialized);
        let key = serde_json::to_string(&intent.name).map_err(CoreError::Json)?;
        rendered.push_str(&" ".repeat(server_indent));
        rendered.push_str(&key);
        rendered.push_str(":\n");
        for line in body.split_inclusive('\n') {
            rendered.push_str(&" ".repeat(server_indent + 2));
            rendered.push_str(line);
        }
        if !body.ends_with('\n') {
            rendered.push('\n');
        }
    }
    Ok(rendered)
}

fn find_root_mcp_servers_header(lines: &[&str]) -> Option<usize> {
    lines
        .iter()
        .position(|line| mapping_key_at_indent(line, 0).as_deref() == Some("mcp_servers"))
}

fn ensure_block_style_header(line: &str) -> Result<(), CoreError> {
    let raw = trim_line_ending(line).trim_start();
    let (_, remainder) = raw
        .split_once(':')
        .ok_or_else(|| renderer_error("mcp_servers header is malformed"))?;
    if remainder.trim().is_empty() || remainder.trim_start().starts_with('#') {
        Ok(())
    } else {
        Err(renderer_error(
            "mcp_servers must use a block mapping so foreign entries can be preserved",
        ))
    }
}

fn find_section_end(lines: &[&str], header_index: usize) -> usize {
    (header_index + 1..lines.len())
        .find(|index| {
            let line = trim_line_ending(lines[*index]);
            !is_blank_or_comment(line) && indentation(line) == 0
        })
        .unwrap_or(lines.len())
}

fn first_child_indent(lines: &[&str], start: usize, end: usize) -> Option<usize> {
    (start..end).find_map(|index| {
        let line = trim_line_ending(lines[index]);
        (!is_blank_or_comment(line))
            .then(|| indentation(line))
            .filter(|indent| *indent > 0)
    })
}

fn find_server_starts(lines: &[&str], start: usize, end: usize, indent: usize) -> Vec<usize> {
    (start..end)
        .filter(|index| mapping_key_at_indent(lines[*index], indent).is_some())
        .collect()
}

fn owned_comment_start(lines: &[&str], start: usize, floor: usize, server_indent: usize) -> usize {
    let mut candidate = start;
    while candidate > floor {
        let previous = trim_line_ending(lines[candidate - 1]);
        if previous.trim().is_empty()
            || (previous.trim_start().starts_with('#') && indentation(previous) <= server_indent)
        {
            candidate -= 1;
        } else {
            break;
        }
    }
    candidate
}

fn trailing_comment_start(
    lines: &[&str],
    last_start: usize,
    end: usize,
    server_indent: usize,
) -> usize {
    let mut candidate = end;
    while candidate > last_start + 1 {
        let previous = trim_line_ending(lines[candidate - 1]);
        if previous.trim().is_empty()
            || (previous.trim_start().starts_with('#') && indentation(previous) <= server_indent)
        {
            candidate -= 1;
        } else {
            break;
        }
    }
    candidate
}

fn mapping_key_at_indent(line: &str, expected_indent: usize) -> Option<String> {
    let line = trim_line_ending(line);
    if indentation(line) != expected_indent {
        return None;
    }
    let content = &line[expected_indent..];
    if is_blank_or_comment(content) || content.starts_with('-') {
        return None;
    }
    let YamlValue::Mapping(mapping) = serde_yaml::from_str::<YamlValue>(content).ok()? else {
        return None;
    };
    if mapping.len() != 1 {
        return None;
    }
    mapping
        .into_iter()
        .next()
        .and_then(|(key, _)| key.as_str().map(str::to_owned))
}

fn trim_line_ending(line: &str) -> &str {
    line.strip_suffix('\n')
        .unwrap_or(line)
        .strip_suffix('\r')
        .unwrap_or(line.strip_suffix('\n').unwrap_or(line))
}

fn indentation(line: &str) -> usize {
    line.bytes().take_while(|byte| *byte == b' ').count()
}

fn is_blank_or_comment(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.is_empty() || trimmed.starts_with('#')
}

fn join_lines(lines: &[&str]) -> String {
    lines.concat()
}

fn renderer_error(reason: impl Into<String>) -> CoreError {
    CoreError::TemplateRender {
        template: "hermes mcp container".to_owned(),
        reason: reason.into(),
        hint: "修复 Hermes config.yaml 后重新生成计划".to_owned(),
    }
}
