//! 平台侧资产扫描（反向同步 / platform view）。
//!
//! 从各平台适配器目录枚举 skills / rules / agents / MCP server，
//! 并与 ai-config 源对比得出 `SourceState`。

use std::fs;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::asset_ops::parse_skill_description;
use crate::error::CoreError;
use crate::hook_lifecycle::{self, HookLifecycleView};
use crate::mcp_json;
use crate::model::{AssetKind, PlatformId};
use crate::platform::{self, PlatformAdapter};
use crate::source::{self, ScanResult};
use crate::sync::{asset_dest_for_at_base, link_src_for_create};
use crate::template::read_mcp_json;

/// 平台条目相对 ai-config 源的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    /// 源存在且平台链接正确指向源
    Managed,
    /// 源存在且已链接（与 managed 同义，保留前端兼容）
    Linked,
    /// 仅存在于平台，源中无同名资产
    Unmanaged,
    /// 平台有条目但源缺失或链接破损
    Orphan,
}

/// 单条平台资产（GUI platform view 一行）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformAssetEntry {
    pub name: String,
    pub kind: AssetKind,
    pub description: String,
    pub platform_path: String,
    /// 相对 ai-config 源的纳管态（兼容 import 提示）
    pub source_state: SourceState,
    /// 5 平台同步状态（含 ai-config 源）
    pub states: std::collections::HashMap<PlatformId, LinkState>,
    /// Hook 专用：各生命周期开关状态（仅 `kind == Hook` 时有值）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_lifecycles: Option<Vec<HookLifecycleView>>,
    /// Hook 专用：`command` | `prompt`（仅 `kind == Hook` 时有值）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_type: Option<String>,
}

/// 平台链接状态（与 GUI `LinkState` 对齐）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    /// 本工具纳管的下发（可收回）
    Linked,
    /// 内容与源/镜像一致，但非本工具下发（`npx skills` 等；可覆盖下发，不可收回）
    Synced,
    Unlinked,
    Broken,
    Missing,
}

/// 平台资产列表（`cmd_list_platform` 响应）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformAssetList {
    pub entries: Vec<PlatformAssetEntry>,
    pub platform: PlatformId,
}

