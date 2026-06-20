//! 单条资产 CRUD + deploy/retract（GUI / 未来 CLI 共用）。

use std::fs;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::asset_scope::{self, locate_source};
use crate::error::CoreError;
use crate::hermes_config;
use crate::materialize;
use crate::mcp_json;
use crate::model::{AssetKind, PlatformId};
use crate::platform;
use crate::sync::{asset_dest_for_at_base, link_src_for_create};

/// 作用域三路径：默认根、资产扫描根、平台下发根。
#[derive(Debug, Clone, Copy)]
pub struct ScopeRoots<'a> {
    pub default_root: &'a Utf8Path,
    pub asset_root: &'a Utf8Path,
    pub deploy_base: &'a Utf8Path,
}

/// 资产文件详情（GUI 抽屉 / API）。
#[derive(Debug, Clone, Serialize)]
pub struct AssetFileDetail {
    pub name: String,
    pub description: String,
    pub content: String,
    pub source_path: String,
    pub parent_path: String,
}

/// 下发到目标平台。
pub fn deploy(
    scope: &ScopeRoots<'_>,
    kind: AssetKind,
    name: &str,
    plat: PlatformId,
) -> Result<String, CoreError> {
    ensure_deploy_target(plat)?;
    ensure_platform_supports(scope, plat, kind)?;
    match kind {
        AssetKind::Mcp => deploy_mcp(scope, name, plat),
        AssetKind::Skill => deploy_skill(scope, name, plat),
        AssetKind::Rule | AssetKind::Command | AssetKind::Agent => {
            deploy_materialized(scope, kind, name, plat)
        }
    }
}

/// 从平台收回。
pub fn retract(
    scope: &ScopeRoots<'_>,
    kind: AssetKind,
    name: &str,
    plat: PlatformId,
) -> Result<String, CoreError> {
    ensure_deploy_target(plat)?;
    ensure_platform_supports(scope, plat, kind)?;
    match kind {
        AssetKind::Mcp => retract_mcp(scope, name, plat),
        _ => {
            let src = locate_source(scope.default_root, scope.asset_root, kind, name)?;
            let dest = asset_dest_for_at_base(plat, kind, name, &src, scope.deploy_base)
                .ok_or_else(|| CoreError::InvalidPath(format!("无法算 {kind:?} `{name}` ← {plat:?} 的 dest")))?;
            materialize::retract(&dest)?;
            Ok(format!("{kind:?} `{name}` ← {plat:?} OK ({dest})"))
        }
    }
}

/// 读取源侧资产详情。
pub fn get_detail(
    scope: &ScopeRoots<'_>,
    kind: AssetKind,
    name: &str,
) -> Result<AssetFileDetail, CoreError> {
    match kind {
        AssetKind::Mcp => get_mcp_detail(scope, name),
        AssetKind::Skill => {
            let skill_md = locate_source(scope.default_root, scope.asset_root, kind, name)?;
            let content = fs::read_to_string(&skill_md).map_err(CoreError::Io)?;
            let (fm_name, description) = parse_skill_meta(&content);
            Ok(AssetFileDetail {
                name: fm_name.unwrap_or_else(|| name.to_string()),
                description,
                content,
                source_path: skill_md.to_string(),
                parent_path: skill_md
                    .parent()
                    .map(|p| p.to_string())
                    .unwrap_or_default(),
            })
        }
        AssetKind::Rule | AssetKind::Command => {
            let src = locate_source(scope.default_root, scope.asset_root, kind, name)?;
            let content = fs::read_to_string(&src).map_err(CoreError::Io)?;
            let (fm_name, description) = parse_skill_meta(&content);
            Ok(AssetFileDetail {
                name: fm_name.unwrap_or_else(|| name.to_string()),
                description,
                content,
                source_path: src.to_string(),
                parent_path: src.parent().map(|p| p.to_string()).unwrap_or_default(),
            })
        }
        AssetKind::Agent => {
            let src = locate_source(scope.default_root, scope.asset_root, kind, name)?;
            let read_path = agent_read_path(&src);
            let content = fs::read_to_string(&read_path).map_err(CoreError::Io)?;
            let description = parse_description(AssetKind::Agent, &src);
            Ok(AssetFileDetail {
                name: name.to_string(),
                description,
                content,
                source_path: read_path.to_string(),
                parent_path: read_path
                    .parent()
                    .map(|p| p.to_string())
                    .unwrap_or_default(),
            })
        }
    }
}