/// 扫描指定平台在某作用域下的某类资产。
pub fn scan_platform_assets(
    plat: PlatformId,
    kind: AssetKind,
    deploy_base: &Utf8Path,
    default_root: &Utf8Path,
    asset_root: &Utf8Path,
) -> Result<Vec<PlatformAssetEntry>, CoreError> {
    if plat == PlatformId::AiConfig {
        return scan_aiconfig_assets(kind, default_root, asset_root);
    }
    let adapter = platform::for_scope_with_asset(plat, deploy_base, asset_root)?;
    if !adapter.supports(kind) {
        return Ok(Vec::new());
    }

    let source_scan = scan_source_for_scope(default_root, asset_root)?;
    let raw = scan_platform_raw(adapter.as_ref(), kind)?;

    let mut out = Vec::with_capacity(raw.len());
    for (name, platform_path, description, hook_type, binding_key) in raw {
        let state_key = if kind == AssetKind::Hook {
            binding_key.as_str()
        } else {
            name.as_str()
        };
        let states = compute_entry_states(
            plat,
            kind,
            state_key,
            &platform_path,
            deploy_base,
            asset_root,
            &source_scan,
        );
        let source_state = if states.get(&PlatformId::AiConfig) == Some(&LinkState::Linked) {
            SourceState::Managed
        } else {
            SourceState::Unmanaged
        };
        let hook_lifecycles = hook_lifecycle_views(kind, plat, state_key, asset_root, deploy_base);
        out.push(PlatformAssetEntry {
            name,
            kind,
            description,
            platform_path: platform_path.to_string(),
            source_state,
            states,
            hook_lifecycles,
            hook_type,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// ai-config 源视图：直接枚举 asset_root，条目均为已纳管。
fn scan_aiconfig_assets(
    kind: AssetKind,
    default_root: &Utf8Path,
    asset_root: &Utf8Path,
) -> Result<Vec<PlatformAssetEntry>, CoreError> {
    let deploy_base = if asset_root == default_root {
        crate::paths::global_deploy_base()
    } else {
        asset_root
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(crate::paths::global_deploy_base)
    };
    let adapter = platform::aiconfig_adapter(asset_root);
    let source_scan = scan_source_for_scope(default_root, asset_root)?;
    let raw = if kind == AssetKind::Hook {
        crate::hook::list_hook_catalog(asset_root)?
            .into_iter()
            .map(|item| {
                (
                    item.name,
                    item.source_path,
                    item.description,
                    Some(match item.hook_type {
                        crate::hook::HookEntryType::Command => "command".to_string(),
                        crate::hook::HookEntryType::Prompt => "prompt".to_string(),
                    }),
                    item.binding_key,
                )
            })
            .collect()
    } else {
        scan_platform_raw(adapter.as_ref(), kind)?
    };
    let mut out: Vec<PlatformAssetEntry> = raw
        .into_iter()
        .map(
            |(name, platform_path, description, hook_type, binding_key)| {
                let state_key = if kind == AssetKind::Hook {
                    binding_key.as_str()
                } else {
                    name.as_str()
                };
                let states = compute_entry_states(
                    PlatformId::AiConfig,
                    kind,
                    state_key,
                    &platform_path,
                    &deploy_base,
                    asset_root,
                    &source_scan,
                );
                let hook_lifecycles = hook_lifecycle_views(
                    kind,
                    PlatformId::AiConfig,
                    state_key,
                    asset_root,
                    &deploy_base,
                );
                PlatformAssetEntry {
                    name,
                    kind,
                    description,
                    platform_path: platform_path.to_string(),
                    source_state: SourceState::Managed,
                    states,
                    hook_lifecycles,
                    hook_type,
                }
            },
        )
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn hook_lifecycle_views(
    kind: AssetKind,
    browse_plat: PlatformId,
    script_filename: &str,
    asset_root: &Utf8Path,
    deploy_base: &Utf8Path,
) -> Option<Vec<HookLifecycleView>> {
    if kind != AssetKind::Hook {
        return None;
    }
    Some(hook_lifecycle::list_lifecycle_views(
        asset_root,
        deploy_base,
        browse_plat,
        script_filename,
    ))
}

pub fn scan_source_for_scope(
    default_root: &Utf8Path,
    asset_root: &Utf8Path,
) -> Result<ScanResult, CoreError> {
    if asset_root != default_root {
        if !asset_root.exists() {
            return Ok(ScanResult::default());
        }
        return source::scan_project_root(asset_root);
    }
    source::scan_project_root(asset_root)
}

type RawEntry = (String, Utf8PathBuf, String, Option<String>, String);

fn scan_platform_raw(
    adapter: &dyn PlatformAdapter,
    kind: AssetKind,
) -> Result<Vec<RawEntry>, CoreError> {
    match kind {
        AssetKind::Skill => scan_platform_skills(adapter),
        AssetKind::Rule => scan_platform_rules(adapter),
        AssetKind::Agent => scan_platform_agents(adapter),
        AssetKind::Command => scan_platform_commands(adapter),
        AssetKind::Hook => scan_platform_hooks(adapter),
        AssetKind::Mcp => scan_platform_mcp(adapter),
    }
}

fn is_noise_entry_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name);
    let is_readme = stem.eq_ignore_ascii_case("README");
    name.starts_with('.')
        || crate::link::is_legacy_bak_entry_name(name)
        || name.ends_with(".ai-config-deploy.json")
        || name.ends_with(".orig")
        || name.ends_with('~')
        || is_readme
}

fn platform_asset_key_from_command(command: &str, hooks_dir: &Utf8Path) -> Option<String> {
    let executable = command.split_whitespace().next()?.trim();
    if executable.is_empty() {
        return None;
    }
    let rel = executable.strip_prefix("./").unwrap_or(executable);
    let path = Utf8Path::new(rel);
    let components: Vec<_> = path.iter().collect();
    if let Some(i) = components.iter().position(|c| *c == "hooks") {
        let rest = &components[i + 1..];
        if rest.is_empty() {
            return None;
        }
        if rest.len() >= 2 && hooks_dir.join(rest[0]).is_dir() {
            return Some(rest[0].to_string());
        }
        return rest.last().map(|s| s.to_string());
    }
    path.file_name().map(str::to_string)
}

fn scan_platform_hooks(adapter: &dyn PlatformAdapter) -> Result<Vec<RawEntry>, CoreError> {
    let plat = adapter.id();
    if plat == PlatformId::AiConfig {
        return Ok(Vec::new());
    }
    let deploy_base = adapter.skills_dir();
    let deploy_base = deploy_base
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(&deploy_base)
        .to_path_buf();
    let config_path = crate::hook_adapter::platform_config_path_for(&deploy_base, plat);
    let hooks_root = crate::hook_adapter::platform_hooks_dir(&deploy_base, plat);
    let mut by_key: std::collections::HashMap<String, RawEntry> = std::collections::HashMap::new();

    if hooks_root.is_dir() {
        for entry in fs::read_dir(hooks_root.as_std_path()).map_err(CoreError::Io)? {
            let entry = entry.map_err(CoreError::Io)?;
            let path = Utf8PathBuf::from_path_buf(entry.path()).unwrap_or_default();
            let Some(name) = path.file_name().map(str::to_string) else {
                continue;
            };
            if name.starts_with('.') || is_noise_entry_name(&name) {
                continue;
            }
            if path.is_file() {
                if !crate::hook::is_hook_script_file(&name) {
                    continue;
                }
                let description = fs::read_to_string(path.as_std_path())
                    .ok()
                    .map(|c| crate::hook::parse_script_description(&c))
                    .unwrap_or_default();
                by_key.insert(
                    name.clone(),
                    (
                        name.clone(),
                        path,
                        description,
                        Some("command".into()),
                        name,
                    ),
                );
            } else if path.is_dir() {
                by_key.insert(
                    name.clone(),
                    (
                        name.clone(),
                        path,
                        String::new(),
                        Some("command".into()),
                        name,
                    ),
                );
            }
        }
    }

    if config_path.is_file() {
        let raw = fs::read_to_string(config_path.as_std_path()).map_err(CoreError::Io)?;
        let doc: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| CoreError::TemplateRender {
                template: config_path.as_str().to_owned(),
                reason: e.to_string(),
                hint: "hooks.json 解析失败".into(),
            })?;
        if let Some(hooks) = doc.get("hooks").and_then(|v| v.as_object()) {
            for (lifecycle, entries) in hooks {
                let Some(arr) = entries.as_array() else {
                    continue;
                };
                for (index, entry) in arr.iter().enumerate() {
                    let entry_type = entry
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("command")
                        .to_ascii_lowercase();
                    let prompt = entry
                        .get("prompt")
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                    if entry_type == "prompt"
                        || (entry.get("command").is_none() && prompt.is_some())
                    {
                        let key = format!("prompt:{lifecycle}:{index}");
                        let title = prompt
                            .as_deref()
                            .filter(|p| !p.is_empty())
                            .map(crate::hook::prompt_entry_title)
                            .unwrap_or_else(|| format!("prompt@{lifecycle}"));
                        by_key.insert(
                            key.clone(),
                            (
                                title,
                                config_path.clone(),
                                String::new(),
                                Some("prompt".into()),
                                key,
                            ),
                        );
                        continue;
                    }
                    let Some(command) = entry.get("command").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let Some(key) = platform_asset_key_from_command(command, &hooks_root) else {
                        continue;
                    };
                    if is_noise_entry_name(&key) {
                        continue;
                    }
                    let path = hooks_root.join(&key);
                    let resolved = if path.is_file() || path.is_dir() {
                        path
                    } else {
                        hooks_root.join(
                            crate::hook::filename_from_command(command).unwrap_or(key.clone()),
                        )
                    };
                    let description = if resolved.is_file() {
                        fs::read_to_string(resolved.as_std_path())
                            .ok()
                            .map(|c| crate::hook::parse_script_description(&c))
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };
                    by_key.insert(
                        key.clone(),
                        (
                            key.clone(),
                            resolved,
                            description,
                            Some("command".into()),
                            key,
                        ),
                    );
                }
            }
        }
    }

    let mut out: Vec<RawEntry> = by_key.into_values().collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn scan_platform_skills(adapter: &dyn PlatformAdapter) -> Result<Vec<RawEntry>, CoreError> {
    let dir = adapter.skills_dir();
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(dir.as_std_path()).map_err(CoreError::Io)? {
        let entry = entry.map_err(CoreError::Io)?;
        let name = entry.file_name().to_string_lossy().to_string();
        if is_noise_entry_name(&name) {
            continue;
        }
        let entry_path = dir.join(&name);
        // Path::is_dir / is_file 跟随 symlink（平台 deploy 后 skills 常为链接）
        if !entry_path.is_dir() {
            continue;
        }
        let skill_md = entry_path.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        let description = fs::read_to_string(skill_md.as_std_path())
            .ok()
            .map(|c| parse_skill_description(&c))
            .unwrap_or_default();
        out.push((name.clone(), entry_path, description, None, name));
    }
    Ok(out)
}

fn scan_platform_rules(adapter: &dyn PlatformAdapter) -> Result<Vec<RawEntry>, CoreError> {
    let dir = adapter.rules_dir();
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(dir.as_std_path()).map_err(CoreError::Io)? {
        let entry = entry.map_err(CoreError::Io)?;
        let fname = entry.file_name().to_string_lossy().to_string();
        if is_noise_entry_name(&fname) {
            continue;
        }
        let path = Utf8PathBuf::from_path_buf(entry.path()).map_err(|_| {
            CoreError::InvalidPath(format!("非 UTF-8 路径: {}", entry.path().display()))
        })?;
        if !path.is_file() {
            continue;
        }
        if !(fname.ends_with(".mdc") || fname.ends_with(".md")) {
            continue;
        }
        let stem = path.file_stem().unwrap_or(&fname).to_string();
        let description = fs::read_to_string(path.as_std_path())
            .ok()
            .map(|c| parse_skill_description(&c))
            .unwrap_or_default();
        out.push((stem.clone(), path, description, None, stem));
    }
    Ok(out)
}

fn scan_platform_commands(adapter: &dyn PlatformAdapter) -> Result<Vec<RawEntry>, CoreError> {
    let dir = adapter.commands_dir();
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(dir.as_std_path()).map_err(CoreError::Io)? {
        let entry = entry.map_err(CoreError::Io)?;
        let fname = entry.file_name().to_string_lossy().to_string();
        if is_noise_entry_name(&fname) {
            continue;
        }
        let path = Utf8PathBuf::from_path_buf(entry.path()).map_err(|_| {
            CoreError::InvalidPath(format!("非 UTF-8 路径: {}", entry.path().display()))
        })?;
        if !path.is_file() || !fname.ends_with(".md") {
            continue;
        }
        let stem = path.file_stem().unwrap_or(&fname).to_string();
        let description = fs::read_to_string(path.as_std_path())
            .ok()
            .map(|c| parse_skill_description(&c))
            .unwrap_or_default();
        out.push((stem.clone(), path, description, None, stem));
    }
    Ok(out)
}

fn scan_platform_agents(adapter: &dyn PlatformAdapter) -> Result<Vec<RawEntry>, CoreError> {
    let dir = adapter.agents_dir();
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in WalkDir::new(dir.as_std_path())
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .filter_map(Result::ok)
    {
        let name_os = entry.file_name().to_string_lossy();
        if name_os.starts_with('.') || is_noise_entry_name(&name_os) {
            continue;
        }
        let path = Utf8PathBuf::from_path_buf(entry.into_path())
            .map_err(|_| CoreError::InvalidPath("agents 路径非 UTF-8".into()))?;
        let name = if path.is_dir() {
            path.file_name().map(|s| s.to_string())
        } else {
            path.file_stem().map(|s| s.to_string())
        };
        let Some(name) = name else { continue };
        let description = agent_description_at(&path);
        out.push((name.clone(), path, description, None, name));
    }
    Ok(out)
}

fn agent_description_at(path: &Utf8Path) -> String {
    let read_path = if path.is_dir() {
        let agent_md = path.join("AGENT.md");
        if agent_md.is_file() {
            agent_md
        } else {
            fs::read_dir(path.as_std_path())
                .ok()
                .and_then(|entries| {
                    for e in entries.flatten() {
                        let p = e.path();
                        if p.extension().map(|x| x == "md").unwrap_or(false) {
                            return Utf8PathBuf::from_path_buf(p).ok();
                        }
                    }
                    None
                })
                .unwrap_or_else(|| path.join("AGENT.md"))
        }
    } else {
        path.to_path_buf()
    };
    fs::read_to_string(read_path.as_std_path())
        .ok()
        .map(|c| parse_skill_description(&c))
        .unwrap_or_default()
}

fn scan_platform_mcp(adapter: &dyn PlatformAdapter) -> Result<Vec<RawEntry>, CoreError> {
    let path = adapter.mcp_deploy_path();
    if adapter.id() == PlatformId::Hermes {
        return scan_hermes_mcp_servers(&path);
    }
    let Some(doc) = read_mcp_json(&path)? else {
        return Ok(Vec::new());
    };
    let mut names: Vec<String> = doc
        .get("mcpServers")
        .and_then(|v| v.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    names.sort();
    Ok(names
        .into_iter()
        .map(|name| {
            let cfg = doc
                .get("mcpServers")
                .and_then(|v| v.get(&name))
                .cloned()
                .unwrap_or_default();
            let description = mcp_json::server_transport_summary(&cfg);
            (name.clone(), path.clone(), description, None, name)
        })
        .collect())
}

fn scan_hermes_mcp_servers(config_path: &Utf8Path) -> Result<Vec<RawEntry>, CoreError> {
    let raw = fs::read_to_string(config_path.as_std_path());
    let Ok(raw) = raw else {
        return Ok(Vec::new());
    };
    let doc: serde_yaml::Value = match serde_yaml::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return Ok(Vec::new()),
    };
    let serde_yaml::Value::Mapping(map) = doc else {
        return Ok(Vec::new());
    };
    let Some(serde_yaml::Value::Mapping(servers)) =
        map.get(serde_yaml::Value::String("mcp_servers".into()))
    else {
        return Ok(Vec::new());
    };
    let mut names: Vec<String> = servers
        .keys()
        .filter_map(|k| k.as_str().map(str::to_owned))
        .collect();
    names.sort();
    Ok(names
        .into_iter()
        .map(|name| {
            let cfg_yaml = servers
                .get(serde_yaml::Value::String(name.clone()))
                .cloned()
                .unwrap_or(serde_yaml::Value::Null);
            let cfg: serde_json::Value = serde_json::to_value(cfg_yaml).unwrap_or_default();
            let description = mcp_json::server_transport_summary(&cfg);
            (
                name.clone(),
                config_path.to_path_buf(),
                description,
                None,
                name,
            )
        })
        .collect())
}

pub fn find_source_path(
    kind: AssetKind,
    name: &str,
    source_scan: &ScanResult,
) -> Option<Utf8PathBuf> {
    match kind {
        AssetKind::Skill => source_scan
            .skills
            .iter()
            .find(|p| {
                p.parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n == name)
                    .unwrap_or(false)
            })
            .cloned(),
        AssetKind::Rule => source_scan
            .rules
            .iter()
            .find(|p| p.file_stem().map(|n| n == name).unwrap_or(false))
            .cloned(),
        AssetKind::Agent => source_scan
            .agents
            .iter()
            .find(|p| {
                let candidate = if p.is_dir() {
                    p.file_name().map(|s| s.to_string())
                } else {
                    p.file_stem().map(|s| s.to_string())
                };
                candidate.as_deref() == Some(name)
            })
            .cloned(),
        AssetKind::Command => source_scan
            .commands
            .iter()
            .find(|p| p.file_stem().map(|n| n == name).unwrap_or(false))
            .cloned(),
        AssetKind::Mcp => {
            let asset_root = source_scan.mcp_json.as_ref()?.parent()?;
            mcp_json::get_server_config(asset_root, name)
                .ok()
                .flatten()?;
            source_scan.mcp_json.as_ref().cloned()
        }
        AssetKind::Hook => source_scan
            .hooks
            .iter()
            .find(|item| item.script_filename == name)
            .map(|item| item.script_path.clone()),
    }
}

fn compute_entry_states(
    browse_plat: PlatformId,
    kind: AssetKind,
    name: &str,
    platform_path: &Utf8Path,
    deploy_base: &Utf8Path,
    asset_root: &Utf8Path,
    source_scan: &ScanResult,
) -> std::collections::HashMap<PlatformId, LinkState> {
    use std::collections::HashMap;

    let src = find_source_path(kind, name, source_scan);
    let mut states = HashMap::new();
    for plat in platform::ui_platform_ids() {
        let st = if plat == browse_plat {
            browse_platform_link_state(
                kind,
                platform_path,
                src.as_deref(),
                deploy_base,
                browse_plat,
                name,
            )
        } else if plat == PlatformId::AiConfig {
            // IDE 视图下，ai-config icon 仅反映源是否存在该资产（避免被平台副本元数据干扰）
            if src.as_ref().is_some_and(|s| s.exists()) {
                LinkState::Linked
            } else if browse_plat != PlatformId::AiConfig
                && std::fs::symlink_metadata(platform_path.as_std_path()).is_ok()
            {
                LinkState::Missing
            } else {
                LinkState::Unlinked
            }
        } else if browse_plat == PlatformId::AiConfig {
            if kind == AssetKind::Mcp {
                if let Some(src_path) = src.as_deref() {
                    ide_mcp_link_state(plat, name, Some(src_path), deploy_base)
                } else {
                    platform_mirror_link_state(
                        kind,
                        name,
                        platform_path,
                        plat,
                        deploy_base,
                        asset_root,
                    )
                }
            } else if let Some(src_path) = src.as_ref() {
                ide_asset_link_state(plat, kind, name, src_path, deploy_base)
            } else {
                platform_mirror_link_state(kind, name, platform_path, plat, deploy_base, asset_root)
            }
        } else if !platform::supports_at_scope(plat, kind, deploy_base) {
            LinkState::Unlinked
        } else {
            platform_mirror_link_state(kind, name, platform_path, plat, deploy_base, asset_root)
        };
        states.insert(plat, st);
    }
    states
}

fn browse_platform_link_state(
    kind: AssetKind,
    platform_path: &Utf8Path,
    src: Option<&Utf8Path>,
    _deploy_base: &Utf8Path,
    browse_plat: PlatformId,
    _name: &str,
) -> LinkState {
    // ai-config 源视图：列表行在源中存在即视为已链接
    if browse_plat == PlatformId::AiConfig {
        if let Some(src_path) = src {
            if src_path.exists() {
                return LinkState::Linked;
            }
        }
        if std::fs::symlink_metadata(platform_path.as_std_path()).is_err() {
            return LinkState::Missing;
        }
        if kind == AssetKind::Mcp {
            return LinkState::Synced;
        }
        if crate::materialize::is_managed_deploy(platform_path) {
            return LinkState::Linked;
        }
        return LinkState::Synced;
    }
    // IDE 平台视图：左侧选中平台即基准，存在即激活
    if std::fs::symlink_metadata(platform_path.as_std_path()).is_err() {
        return LinkState::Missing;
    }
    if kind == AssetKind::Mcp {
        return LinkState::Synced;
    }
    if crate::materialize::is_managed_deploy(platform_path) {
        LinkState::Linked
    } else {
        LinkState::Synced
    }
}

/// 无 ai-config 源时：目标平台是否与当前浏览平台上的资产内容一致（跨 IDE 硬拷贝）。
fn platform_mirror_link_state(
    kind: AssetKind,
    name: &str,
    mirror_platform_path: &Utf8Path,
    dest_plat: PlatformId,
    deploy_base: &Utf8Path,
    asset_root: &Utf8Path,
) -> LinkState {
    let Ok(dest_adapter) = platform::for_scope_with_asset(dest_plat, deploy_base, asset_root)
    else {
        return LinkState::Unlinked;
    };
    let dest = match kind {
        AssetKind::Skill => dest_adapter.skills_dir().join(name),
        AssetKind::Rule => dest_adapter.rules_dir().join(format!("{name}.mdc")),
        AssetKind::Command => dest_adapter.commands_dir().join(format!("{name}.md")),
        AssetKind::Agent => {
            if mirror_platform_path.is_dir() {
                dest_adapter.agents_dir().join(name)
            } else {
                let ext = mirror_platform_path
                    .extension()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "md".to_string());
                dest_adapter.agents_dir().join(format!("{name}.{ext}"))
            }
        }
        AssetKind::Mcp => {
            let Ok(expected) =
                mcp_json::get_server_config_from_deploy_file(mirror_platform_path, name)
            else {
                return LinkState::Unlinked;
            };
            let dest_path = dest_adapter.mcp_deploy_path();
            let sync = if dest_plat == PlatformId::Hermes {
                crate::hermes_config::mcp_server_sync_state(&dest_path, name, &expected)
            } else {
                crate::template::mcp_server_sync_state(&dest_path, name, &expected)
            };
            return match sync {
                crate::template::McpSyncState::Linked => LinkState::Synced,
                crate::template::McpSyncState::Unlinked => LinkState::Missing,
                crate::template::McpSyncState::WrongValue
                | crate::template::McpSyncState::Broken => LinkState::Unlinked,
            };
        }
        AssetKind::Hook => {
            return if crate::hook_adapter::is_deployed(deploy_base, dest_plat, name) {
                LinkState::Synced
            } else {
                LinkState::Missing
            };
        }
    };
    if fs::symlink_metadata(dest.as_std_path()).is_err() {
        return LinkState::Missing;
    }
    let matches = match kind {
        AssetKind::Skill => {
            let skill_md = mirror_platform_path.join("SKILL.md");
            crate::materialize::content_matches_source(AssetKind::Skill, &skill_md, &dest)
        }
        AssetKind::Rule | AssetKind::Command | AssetKind::Agent => {
            crate::materialize::content_matches_source(kind, mirror_platform_path, &dest)
        }
        AssetKind::Mcp => false,
        AssetKind::Hook => unreachable!("handled above"),
    };
    if !matches {
        return LinkState::Unlinked;
    }
    if crate::materialize::is_managed_deploy(&dest) {
        LinkState::Linked
    } else {
        LinkState::Synced
    }
}

fn ide_mcp_link_state(
    plat: PlatformId,
    name: &str,
    src: Option<&Utf8Path>,
    deploy_base: &Utf8Path,
) -> LinkState {
    let Some(src) = src else {
        return LinkState::Unlinked;
    };
    let mcp_root = src.parent().unwrap_or(src);
    match mcp_json::mcp_server_sync_state_for_platform_at(mcp_root, name, plat, deploy_base) {
        crate::template::McpSyncState::Linked => LinkState::Linked,
        crate::template::McpSyncState::Unlinked => LinkState::Unlinked,
        crate::template::McpSyncState::WrongValue | crate::template::McpSyncState::Broken => {
            LinkState::Broken
        }
    }
}

fn deploy_health_to_link_state(
    dest: &Utf8Path,
    health: crate::materialize::DeployHealth,
) -> LinkState {
    match health {
        crate::materialize::DeployHealth::Linked { .. } => LinkState::Linked,
        crate::materialize::DeployHealth::Broken => LinkState::Broken,
        crate::materialize::DeployHealth::Unlinked => {
            if fs::symlink_metadata(dest.as_std_path()).is_ok() {
                LinkState::Unlinked
            } else {
                LinkState::Missing
            }
        }
    }
}

fn ide_asset_link_state(
    plat: PlatformId,
    kind: AssetKind,
    name: &str,
    src: &Utf8Path,
    deploy_base: &Utf8Path,
) -> LinkState {
    if kind == AssetKind::Hook {
        return if crate::hook_adapter::is_deployed(deploy_base, plat, name) {
            LinkState::Linked
        } else {
            LinkState::Unlinked
        };
    }
    let dest = match asset_dest_for_at_base(plat, kind, name, src, deploy_base) {
        Some(d) => d,
        None => return LinkState::Unlinked,
    };
    if !src.exists() {
        return LinkState::Broken;
    }
    let expected_src = link_src_for_create(kind, src);
    deploy_health_to_link_state(&dest, crate::materialize::check(&dest, &expected_src))
}

fn is_symlink_path(path: &Utf8Path) -> bool {
    fs::symlink_metadata(path.as_std_path())
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

// ── 从平台导入到 ai-config 源 ─────────────────────────────────────

/// 将平台 skill 目录复制到 `asset_root/skills/<name>/`。
pub fn import_skill_from_platform(
    name: &str,
    plat: PlatformId,
    default_root: &Utf8Path,
    asset_root: &Utf8Path,
    deploy_base: &Utf8Path,
) -> Result<Utf8PathBuf, CoreError> {
    if plat == PlatformId::AiConfig {
        return Err(CoreError::InvalidPath(
            "ai-config 为资产源，不能从自身导入".into(),
        ));
    }
    let adapter = platform::for_scope_with_asset(plat, deploy_base, asset_root)?;
    let platform_dir = adapter.skills_dir().join(name);
    if !platform_dir.join("SKILL.md").is_file() {
        return Err(CoreError::AssetNotFound {
            kind: AssetKind::Skill,
            name: name.into(),
            hint: format!("平台 `{}` 上找不到 skill `{name}`", plat_label(plat)),
        });
    }

    let source_scan = scan_source_for_scope(default_root, asset_root)?;
    if find_source_path(AssetKind::Skill, name, &source_scan).is_some() {
        return Err(CoreError::InvalidPath(format!(
            "源中已存在 skill `{name}`，请先删除或重命名"
        )));
    }

    if is_symlink_path(&platform_dir) {
        if let Ok(target) = fs::read_link(platform_dir.as_std_path()) {
            let target = Utf8PathBuf::from_path_buf(target).unwrap_or_default();
            let expected = asset_root.join("skills").join(name);
            if (target == expected || target.ends_with(name))
                && find_source_path(AssetKind::Skill, name, &source_scan).is_some()
            {
                return Ok(expected);
            }
        }
    }

    let dest_dir = asset_root.join("skills").join(name);
    let material_src = crate::materialize::resolve_copy_source(&platform_dir);
    crate::materialize::copy_tree(&material_src, &dest_dir)?;
    let skill_md = dest_dir.join("SKILL.md");
    let link_src = crate::sync::link_src_for_create(AssetKind::Skill, &skill_md);
    crate::materialize::adopt_existing_deploy(&link_src, &platform_dir)?;
    Ok(dest_dir)
}

/// 将平台 rule 文件复制到 `asset_root/rules/<name>.mdc`。
pub fn import_rule_from_platform(
    name: &str,
    plat: PlatformId,
    default_root: &Utf8Path,
    asset_root: &Utf8Path,
    deploy_base: &Utf8Path,
) -> Result<Utf8PathBuf, CoreError> {
    if plat == PlatformId::AiConfig {
        return Err(CoreError::InvalidPath(
            "ai-config 为资产源，不能从自身导入".into(),
        ));
    }
    let adapter = platform::for_scope_with_asset(plat, deploy_base, asset_root)?;
    if !adapter.supports(AssetKind::Rule) {
        return Err(CoreError::UnsupportedAsset {
            platform: plat,
            asset: AssetKind::Rule,
            hint: "该平台不支持 rules".into(),
        });
    }
    let rules_dir = adapter.rules_dir();
    let platform_path = find_rule_on_platform(&rules_dir, name)?;
    let source_scan = scan_source_for_scope(default_root, asset_root)?;
    if find_source_path(AssetKind::Rule, name, &source_scan).is_some() {
        return Err(CoreError::InvalidPath(format!("源中已存在 rule `{name}`")));
    }
    let dest = asset_root.join("rules").join(format!("{name}.mdc"));
    crate::paths::ensure_parent_dir(&dest)?;
    let material_src = crate::materialize::resolve_copy_source(&platform_path);
    if material_src.is_dir() {
        return Err(CoreError::InvalidPath(format!(
            "平台 rule `{name}` 为目录,无法导入"
        )));
    }
    fs::copy(material_src.as_std_path(), dest.as_std_path()).map_err(CoreError::Io)?;
    crate::materialize::adopt_existing_deploy(&dest, &platform_path)?;
    Ok(dest)
}

/// 将平台 agent 复制到 `asset_root/agents/`。
pub fn import_agent_from_platform(
    name: &str,
    plat: PlatformId,
    default_root: &Utf8Path,
    asset_root: &Utf8Path,
    deploy_base: &Utf8Path,
) -> Result<Utf8PathBuf, CoreError> {
    if plat == PlatformId::AiConfig {
        return Err(CoreError::InvalidPath(
            "ai-config 为资产源，不能从自身导入".into(),
        ));
    }
    let adapter = platform::for_scope_with_asset(plat, deploy_base, asset_root)?;
    if !adapter.supports(AssetKind::Agent) {
        return Err(CoreError::UnsupportedAsset {
            platform: plat,
            asset: AssetKind::Agent,
            hint: "该平台不支持 agents".into(),
        });
    }
    let agents_dir = adapter.agents_dir();
    let platform_path = find_agent_on_platform(&agents_dir, name)?;
    let source_scan = scan_source_for_scope(default_root, asset_root)?;
    if find_source_path(AssetKind::Agent, name, &source_scan).is_some() {
        return Err(CoreError::InvalidPath(format!("源中已存在 agent `{name}`")));
    }
    let dest = if platform_path.is_dir() {
        asset_root.join("agents").join(name)
    } else {
        let ext = platform_path
            .extension()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "md".to_string());
        asset_root.join("agents").join(format!("{name}.{ext}"))
    };
    let material_src = crate::materialize::resolve_copy_source(&platform_path);
    if material_src.is_dir() {
        crate::materialize::copy_tree(&material_src, &dest)?;
    } else {
        crate::paths::ensure_parent_dir(&dest)?;
        fs::copy(material_src.as_std_path(), dest.as_std_path()).map_err(CoreError::Io)?;
    }
    let link_src = crate::sync::link_src_for_create(AssetKind::Agent, &dest);
    crate::materialize::adopt_existing_deploy(&link_src, &platform_path)?;
    Ok(dest)
}

/// 将平台 command 文件复制到 `asset_root/commands/<name>.md`。
pub fn import_command_from_platform(
    name: &str,
    plat: PlatformId,
    default_root: &Utf8Path,
    asset_root: &Utf8Path,
    deploy_base: &Utf8Path,
) -> Result<Utf8PathBuf, CoreError> {
    if plat == PlatformId::AiConfig {
        return Err(CoreError::InvalidPath(
            "ai-config 为资产源，不能从自身导入".into(),
        ));
    }
    let adapter = platform::for_scope_with_asset(plat, deploy_base, asset_root)?;
    if !adapter.supports(AssetKind::Command) {
        return Err(CoreError::UnsupportedAsset {
            platform: plat,
            asset: AssetKind::Command,
            hint: "该平台不支持斜杠 commands".into(),
        });
    }
    let commands_dir = adapter.commands_dir();
    let platform_path = find_command_on_platform(&commands_dir, name)?;
    let source_scan = scan_source_for_scope(default_root, asset_root)?;
    if find_source_path(AssetKind::Command, name, &source_scan).is_some() {
        return Err(CoreError::InvalidPath(format!(
            "源中已存在 command `{name}`"
        )));
    }
    let dest = asset_root.join("commands").join(format!("{name}.md"));
    crate::paths::ensure_parent_dir(&dest)?;
    let material_src = crate::materialize::resolve_copy_source(&platform_path);
    fs::copy(material_src.as_std_path(), dest.as_std_path()).map_err(CoreError::Io)?;
    crate::materialize::adopt_existing_deploy(&dest, &platform_path)?;
    Ok(dest)
}

/// 将平台 MCP server 配置写入源 `mcp.json`。
pub fn import_mcp_from_platform(
    name: &str,
    plat: PlatformId,
    default_root: &Utf8Path,
    asset_root: &Utf8Path,
    deploy_base: &Utf8Path,
) -> Result<Utf8PathBuf, CoreError> {
    if plat == PlatformId::AiConfig {
        return Err(CoreError::InvalidPath(
            "ai-config 为资产源，不能从自身导入".into(),
        ));
    }
    let adapter = platform::for_scope_with_asset(plat, deploy_base, asset_root)?;
    let config = read_platform_mcp_server_config(adapter.as_ref(), name)?;
    let source_scan = scan_source_for_scope(default_root, asset_root)?;
    if find_source_path(AssetKind::Mcp, name, &source_scan).is_some() {
        return Err(CoreError::InvalidPath(format!(
            "源中已存在 MCP server `{name}`"
        )));
    }
    mcp_json::ensure_mcp_json(asset_root)?;
    mcp_json::upsert_server_in_document(asset_root, name, config)?;
    Ok(mcp_json::mcp_json_path(asset_root))
}

/// 将平台 hook 脚本与 `hooks.json` 绑定导入到 `asset_root`。
pub fn import_hook_from_platform(
    script_filename: &str,
    plat: PlatformId,
    default_root: &Utf8Path,
    asset_root: &Utf8Path,
    deploy_base: &Utf8Path,
) -> Result<Utf8PathBuf, CoreError> {
    if plat == PlatformId::AiConfig {
        return Err(CoreError::InvalidPath(
            "ai-config 为资产源，不能从自身导入".into(),
        ));
    }
    let adapter = platform::for_scope_with_asset(plat, deploy_base, asset_root)?;
    if !adapter.supports(AssetKind::Hook) {
        return Err(CoreError::UnsupportedAsset {
            platform: plat,
            asset: AssetKind::Hook,
            hint: "该平台不支持 hooks".into(),
        });
    }
    if !crate::hook_adapter::platform_script_path(deploy_base, plat, script_filename).is_file() {
        return Err(CoreError::AssetNotFound {
            kind: AssetKind::Hook,
            name: script_filename.into(),
            hint: format!(
                "平台 `{}` 上找不到 hook `{script_filename}`",
                plat_label(plat)
            ),
        });
    }
    let source_scan = scan_source_for_scope(default_root, asset_root)?;
    if find_source_path(AssetKind::Hook, script_filename, &source_scan).is_some() {
        return Err(CoreError::InvalidPath(format!(
            "源中已存在 hook `{script_filename}`，请先删除或重命名"
        )));
    }
    crate::hook_adapter::import_to_source(asset_root, deploy_base, script_filename, plat)
}

/// 将已纳管资产从 `from_root` 复制到 `to_root`（跨项目粘贴）。
pub fn copy_asset_to_asset_root(
    kind: AssetKind,
    name: &str,
    from_default: &Utf8Path,
    from_root: &Utf8Path,
    to_default: &Utf8Path,
    to_root: &Utf8Path,
) -> Result<Utf8PathBuf, CoreError> {
    if from_root == to_root {
        return Err(CoreError::InvalidPath(
            "源项目与目标项目相同，无法复制".into(),
        ));
    }
    let from_scan = scan_source_for_scope(from_default, from_root)?;
    let Some(src) = find_source_path(kind, name, &from_scan) else {
        return Err(CoreError::AssetNotFound {
            kind,
            name: name.into(),
            hint: format!("源项目无 `{name}`"),
        });
    };
    let to_scan = scan_source_for_scope(to_default, to_root)?;
    if find_source_path(kind, name, &to_scan).is_some() {
        return Err(CoreError::InvalidPath(format!(
            "目标项目已存在 `{name}`，请先删除或重命名"
        )));
    }

    let dest = match kind {
        AssetKind::Skill => {
            let dest_dir = to_root.join("skills").join(name);
            let link_src = crate::sync::link_src_for_create(kind, &src);
            let material = crate::materialize::resolve_copy_source(&link_src);
            crate::materialize::copy_tree(&material, &dest_dir)?;
            dest_dir
        }
        AssetKind::Rule => {
            let dest = to_root.join("rules").join(format!("{name}.mdc"));
            crate::paths::ensure_parent_dir(&dest)?;
            let material = crate::materialize::resolve_copy_source(&src);
            std::fs::copy(material.as_std_path(), dest.as_std_path()).map_err(CoreError::Io)?;
            dest
        }
        AssetKind::Agent => {
            let dest = if src.is_dir() {
                to_root.join("agents").join(name)
            } else {
                let ext = src
                    .extension()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "md".to_string());
                to_root.join("agents").join(format!("{name}.{ext}"))
            };
            let link_src = crate::sync::link_src_for_create(kind, &src);
            let material = crate::materialize::resolve_copy_source(&link_src);
            if material.is_dir() {
                crate::materialize::copy_tree(&material, &dest)?;
            } else {
                crate::paths::ensure_parent_dir(&dest)?;
                std::fs::copy(material.as_std_path(), dest.as_std_path()).map_err(CoreError::Io)?;
            }
            dest
        }
        AssetKind::Command => {
            let dest = to_root.join("commands").join(format!("{name}.md"));
            crate::paths::ensure_parent_dir(&dest)?;
            let material = crate::materialize::resolve_copy_source(&src);
            std::fs::copy(material.as_std_path(), dest.as_std_path()).map_err(CoreError::Io)?;
            dest
        }
        AssetKind::Mcp => {
            let asset_root = src.parent().unwrap_or(&src);
            let config = mcp_json::get_server_config(asset_root, name)?.ok_or_else(|| {
                CoreError::AssetNotFound {
                    kind,
                    name: name.into(),
                    hint: format!("MCP `{name}` 配置缺失"),
                }
            })?;
            mcp_json::ensure_mcp_json(to_root)?;
            mcp_json::upsert_server_in_document(to_root, name, config)?;
            mcp_json::mcp_json_path(to_root)
        }
        AssetKind::Hook => {
            let script_filename = src
                .file_name()
                .map(str::to_string)
                .unwrap_or_else(|| name.to_string());
            let dest_script = to_root.join("hooks").join(&script_filename);
            if dest_script.is_file() {
                return Err(CoreError::InvalidPath(format!(
                    "目标已存在 hook `{script_filename}`"
                )));
            }
            crate::paths::ensure_parent_dir(&dest_script)?;
            std::fs::copy(src.as_std_path(), dest_script.as_std_path()).map_err(CoreError::Io)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = std::fs::metadata(dest_script.as_std_path()) {
                    let mut perms = meta.permissions();
                    perms.set_mode(0o755);
                    let _ = std::fs::set_permissions(dest_script.as_std_path(), perms);
                }
            }
            let spec = crate::hook::load_spec(from_root, &script_filename)?;
            let from_deploy = crate::hook::deploy_base_for_asset_root(from_root);
            let bindings = if spec.bindings.is_empty() {
                crate::hook::collect_bindings_for_script(from_root, &from_deploy, &script_filename)?
            } else {
                spec.bindings
            };
            crate::hook::merge_bindings_into_manifest(to_root, &script_filename, &bindings)?;
            dest_script
        }
    };
    Ok(dest)
}

pub fn read_platform_mcp_server_config(
    adapter: &dyn PlatformAdapter,
    name: &str,
) -> Result<serde_json::Value, CoreError> {
    if adapter.id() == PlatformId::Hermes {
        return crate::hermes_config::get_mcp_server_config(&adapter.mcp_deploy_path(), name);
    }
    mcp_json::get_server_config_from_deploy_file(&adapter.mcp_deploy_path(), name)
}

fn find_command_on_platform(commands_dir: &Utf8Path, name: &str) -> Result<Utf8PathBuf, CoreError> {
    let p = commands_dir.join(format!("{name}.md"));
    if p.is_file() {
        return Ok(p);
    }
    Err(CoreError::AssetNotFound {
        kind: AssetKind::Command,
        name: name.into(),
        hint: format!("平台 commands 目录无 `{name}.md`"),
    })
}

fn find_rule_on_platform(rules_dir: &Utf8Path, name: &str) -> Result<Utf8PathBuf, CoreError> {
    for ext in [".mdc", ".md"] {
        let p = rules_dir.join(format!("{name}{ext}"));
        if p.is_file() {
            return Ok(p);
        }
    }
    Err(CoreError::AssetNotFound {
        kind: AssetKind::Rule,
        name: name.into(),
        hint: format!("平台 rules 目录无 `{name}`"),
    })
}