/// 保存源侧正文。
pub fn save_content(
    scope: &ScopeRoots<'_>,
    kind: AssetKind,
    name: &str,
    content: &str,
) -> Result<String, CoreError> {
    match kind {
        AssetKind::Mcp => save_mcp(scope, name, content),
        AssetKind::Skill => {
            let path = locate_source(scope.default_root, scope.asset_root, kind, name)?;
            fs::write(&path, content).map_err(CoreError::Io)?;
            Ok(format!("skill `{name}` 已保存"))
        }
        AssetKind::Rule | AssetKind::Command => {
            let path = locate_source(scope.default_root, scope.asset_root, kind, name)?;
            fs::write(&path, content).map_err(CoreError::Io)?;
            Ok(format!("{kind:?} `{name}` 已保存"))
        }
        AssetKind::Agent => {
            let src = locate_source(scope.default_root, scope.asset_root, kind, name)?;
            let write_path = agent_edit_path(&src).unwrap_or(src);
            fs::write(&write_path, content).map_err(CoreError::Io)?;
            Ok(format!("agent `{name}` 已保存"))
        }
    }
}

/// 删除源并尽力收回各平台。
///
/// **警告**：会先对 cursor/codex/claude/hermes 全部调用收回，再删除 `asset_root` 源文件。
/// GUI 仅允许在「源视图」经删除按钮 + 确认框调用；平台 icon 不得触发本函数。
pub fn delete_source(
    scope: &ScopeRoots<'_>,
    kind: AssetKind,
    name: &str,
) -> Result<String, CoreError> {
    asset_scope::retract_all_platforms_best_effort(
        scope.default_root,
        scope.asset_root,
        scope.deploy_base,
        kind,
        name,
    );
    match kind {
        AssetKind::Mcp => delete_mcp(scope, name),
        AssetKind::Skill => {
            let skill_md = locate_source(scope.default_root, scope.asset_root, kind, name)?;
            let skill_dir = skill_md
                .parent()
                .ok_or_else(|| CoreError::InvalidPath(format!("skill `{name}` 无父目录")))?;
            let dir_str = skill_dir.to_string();
            fs::remove_dir_all(skill_dir).map_err(CoreError::Io)?;
            Ok(format!("skill `{name}` 已删除 ({dir_str})"))
        }
        AssetKind::Rule | AssetKind::Command => {
            let path = locate_source(scope.default_root, scope.asset_root, kind, name)?;
            let path_str = path.to_string();
            fs::remove_file(&path).map_err(CoreError::Io)?;
            Ok(format!("{kind:?} `{name}` 已删除 ({path_str})"))
        }
        AssetKind::Agent => {
            let src = locate_source(scope.default_root, scope.asset_root, kind, name)?;
            let path_str = src.to_string();
            if src.is_dir() {
                fs::remove_dir_all(&src).map_err(CoreError::Io)?;
            } else {
                fs::remove_file(&src).map_err(CoreError::Io)?;
            }
            Ok(format!("agent `{name}` 已删除 ({path_str})"))
        }
    }
}

/// 从平台已下发路径只读预览（无 ai-config 源时）。
pub fn read_platform_preview(
    kind: AssetKind,
    platform_path: &Utf8Path,
    fallback_name: &str,
) -> Result<AssetFileDetail, CoreError> {
    let read_path = platform_read_path(kind, platform_path)?;
    let content = fs::read_to_string(&read_path).map_err(CoreError::Io)?;
    let (fm_name, description) = parse_skill_meta(&content);
    Ok(AssetFileDetail {
        name: fm_name.unwrap_or_else(|| fallback_name.to_string()),
        description,
        content,
        source_path: read_path.to_string(),
        parent_path: read_path
            .parent()
            .map(|p| p.to_string())
            .unwrap_or_default(),
    })
}

fn ensure_deploy_target(plat: PlatformId) -> Result<(), CoreError> {
    if plat.is_deploy_target() {
        Ok(())
    } else {
        Err(CoreError::InvalidPath(format!(
            "平台 `{}` 为资产源，不能 deploy / retract",
            platform::platform_label(plat)
        )))
    }
}

fn ensure_platform_supports(
    scope: &ScopeRoots<'_>,
    plat: PlatformId,
    kind: AssetKind,
) -> Result<(), CoreError> {
    if platform::supports_at_scope(plat, kind, scope.deploy_base) {
        Ok(())
    } else {
        Err(CoreError::InvalidPath(platform::capability_skip_reason(
            plat, kind,
        )))
    }
}