fn find_agent_on_platform(agents_dir: &Utf8Path, name: &str) -> Result<Utf8PathBuf, CoreError> {
    let dir = agents_dir.join(name);
    if dir.is_dir() {
        return Ok(dir);
    }
    for ext in ["md", "yaml", "json"] {
        let p = agents_dir.join(format!("{name}.{ext}"));
        if p.is_file() {
            return Ok(p);
        }
    }
    Err(CoreError::AssetNotFound {
        kind: AssetKind::Agent,
        name: name.into(),
        hint: format!("平台 agents 目录无 `{name}`"),
    })
}

fn plat_label(p: PlatformId) -> &'static str {
    match p {
        PlatformId::AiConfig => "aiconfig",
        PlatformId::Cursor => "cursor",
        PlatformId::Codex => "codex",
        PlatformId::Claude => "claude",
        PlatformId::Hermes => "hermes",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn touch(path: &Utf8Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent.as_std_path()).unwrap();
        }
        fs::write(path.as_std_path(), content).unwrap();
    }

    #[test]
    fn scan_platform_skills_finds_entries() {
        let plat_root = TempDir::new().unwrap();
        let home = Utf8Path::from_path(plat_root.path()).unwrap();
        let skills = home.join(".claude/skills");
        touch(
            &skills.join("antd").join("SKILL.md"),
            "---\ndescription: Ant Design skill\n---\n# Ant",
        );
        touch(&skills.join("empty-dir").join("README.md"), "no skill md");

        struct FakeAdapter {
            skills: Utf8PathBuf,
        }
        impl PlatformAdapter for FakeAdapter {
            fn id(&self) -> PlatformId {
                PlatformId::Claude
            }
            fn skills_dir(&self) -> Utf8PathBuf {
                self.skills.clone()
            }
            fn rules_dir(&self) -> Utf8PathBuf {
                self.skills.parent().unwrap().join("rules")
            }
            fn agents_dir(&self) -> Utf8PathBuf {
                self.skills.parent().unwrap().join("agents")
            }
            fn commands_dir(&self) -> Utf8PathBuf {
                self.skills.parent().unwrap().join("commands")
            }
            fn mcp_json_path(&self) -> Utf8PathBuf {
                self.skills.parent().unwrap().join("mcp.json")
            }
        }
        let adapter = FakeAdapter { skills };
        let entries = scan_platform_skills(&adapter).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "antd");
        assert_eq!(entries[0].2, "Ant Design skill");
    }

    #[test]
    fn scan_platform_skills_follows_symlink_dirs() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(tmp.path()).unwrap();
        let target = root.join("real-skill");
        touch(
            &target.join("SKILL.md"),
            "---\ndescription: via symlink\n---\n# Symlink",
        );
        let skills = root.join(".claude/skills");
        fs::create_dir_all(skills.as_std_path()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(&target, skills.join("linked-skill").as_std_path()).unwrap();
        }
        #[cfg(not(unix))]
        {
            return;
        }

        struct FakeAdapter {
            skills: Utf8PathBuf,
        }
        impl PlatformAdapter for FakeAdapter {
            fn id(&self) -> PlatformId {
                PlatformId::Claude
            }
            fn skills_dir(&self) -> Utf8PathBuf {
                self.skills.clone()
            }
            fn rules_dir(&self) -> Utf8PathBuf {
                self.skills.parent().unwrap().join("rules")
            }
            fn agents_dir(&self) -> Utf8PathBuf {
                self.skills.parent().unwrap().join("agents")
            }
            fn commands_dir(&self) -> Utf8PathBuf {
                self.skills.parent().unwrap().join("commands")
            }
            fn mcp_json_path(&self) -> Utf8PathBuf {
                self.skills.parent().unwrap().join("mcp.json")
            }
        }
        let entries = scan_platform_skills(&FakeAdapter { skills }).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "linked-skill");
    }

    #[test]
    fn scan_platform_agents_ignores_deploy_marker_files() {
        let tmp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(tmp.path()).unwrap();
        let agents = home.join(".cursor/agents");
        touch(
            &agents.join("backend-java-dev.md"),
            "---\ndescription: backend java\n---\n# backend",
        );
        touch(
            &agents.join("backend-java-dev.md.ai-config-deploy.json"),
            "{\"version\":1,\"source\":\"/tmp/x\"}",
        );

        struct FakeAdapter {
            agents: Utf8PathBuf,
        }
        impl PlatformAdapter for FakeAdapter {
            fn id(&self) -> PlatformId {
                PlatformId::Cursor
            }
            fn skills_dir(&self) -> Utf8PathBuf {
                self.agents.parent().unwrap().join("skills")
            }
            fn rules_dir(&self) -> Utf8PathBuf {
                self.agents.parent().unwrap().join("rules")
            }
            fn agents_dir(&self) -> Utf8PathBuf {
                self.agents.clone()
            }
            fn commands_dir(&self) -> Utf8PathBuf {
                self.agents.parent().unwrap().join("commands")
            }
            fn mcp_json_path(&self) -> Utf8PathBuf {
                self.agents.parent().unwrap().join("mcp.json")
            }
        }

        let entries = scan_platform_agents(&FakeAdapter { agents }).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "backend-java-dev");
    }

    #[test]
    fn import_skill_copies_to_source() {
        let tmp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(tmp.path()).unwrap();
        let _home_guard = crate::test_env::EnvGuard::set("HOME", tmp.path().to_str().unwrap());

        let claude_skills = home.join(".claude/skills/import-me");
        touch(
            &claude_skills.join("SKILL.md"),
            "# Import Me\n\ndescription here",
        );
        touch(&claude_skills.join("scripts").join("run.sh"), "#!/bin/sh\n");

        let asset_root = home.join(".ai-config");
        fs::create_dir_all(asset_root.join("skills")).unwrap();
        mcp_json::ensure_mcp_json(&asset_root).unwrap();

        let dest = import_skill_from_platform(
            "import-me",
            PlatformId::Claude,
            &asset_root,
            &asset_root,
            home,
        )
        .unwrap();
        assert!(dest.join("SKILL.md").is_file());
        assert!(dest.join("scripts/run.sh").is_file());
        assert!(
            !claude_skills.join(".ai-config-deploy.json").exists(),
            "导入纳管不应生成 deploy marker"
        );

        let source_scan = scan_source_for_scope(&asset_root, &asset_root).unwrap();
        let states = compute_entry_states(
            PlatformId::AiConfig,
            AssetKind::Skill,
            "import-me",
            &claude_skills,
            home,
            &asset_root,
            &source_scan,
        );
        assert_eq!(states.get(&PlatformId::AiConfig), Some(&LinkState::Linked));
        assert_eq!(
            states.get(&PlatformId::Claude),
            Some(&LinkState::Linked),
            "从 Claude 导入后在 ai-config 视图应显示 Claude 已同步"
        );
    }

    #[test]
    fn platform_mirror_detects_cross_ide_skill_copy() {
        let tmp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(tmp.path()).unwrap();
        let _home_guard = crate::test_env::EnvGuard::set("HOME", tmp.path().to_str().unwrap());

        let claude_skill = home.join(".claude/skills/find-skills");
        fs::create_dir_all(&claude_skill).unwrap();
        fs::write(claude_skill.join("SKILL.md"), "# Find\n").unwrap();

        let asset_root = home.join(".ai-config");
        fs::create_dir_all(asset_root.join("skills")).unwrap();
        mcp_json::ensure_mcp_json(&asset_root).unwrap();

        let scope = crate::asset_ops::ScopeRoots {
            default_root: &asset_root,
            asset_root: &asset_root,
            deploy_base: home,
        };
        crate::asset_ops::deploy_from_platform(
            &scope,
            AssetKind::Skill,
            "find-skills",
            PlatformId::Claude,
            PlatformId::Cursor,
        )
        .unwrap();

        let source_scan = scan_source_for_scope(&asset_root, &asset_root).unwrap();
        let states = compute_entry_states(
            PlatformId::Claude,
            AssetKind::Skill,
            "find-skills",
            &claude_skill,
            home,
            &asset_root,
            &source_scan,
        );
        assert_eq!(states.get(&PlatformId::Claude), Some(&LinkState::Synced));
        assert_eq!(states.get(&PlatformId::Cursor), Some(&LinkState::Synced));
    }

    #[test]
    fn external_platform_copy_is_synced_not_retractable() {
        let tmp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(tmp.path()).unwrap();
        let _home_guard = crate::test_env::EnvGuard::set("HOME", tmp.path().to_str().unwrap());

        let claude_skill = home.join(".claude/skills/find-skills");
        fs::create_dir_all(&claude_skill).unwrap();
        fs::write(claude_skill.join("SKILL.md"), "# Find\n").unwrap();

        let hermes_skill = home.join(".hermes/skills/find-skills");
        fs::create_dir_all(&hermes_skill).unwrap();
        fs::write(hermes_skill.join("SKILL.md"), "# Find\n").unwrap();

        let asset_root = home.join(".ai-config");
        fs::create_dir_all(asset_root.join("skills")).unwrap();
        mcp_json::ensure_mcp_json(&asset_root).unwrap();

        let source_scan = scan_source_for_scope(&asset_root, &asset_root).unwrap();
        let states = compute_entry_states(
            PlatformId::Claude,
            AssetKind::Skill,
            "find-skills",
            &claude_skill,
            home,
            &asset_root,
            &source_scan,
        );
        assert_eq!(states.get(&PlatformId::Hermes), Some(&LinkState::Synced));
    }

    #[test]
    fn browse_hermes_shows_synced_for_external_skill() {
        let tmp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(tmp.path()).unwrap();
        let _home_guard = crate::test_env::EnvGuard::set("HOME", tmp.path().to_str().unwrap());

        let hermes_skill = home.join(".hermes/skills/external-one");
        fs::create_dir_all(&hermes_skill).unwrap();
        fs::write(hermes_skill.join("SKILL.md"), "# ext\n").unwrap();

        let asset_root = home.join(".ai-config");
        fs::create_dir_all(asset_root.join("skills")).unwrap();
        mcp_json::ensure_mcp_json(&asset_root).unwrap();

        let source_scan = scan_source_for_scope(&asset_root, &asset_root).unwrap();
        let states = compute_entry_states(
            PlatformId::Hermes,
            AssetKind::Skill,
            "external-one",
            &hermes_skill,
            home,
            &asset_root,
            &source_scan,
        );
        assert_eq!(states.get(&PlatformId::Hermes), Some(&LinkState::Synced));
    }

    #[test]
    fn aiconfig_unlinked_when_not_in_source() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(tmp.path()).unwrap();
        let states = compute_entry_states(
            PlatformId::Claude,
            AssetKind::Skill,
            "x",
            &root.join("platform-skill"),
            root,
            root,
            &ScanResult::default(),
        );
        assert_eq!(
            states.get(&PlatformId::AiConfig),
            Some(&LinkState::Unlinked)
        );
        assert_eq!(states.get(&PlatformId::Claude), Some(&LinkState::Missing));
    }

    #[test]
    fn aiconfig_linked_when_exists_in_source_on_ide_view() {
        let tmp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(tmp.path()).unwrap();
        let _home_guard = crate::test_env::EnvGuard::set("HOME", tmp.path().to_str().unwrap());

        let cursor_skill = home.join(".cursor/skills/demo");
        fs::create_dir_all(&cursor_skill).unwrap();
        fs::write(cursor_skill.join("SKILL.md"), "# Demo\n").unwrap();

        let asset_root = home.join(".ai-config");
        fs::create_dir_all(asset_root.join("skills/demo")).unwrap();
        fs::write(asset_root.join("skills/demo/SKILL.md"), "# Demo\n").unwrap();
        mcp_json::ensure_mcp_json(&asset_root).unwrap();

        let source_scan = scan_source_for_scope(&asset_root, &asset_root).unwrap();
        let states = compute_entry_states(
            PlatformId::Cursor,
            AssetKind::Skill,
            "demo",
            &cursor_skill,
            home,
            &asset_root,
            &source_scan,
        );
        assert_eq!(states.get(&PlatformId::AiConfig), Some(&LinkState::Linked));
    }

    #[test]
    fn browse_cursor_mcp_without_source_shows_synced() {
        let tmp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(tmp.path()).unwrap();
        let _home_guard = crate::test_env::EnvGuard::set("HOME", tmp.path().to_str().unwrap());

        let cursor_mcp = home.join(".cursor/mcp.json");
        crate::paths::ensure_parent_dir(&cursor_mcp).unwrap();
        fs::write(
            &cursor_mcp,
            r#"{"mcpServers":{"demo":{"command":"npx","args":["-y","demo"]}}}"#,
        )
        .unwrap();

        let asset_root = home.join(".ai-config");
        mcp_json::ensure_mcp_json(&asset_root).unwrap();

        let source_scan = scan_source_for_scope(&asset_root, &asset_root).unwrap();
        let states = compute_entry_states(
            PlatformId::Cursor,
            AssetKind::Mcp,
            "demo",
            &cursor_mcp,
            home,
            &asset_root,
            &source_scan,
        );
        assert_eq!(
            states.get(&PlatformId::Cursor),
            Some(&LinkState::Synced),
            "仅存在于 Cursor 的 MCP 在 Cursor 视图应显示已同步"
        );
        assert_eq!(
            states.get(&PlatformId::AiConfig),
            Some(&LinkState::Missing),
            "仅存在于 Cursor 的 MCP 在 Cursor 视图下 ai-config icon 应显示源缺失"
        );
    }

    #[test]
    fn mirror_mcp_shows_synced_on_target_after_cross_platform_copy() {
        let tmp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(tmp.path()).unwrap();
        let _home_guard = crate::test_env::EnvGuard::set("HOME", tmp.path().to_str().unwrap());

        let cursor_mcp = home.join(".cursor/mcp.json");
        crate::paths::ensure_parent_dir(&cursor_mcp).unwrap();
        fs::write(
            &cursor_mcp,
            r#"{"mcpServers":{"Chrome DevTools MCP":{"command":"npx","args":["-y","chrome-devtools-mcp"]}}}"#,
        )
        .unwrap();

        let asset_root = home.join(".ai-config");
        mcp_json::ensure_mcp_json(&asset_root).unwrap();

        let scope = crate::asset_ops::ScopeRoots {
            default_root: &asset_root,
            asset_root: &asset_root,
            deploy_base: home,
        };
        crate::asset_ops::deploy_from_platform(
            &scope,
            AssetKind::Mcp,
            "Chrome DevTools MCP",
            PlatformId::Cursor,
            PlatformId::Claude,
        )
        .unwrap();

        let source_scan = scan_source_for_scope(&asset_root, &asset_root).unwrap();
        let states = compute_entry_states(
            PlatformId::Cursor,
            AssetKind::Mcp,
            "Chrome DevTools MCP",
            &cursor_mcp,
            home,
            &asset_root,
            &source_scan,
        );
        assert_eq!(
            states.get(&PlatformId::Claude),
            Some(&LinkState::Synced),
            "Cursor→Claude 拷贝后，在 Cursor 视图 Claude 图标应显示已同步"
        );
        assert_eq!(
            states.get(&PlatformId::Codex),
            Some(&LinkState::Missing),
            "未拷贝的平台仍为 missing"
        );
    }

    #[test]
    fn mirror_mcp_missing_when_target_not_deployed() {
        let tmp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(tmp.path()).unwrap();
        let _home_guard = crate::test_env::EnvGuard::set("HOME", tmp.path().to_str().unwrap());

        let cursor_mcp = home.join(".cursor/mcp.json");
        crate::paths::ensure_parent_dir(&cursor_mcp).unwrap();
        fs::write(&cursor_mcp, r#"{"mcpServers":{"demo":{"command":"npx"}}}"#).unwrap();

        let asset_root = home.join(".ai-config");
        mcp_json::ensure_mcp_json(&asset_root).unwrap();

        let source_scan = scan_source_for_scope(&asset_root, &asset_root).unwrap();
        let states = compute_entry_states(
            PlatformId::Cursor,
            AssetKind::Mcp,
            "demo",
            &cursor_mcp,
            home,
            &asset_root,
            &source_scan,
        );
        assert_eq!(states.get(&PlatformId::Codex), Some(&LinkState::Missing));
    }

    #[test]
    fn compute_entry_states_mcp_with_source_shows_linked_on_deployed_platform() {
        let tmp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(tmp.path()).unwrap();
        let _home_guard = crate::test_env::EnvGuard::set("HOME", tmp.path().to_str().unwrap());

        let asset_root = home.join(".ai-config");
        mcp_json::ensure_mcp_json(&asset_root).unwrap();
        let cfg = serde_json::json!({ "command": "uvx", "args": ["demo"] });
        mcp_json::upsert_server_in_document(&asset_root, "svc", cfg.clone()).unwrap();

        let cursor_mcp = home.join(".cursor/mcp.json");
        mcp_json::upsert_server_on_platform(PlatformId::Cursor, &cursor_mcp, "svc", &cfg, None)
            .unwrap();

        let source_scan = scan_source_for_scope(&asset_root, &asset_root).unwrap();
        let mcp_path = mcp_json::mcp_json_path(&asset_root);
        let states = compute_entry_states(
            PlatformId::AiConfig,
            AssetKind::Mcp,
            "svc",
            &mcp_path,
            home,
            &asset_root,
            &source_scan,
        );
        assert_eq!(states.get(&PlatformId::AiConfig), Some(&LinkState::Linked));
        assert_eq!(states.get(&PlatformId::Cursor), Some(&LinkState::Linked));
        assert_eq!(states.get(&PlatformId::Codex), Some(&LinkState::Unlinked));
    }

    #[test]
    fn copy_skill_to_other_project_asset_root() {
        let global = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        let global_root = Utf8Path::from_path(global.path()).unwrap();
        let project_root = Utf8Path::from_path(project.path()).unwrap();
        touch(
            &global_root
                .join("skills")
                .join("git-sync-github")
                .join("SKILL.md"),
            "---\ndescription: GitHub sync\n---\n# Git Sync",
        );

        let dest = copy_asset_to_asset_root(
            AssetKind::Skill,
            "git-sync-github",
            global_root,
            global_root,
            project_root,
            project_root,
        )
        .unwrap();

        assert_eq!(dest, project_root.join("skills").join("git-sync-github"));
        assert!(dest.join("SKILL.md").is_file());
        assert_eq!(
            fs::read_to_string(dest.join("SKILL.md").as_std_path()).unwrap(),
            "---\ndescription: GitHub sync\n---\n# Git Sync"
        );
    }

    #[test]
    fn copy_hook_pulls_bindings_from_source_platform_when_manifest_empty() {
        let global = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        let global_root = Utf8Path::from_path(global.path()).unwrap();
        let project_root = Utf8Path::from_path(project.path()).unwrap();
        let global_asset = global_root.join(".ai-config");
        fs::create_dir_all(global_asset.join("hooks")).unwrap();
        fs::write(
            global_asset.join("hooks.json"),
            r#"{"version":1,"hooks":{}}"#,
        )
        .unwrap();
        fs::write(
            global_asset.join("hooks/speak-lifecycle.py"),
            "#!/usr/bin/env python3\n\"\"\"TTS\"\"\"\n",
        )
        .unwrap();
        fs::create_dir_all(global_root.join(".cursor/hooks")).unwrap();
        fs::write(
            global_root.join(".cursor/hooks.json"),
            r#"{"version":1,"hooks":{"sessionStart":[{"command":".cursor/hooks/speak-lifecycle.py sessionStart"}]}}"#,
        )
        .unwrap();
        fs::write(
            global_root.join(".cursor/hooks/speak-lifecycle.py"),
            "#!/usr/bin/env python3\n",
        )
        .unwrap();

        let project_asset = project_root.join(".ai-config");
        fs::create_dir_all(&project_asset).unwrap();

        let dest = copy_asset_to_asset_root(
            AssetKind::Hook,
            "speak-lifecycle.py",
            global_root,
            &global_asset,
            project_root,
            &project_asset,
        )
        .unwrap();

        assert_eq!(dest, project_asset.join("hooks/speak-lifecycle.py"));
        let manifest = fs::read_to_string(project_asset.join("hooks.json").as_std_path()).unwrap();
        assert!(manifest.contains("sessionStart"));
        assert!(manifest.contains("./hooks/speak-lifecycle.py sessionStart"));
    }

    #[test]
    fn copy_hook_to_other_project_asset_root() {
        let global = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        let global_root = Utf8Path::from_path(global.path()).unwrap();
        let project_root = Utf8Path::from_path(project.path()).unwrap();
        touch(
            &global_root.join("hooks.json"),
            r#"{"version":1,"hooks":{"sessionStart":[{"command":"./hooks/speak-lifecycle.py"}]}}"#,
        );
        touch(
            &global_root.join("hooks/speak-lifecycle.py"),
            "#!/usr/bin/env python3\n\"\"\"TTS\"\"\"\n",
        );

        let dest = copy_asset_to_asset_root(
            AssetKind::Hook,
            "speak-lifecycle.py",
            global_root,
            global_root,
            project_root,
            project_root,
        )
        .unwrap();

        assert_eq!(dest, project_root.join("hooks/speak-lifecycle.py"));
        assert!(dest.is_file());
        assert!(project_root.join("hooks.json").is_file());
        let manifest = fs::read_to_string(project_root.join("hooks.json").as_std_path()).unwrap();
        assert!(manifest.contains("speak-lifecycle.py"));
    }

    #[test]
    fn copy_asset_rejects_same_project() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(tmp.path()).unwrap();
        touch(&root.join("skills").join("x").join("SKILL.md"), "# x");

        let err =
            copy_asset_to_asset_root(AssetKind::Skill, "x", root, root, root, root).unwrap_err();
        assert!(err.to_string().contains("相同"));
    }

    #[test]
    fn copy_asset_rejects_existing_target_name() {
        let global = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        let global_root = Utf8Path::from_path(global.path()).unwrap();
        let project_root = Utf8Path::from_path(project.path()).unwrap();
        touch(
            &global_root.join("skills").join("dup").join("SKILL.md"),
            "# global",
        );
        touch(
            &project_root.join("skills").join("dup").join("SKILL.md"),
            "# project",
        );

        let err = copy_asset_to_asset_root(
            AssetKind::Skill,
            "dup",
            global_root,
            global_root,
            project_root,
            project_root,
        )
        .unwrap_err();
        assert!(err.to_string().contains("已存在"));
    }

    #[test]
    fn import_hook_from_cursor_project_scope() {
        let tmp = TempDir::new().unwrap();
        let repo = Utf8Path::from_path(tmp.path()).unwrap();
        let asset_root = repo.join(".ai-config");
        fs::create_dir_all(asset_root.join("hooks")).unwrap();
        mcp_json::ensure_mcp_json(&asset_root).unwrap();

        let hooks_json = repo.join(".cursor/hooks.json");
        touch(
            &hooks_json,
            r#"{"version":1,"hooks":{"afterAgentResponse":[{"command":"./hooks/lifecycle-tts.sh","matcher":"*"}]}}"#,
        );
        touch(
            &repo.join(".cursor/hooks/lifecycle-tts.sh"),
            "#!/bin/sh\n\"\"\"生命周期 TTS\"\"\"\n",
        );

        let dest = import_hook_from_platform(
            "lifecycle-tts.sh",
            PlatformId::Cursor,
            &asset_root,
            &asset_root,
            repo,
        )
        .unwrap();

        assert_eq!(dest, asset_root.join("hooks/lifecycle-tts.sh"));
        assert!(dest.is_file());
        assert!(asset_root.join("hooks.json").is_file());

        let source_scan = scan_source_for_scope(&asset_root, &asset_root).unwrap();
        let cursor_path = repo.join(".cursor/hooks/lifecycle-tts.sh");
        let states = compute_entry_states(
            PlatformId::Cursor,
            AssetKind::Hook,
            "lifecycle-tts.sh",
            &cursor_path,
            repo,
            &asset_root,
            &source_scan,
        );
        assert_eq!(states.get(&PlatformId::AiConfig), Some(&LinkState::Linked));
        assert_eq!(states.get(&PlatformId::Cursor), Some(&LinkState::Synced));
    }

    #[test]
    fn scan_platform_hooks_cursor_path_with_args() {
        let tmp = TempDir::new().unwrap();
        let repo = Utf8Path::from_path(tmp.path()).unwrap();
        touch(
            &repo.join(".cursor/hooks.json"),
            r#"{"version":1,"hooks":{"sessionStart":[{"command":".cursor/hooks/speak-lifecycle.py sessionStart"}]}}"#,
        );
        touch(
            &repo.join(".cursor/hooks/speak-lifecycle.py"),
            "#!/usr/bin/env python3\n\"\"\"生命周期 TTS\"\"\"\n",
        );

        struct CursorAdapter {
            base: Utf8PathBuf,
        }
        impl PlatformAdapter for CursorAdapter {
            fn id(&self) -> PlatformId {
                PlatformId::Cursor
            }
            fn skills_dir(&self) -> Utf8PathBuf {
                self.base.join(".cursor/skills")
            }
            fn rules_dir(&self) -> Utf8PathBuf {
                self.base.join(".cursor/rules")
            }
            fn agents_dir(&self) -> Utf8PathBuf {
                self.base.join(".cursor/agents")
            }
            fn commands_dir(&self) -> Utf8PathBuf {
                self.base.join(".cursor/commands")
            }
            fn mcp_json_path(&self) -> Utf8PathBuf {
                self.base.join(".cursor/mcp.json")
            }
        }

        let entries = scan_platform_hooks(&CursorAdapter {
            base: repo.to_path_buf(),
        })
        .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "speak-lifecycle.py");
        assert_eq!(entries[0].2, "生命周期 TTS");
    }
}