fn deploy_skill(scope: &ScopeRoots<'_>, name: &str, plat: PlatformId) -> Result<String, CoreError> {
    let skill_md = locate_source(scope.default_root, scope.asset_root, AssetKind::Skill, name)?;
    let src = skill_link_src(&skill_md);
    let dest = asset_dest_for_at_base(plat, AssetKind::Skill, name, &skill_md, scope.deploy_base)
        .ok_or_else(|| CoreError::InvalidPath(format!("无法算 skill `{name}` → {plat:?} 的 dest")))?;
    materialize::deploy(&src, &dest)?;
    if plat == PlatformId::Hermes {
        let skills_parent = dest
            .parent()
            .unwrap_or(&dest)
            .to_path_buf();
        hermes_config::after_skill_deploy(&skills_parent)?;
    }
    Ok(format!("skill `{name}` → {plat:?} OK ({dest})"))
}

fn deploy_materialized(
    scope: &ScopeRoots<'_>,
    kind: AssetKind,
    name: &str,
    plat: PlatformId,
) -> Result<String, CoreError> {
    let src = locate_source(scope.default_root, scope.asset_root, kind, name)?;
    let link_src = link_src_for_create(kind, &src);
    let dest = asset_dest_for_at_base(plat, kind, name, &src, scope.deploy_base)
        .ok_or_else(|| CoreError::InvalidPath(format!("无法算 {kind:?} `{name}` → {plat:?} 的 dest")))?;
    materialize::deploy(&link_src, &dest)?;
    Ok(format!("{kind:?} `{name}` → {plat:?} OK ({dest})"))
}

fn deploy_mcp(scope: &ScopeRoots<'_>, name: &str, plat: PlatformId) -> Result<String, CoreError> {
    let mcp_path = asset_scope::locate_mcp_json(scope.default_root, scope.asset_root)?;
    let doc_root = mcp_asset_root(&mcp_path)?;
    let config = mcp_json::get_server_config(doc_root, name)?
        .ok_or_else(|| CoreError::AssetNotFound {
            kind: AssetKind::Mcp,
            name: name.into(),
            hint: "MCP server 在 mcp.json 中找不到".into(),
        })?;
    let dest = platform::for_scope(plat, scope.deploy_base)?.mcp_deploy_path();
    mcp_json::upsert_server_on_platform(plat, &dest, name, &config)?;
    Ok(format!("mcp `{name}` → {plat:?} OK ({dest})"))
}

fn retract_mcp(scope: &ScopeRoots<'_>, name: &str, plat: PlatformId) -> Result<String, CoreError> {
    let _ = asset_scope::locate_mcp_json(scope.default_root, scope.asset_root)?;
    let dest = platform::for_scope(plat, scope.deploy_base)?.mcp_deploy_path();
    mcp_json::remove_server_on_platform(plat, &dest, name)?;
    Ok(format!("mcp `{name}` ← {plat:?} 已移除 ({dest})"))
}

fn get_mcp_detail(scope: &ScopeRoots<'_>, name: &str) -> Result<AssetFileDetail, CoreError> {
    let mcp_path = asset_scope::locate_mcp_json(scope.default_root, scope.asset_root)?;
    let doc_root = mcp_asset_root(&mcp_path)?;
    let config = mcp_json::get_server_config(doc_root, name)?
        .ok_or_else(|| CoreError::AssetNotFound {
            kind: AssetKind::Mcp,
            name: name.into(),
            hint: "MCP server 找不到".into(),
        })?;
    let content = serde_json::to_string_pretty(&config).map_err(|e| CoreError::Io(e.into()))?;
    Ok(AssetFileDetail {
        name: name.to_string(),
        description: mcp_json::server_transport_summary(&config),
        content,
        source_path: mcp_path.to_string(),
        parent_path: mcp_path.to_string(),
    })
}

fn save_mcp(scope: &ScopeRoots<'_>, name: &str, content: &str) -> Result<String, CoreError> {
    let mcp_path = asset_scope::locate_mcp_json(scope.default_root, scope.asset_root)?;
    let doc_root = mcp_asset_root(&mcp_path)?;
    let config: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| CoreError::InvalidPath(format!("JSON 格式无效: {e}")))?;
    if !config.is_object() {
        return Err(CoreError::InvalidPath(
            "MCP server config 须为 JSON object".into(),
        ));
    }
    mcp_json::upsert_server_in_document(doc_root, name, config)?;
    Ok(format!("mcp `{name}` 已保存"))
}

fn delete_mcp(scope: &ScopeRoots<'_>, name: &str) -> Result<String, CoreError> {
    let mcp_path = asset_scope::locate_mcp_json(scope.default_root, scope.asset_root)?;
    let doc_root = mcp_asset_root(&mcp_path)?;
    mcp_json::remove_server_from_document(doc_root, name)?;
    Ok(format!("mcp `{name}` 已从 mcp.json 移除"))
}

fn mcp_asset_root(mcp_path: &Utf8Path) -> Result<&Utf8Path, CoreError> {
    mcp_path
        .parent()
        .ok_or_else(|| CoreError::InvalidPath(format!("mcp.json 路径无效: {mcp_path}")))
}

fn platform_read_path(kind: AssetKind, platform_path: &Utf8Path) -> Result<Utf8PathBuf, CoreError> {
    match kind {
        AssetKind::Skill => {
            let skill_md = platform_path.join("SKILL.md");
            if skill_md.is_file() {
                Ok(skill_md)
            } else {
                Err(CoreError::InvalidPath(format!(
                    "平台 skill 无 SKILL.md: {platform_path}"
                )))
            }
        }
        AssetKind::Rule | AssetKind::Command => {
            if platform_path.is_file() {
                Ok(platform_path.to_path_buf())
            } else {
                Err(CoreError::InvalidPath(format!(
                    "平台 {kind:?} 路径不是文件: {platform_path}"
                )))
            }
        }
        AssetKind::Agent => Ok(agent_read_path(platform_path)),
        AssetKind::Mcp => Err(CoreError::InvalidPath(
            "MCP 详情须从 ai-config 源查看".into(),
        )),
    }
}

fn agent_edit_path(src: &Utf8Path) -> Option<Utf8PathBuf> {
    if !src.is_dir() {
        return None;
    }
    let agent_md = src.join("AGENT.md");
    if agent_md.is_file() {
        return Some(agent_md);
    }
    let entries = fs::read_dir(src.as_std_path()).ok()?;
    for entry in entries.flatten() {
        let path = Utf8PathBuf::from_path_buf(entry.path()).ok()?;
        if path.extension().map(|e| e == "md").unwrap_or(false) {
            return Some(path);
        }
    }
    None
}

fn agent_read_path(src: &Utf8Path) -> Utf8PathBuf {
    if src.is_dir() {
        agent_edit_path(src).unwrap_or_else(|| src.join("AGENT.md"))
    } else {
        src.to_path_buf()
    }
}

fn skill_link_src(skill_md: &Utf8Path) -> Utf8PathBuf {
    skill_md
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| skill_md.to_path_buf())
}

/// YAML block scalar 指示符（`>` / `>-` / `>+` / `|` / `|-` / `|+`）。
fn yaml_block_scalar_indicator(s: &str) -> Option<bool> {
    match s.trim() {
        ">" | ">-" | ">+" => Some(true),
        "|" | "|-" | "|+" => Some(false),
        _ => None,
    }
}

/// 拆分 `description:` 行上的 block 指示符与同行剩余正文。
fn split_yaml_block_scalar_prefix(s: &str) -> Option<(bool, &str)> {
    let s = s.trim();
    for (ind, folded) in [
        (">-", true),
        (">+", true),
        (">", true),
        ("|-", false),
        ("|+", false),
        ("|", false),
    ] {
        if let Some(rest) = s.strip_prefix(ind) {
            return Some((folded, rest.trim_start()));
        }
    }
    None
}

fn collect_yaml_block_scalar_lines(lines: &[&str], start: usize, folded: bool) -> (String, usize) {
    let mut parts = Vec::new();
    let mut i = start;
    while i < lines.len() {
        let cont = lines[i];
        let trimmed = cont.trim();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        if is_frontmatter_key_line(cont) {
            break;
        }
        let text = trimmed
            .strip_prefix('>')
            .map(str::trim)
            .unwrap_or(trimmed);
        if !text.is_empty() {
            parts.push(text.to_string());
        }
        i += 1;
    }
    let joined = if folded {
        parts.join(" ")
    } else {
        parts.join(" ")
    };
    (joined.chars().take(200).collect(), i)
}

/// 从 frontmatter / 正文抽 name、description。
pub fn parse_skill_meta(content: &str) -> (Option<String>, String) {
    let mut name = None;
    let mut description = String::new();
    if let Some(after_first) = content.strip_prefix("---") {
        let after_first = after_first.trim_start_matches('\n');
        if let Some(end) = after_first.find("\n---") {
            let lines: Vec<&str> = after_first[..end].lines().collect();
            let mut i = 0;
            while i < lines.len() {
                let raw = lines[i];
                let line = raw.trim();
                if let Some(rest) = line.strip_prefix("name:") {
                    name = Some(rest.trim().trim_matches(['"', '\'']).to_string());
                } else if let Some(rest) = line.strip_prefix("description:") {
                    let d = rest.trim();
                    if let Some((folded, remainder)) = split_yaml_block_scalar_prefix(d) {
                        if remainder.is_empty() {
                            let (text, next_i) =
                                collect_yaml_block_scalar_lines(&lines, i + 1, folded);
                            description = text;
                            i = next_i;
                            continue;
                        }
                        description = remainder.chars().take(200).collect();
                    } else {
                        let inline = d.trim_matches(['"', '\'']);
                        if !inline.is_empty() {
                            description = inline.chars().take(200).collect();
                        }
                    }
                } else if description.is_empty()
                    && line.starts_with('>')
                    && yaml_block_scalar_indicator(line).is_none()
                {
                    description = line
                        .trim_start_matches('>')
                        .trim()
                        .chars()
                        .take(200)
                        .collect();
                }
                i += 1;
            }
        }
    }
    if description.is_empty() {
        for line in content.lines() {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("# ") {
                description = rest
                    .chars()
                    .take(200)
                    .collect::<String>()
                    .trim()
                    .to_string();
                break;
            }
        }
    }
    (name, description)
}

/// 列表 / 扫描用的一句话描述。
pub fn parse_skill_description(content: &str) -> String {
    parse_skill_meta(content).1
}

fn is_frontmatter_key_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    let Some((key, _)) = trimmed.split_once(':') else {
        return false;
    };
    let key = key.trim();
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn parse_description(kind: AssetKind, src: &Utf8Path) -> String {
    match kind {
        AssetKind::Skill | AssetKind::Rule | AssetKind::Command => fs::read_to_string(src)
            .ok()
            .map(|c| parse_skill_meta(&c).1)
            .unwrap_or_default(),
        AssetKind::Mcp => fs::read_to_string(src)
            .ok()
            .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
            .and_then(|v| {
                v.get("description")
                    .and_then(|d| d.as_str())
                    .map(|s| s.chars().take(80).collect::<String>())
            })
            .unwrap_or_default(),
        AssetKind::Agent => fs::read_to_string(agent_read_path(src))
            .ok()
            .map(|c| parse_skill_meta(&c).1)
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skill_meta_extracts_description_from_h1() {
        let (_, d) = parse_skill_meta("# Hello World\n\nbody");
        assert_eq!(d, "Hello World");
    }

    #[test]
    fn parse_skill_meta_yaml_folded_description() {
        let content = "---\ndescription: >\n  第一段说明\n  第二段说明\nname: demo\n---\n# Title\n";
        let (name, d) = parse_skill_meta(content);
        assert_eq!(name.as_deref(), Some("demo"));
        assert_eq!(d, "第一段说明 第二段说明");
    }

    #[test]
    fn parse_skill_meta_yaml_blockquote_lines() {
        let content = "---\ndescription: >\n  > 引用式说明\n  > 续行内容\n---\n";
        let (_, d) = parse_skill_meta(content);
        assert_eq!(d, "引用式说明 续行内容");
    }

    #[test]
    fn parse_skill_meta_yaml_folded_strip_chomp_indicator() {
        let content = "---\ndescription: >-\n  第一段说明\n  第二段说明\nname: demo\n---\n";
        let (name, d) = parse_skill_meta(content);
        assert_eq!(name.as_deref(), Some("demo"));
        assert_eq!(d, "第一段说明 第二段说明");
    }

    #[test]
    fn parse_skill_meta_yaml_folded_inline_after_indicator() {
        let content = "---\ndescription: >- 同行折叠说明\nname: demo\n---\n";
        let (_, d) = parse_skill_meta(content);
        assert_eq!(d, "同行折叠说明");
    }

    #[test]
    fn parse_skill_meta_yaml_literal_strip_indicator() {
        let content = "---\ndescription: |-\n  行一\n  行二\n---\n";
        let (_, d) = parse_skill_meta(content);
        assert_eq!(d, "行一 行二");
    }
}
