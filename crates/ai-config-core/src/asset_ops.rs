//! 单条资产 CRUD + deploy/retract（GUI / 未来 CLI 共用）。

use std::fs;

use camino::{Utf8Path, Utf8PathBuf};
use schemars::JsonSchema;
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
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct AssetFileDetail {
    pub name: String,
    pub description: String,
    pub content: String,
    pub source_path: String,
    pub parent_path: String,
}

/// 从已安装平台硬拷贝到另一平台（无 ai-config 源时，平台视图跨 IDE 同步）。
pub fn deploy_from_platform(
    scope: &ScopeRoots<'_>,
    kind: AssetKind,
    name: &str,
    from_plat: PlatformId,
    to_plat: PlatformId,
) -> Result<String, CoreError> {
    ensure_deploy_target(from_plat)?;
    ensure_deploy_target(to_plat)?;
    if from_plat == to_plat {
        return Err(CoreError::InvalidPath("来源平台与目标平台不能相同".into()));
    }
    ensure_platform_supports(scope, from_plat, kind)?;
    ensure_platform_supports(scope, to_plat, kind)?;
    if kind == AssetKind::Mcp {
        return Err(CoreError::InvalidPath(
            "MCP 请使用导入到源后再下发，或单独编辑 mcp.json".into(),
        ));
    }

    let from_adapter =
        platform::for_scope_with_asset(from_plat, scope.deploy_base, scope.asset_root)?;
    let to_adapter = platform::for_scope_with_asset(to_plat, scope.deploy_base, scope.asset_root)?;
    let (from_src, dest) =
        platform_asset_paths(from_adapter.as_ref(), to_adapter.as_ref(), kind, name)?;

    if !from_src.exists() {
        return Err(CoreError::AssetNotFound {
            kind,
            name: name.into(),
            hint: format!(
                "平台 `{}` 上找不到 {kind:?} `{name}`",
                platform::platform_label(from_plat)
            ),
        });
    }

    materialize::deploy(&from_src, &dest)?;
    if to_plat == PlatformId::Hermes && kind == AssetKind::Skill {
        let skills_parent = dest.parent().unwrap_or(&dest).to_path_buf();
        hermes_config::after_skill_deploy(&skills_parent)?;
    }
    Ok(format!(
        "{kind:?} `{name}`: {} → {} OK ({dest})",
        platform::platform_label(from_plat),
        platform::platform_label(to_plat),
    ))
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

/// 从平台收回（含 ai-config 平台目录；不牵动其它平台副本）。
pub fn retract(
    scope: &ScopeRoots<'_>,
    kind: AssetKind,
    name: &str,
    plat: PlatformId,
) -> Result<String, CoreError> {
    if plat == PlatformId::AiConfig {
        return retract_aiconfig_platform(scope, kind, name);
    }
    ensure_deploy_target(plat)?;
    ensure_platform_supports(scope, plat, kind)?;
    match kind {
        AssetKind::Mcp => retract_mcp(scope, name, plat),
        _ => {
            let dest = resolve_platform_retract_dest(scope, plat, kind, name)?;
            remove_native_platform_copy(&dest)?;
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
                parent_path: skill_md.parent().map(|p| p.to_string()).unwrap_or_default(),
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

/// 仅从 ai-config 平台目录收回（与其它 IDE 平台 `retract` 语义一致，不牵动其它平台副本）。
///
/// 保留别名供 CLI / 旧命令；GUI 平台 icon 走 `retract(..., AiConfig)`。
pub fn retract_source(
    scope: &ScopeRoots<'_>,
    kind: AssetKind,
    name: &str,
) -> Result<String, CoreError> {
    retract(scope, kind, name, PlatformId::AiConfig)
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
    remove_source_entry(scope, kind, name)
}

fn retract_aiconfig_platform(
    scope: &ScopeRoots<'_>,
    kind: AssetKind,
    name: &str,
) -> Result<String, CoreError> {
    let label = platform::platform_label(PlatformId::AiConfig);
    match kind {
        AssetKind::Mcp => {
            let mcp_path = asset_scope::locate_mcp_json(scope.default_root, scope.asset_root)?;
            let doc_root = mcp_asset_root(&mcp_path)?;
            mcp_json::remove_server_from_document(doc_root, name)?;
            Ok(format!("mcp `{name}` ← {label} OK ({mcp_path})"))
        }
        AssetKind::Skill => {
            let adapter = platform::aiconfig_adapter(scope.asset_root);
            let dest = adapter.skills_dir().join(name);
            remove_native_platform_copy(&dest)?;
            Ok(format!("skill `{name}` ← {label} OK ({dest})"))
        }
        AssetKind::Rule => {
            let adapter = platform::aiconfig_adapter(scope.asset_root);
            let dest = adapter.rules_dir().join(format!("{name}.mdc"));
            remove_native_platform_copy(&dest)?;
            Ok(format!("rule `{name}` ← {label} OK ({dest})"))
        }
        AssetKind::Command => {
            let adapter = platform::aiconfig_adapter(scope.asset_root);
            let dest = adapter.commands_dir().join(format!("{name}.md"));
            remove_native_platform_copy(&dest)?;
            Ok(format!("command `{name}` ← {label} OK ({dest})"))
        }
        AssetKind::Agent => {
            let src = locate_source(scope.default_root, scope.asset_root, kind, name)?;
            remove_native_platform_copy(&src)?;
            Ok(format!("agent `{name}` ← {label} OK ({src})"))
        }
    }
}

/// 收回平台副本：优先走 marker / legacy symlink；无标记的本机副本直接删除。
fn remove_native_platform_copy(dest: &Utf8Path) -> Result<(), CoreError> {
    if materialize::retract(dest).is_ok() {
        return Ok(());
    }
    if fs::symlink_metadata(dest.as_std_path()).is_err() {
        return Ok(());
    }
    if dest.is_dir() {
        fs::remove_dir_all(dest.as_std_path()).map_err(CoreError::Io)
    } else {
        fs::remove_file(dest.as_std_path()).map_err(CoreError::Io)
    }
}

/// 计算平台收回路径：有源时按源映射；无源时按平台目录（外部 / synced 安装）。
fn resolve_platform_retract_dest(
    scope: &ScopeRoots<'_>,
    plat: PlatformId,
    kind: AssetKind,
    name: &str,
) -> Result<Utf8PathBuf, CoreError> {
    if let Ok(src) = locate_source(scope.default_root, scope.asset_root, kind, name) {
        if let Some(dest) = asset_dest_for_at_base(plat, kind, name, &src, scope.deploy_base) {
            return Ok(dest);
        }
    }
    platform_native_dest(scope, plat, kind, name)
}

fn platform_native_dest(
    scope: &ScopeRoots<'_>,
    plat: PlatformId,
    kind: AssetKind,
    name: &str,
) -> Result<Utf8PathBuf, CoreError> {
    let adapter = platform::for_scope_with_asset(plat, scope.deploy_base, scope.asset_root)?;
    let dest = match kind {
        AssetKind::Skill => adapter.skills_dir().join(name),
        AssetKind::Rule => adapter.rules_dir().join(format!("{name}.mdc")),
        AssetKind::Command => adapter.commands_dir().join(format!("{name}.md")),
        AssetKind::Agent => adapter.agents_dir().join(name),
        AssetKind::Mcp => {
            return Err(CoreError::InvalidPath(
                "MCP 收回需要 ai-config 源中的条目".into(),
            ));
        }
    };
    Ok(dest)
}

fn remove_source_entry(
    scope: &ScopeRoots<'_>,
    kind: AssetKind,
    name: &str,
) -> Result<String, CoreError> {
    match kind {
        AssetKind::Mcp => delete_mcp(scope, name),
        AssetKind::Skill => {
            let skill_md = locate_source(scope.default_root, scope.asset_root, kind, name)?;
            let skill_dir = skill_md
                .parent()
                .ok_or_else(|| CoreError::InvalidPath(format!("skill `{name}` 无父目录")))?;
            let dir_str = skill_dir.to_string();
            fs::remove_dir_all(skill_dir).map_err(CoreError::Io)?;
            Ok(format!("skill `{name}` 已从源删除 ({dir_str})"))
        }
        AssetKind::Rule | AssetKind::Command => {
            let path = locate_source(scope.default_root, scope.asset_root, kind, name)?;
            let path_str = path.to_string();
            fs::remove_file(&path).map_err(CoreError::Io)?;
            Ok(format!("{kind:?} `{name}` 已从源删除 ({path_str})"))
        }
        AssetKind::Agent => {
            let src = locate_source(scope.default_root, scope.asset_root, kind, name)?;
            let path_str = src.to_string();
            if src.is_dir() {
                fs::remove_dir_all(&src).map_err(CoreError::Io)?;
            } else {
                fs::remove_file(&src).map_err(CoreError::Io)?;
            }
            Ok(format!("agent `{name}` 已从源删除 ({path_str})"))
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

fn platform_asset_paths(
    from: &dyn platform::PlatformAdapter,
    to: &dyn platform::PlatformAdapter,
    kind: AssetKind,
    name: &str,
) -> Result<(Utf8PathBuf, Utf8PathBuf), CoreError> {
    match kind {
        AssetKind::Skill => {
            let from_dir = from.skills_dir().join(name);
            if !from_dir.join("SKILL.md").is_file() {
                return Err(CoreError::AssetNotFound {
                    kind,
                    name: name.into(),
                    hint: format!("缺少 SKILL.md: {from_dir}"),
                });
            }
            Ok((from_dir, to.skills_dir().join(name)))
        }
        AssetKind::Rule => {
            let from_path = from.rules_dir().join(format!("{name}.mdc"));
            Ok((from_path, to.rules_dir().join(format!("{name}.mdc"))))
        }
        AssetKind::Command => {
            let from_path = from.commands_dir().join(format!("{name}.md"));
            Ok((from_path, to.commands_dir().join(format!("{name}.md"))))
        }
        AssetKind::Agent => {
            let from_dir = from.agents_dir().join(name);
            if from_dir.is_dir() {
                Ok((from_dir.clone(), to.agents_dir().join(name)))
            } else {
                let from_file = find_agent_file(&from.agents_dir(), name)?;
                let ext = from_file
                    .extension()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "md".to_string());
                Ok((from_file, to.agents_dir().join(format!("{name}.{ext}"))))
            }
        }
        AssetKind::Mcp => Err(CoreError::InvalidPath("MCP 不支持平台间直拷".into())),
    }
}

fn find_agent_file(agents_dir: &Utf8Path, name: &str) -> Result<Utf8PathBuf, CoreError> {
    let direct = agents_dir.join(format!("{name}.md"));
    if direct.is_file() {
        return Ok(direct);
    }
    for ext in ["yaml", "yml", "toml"] {
        let path = agents_dir.join(format!("{name}.{ext}"));
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(CoreError::AssetNotFound {
        kind: AssetKind::Agent,
        name: name.into(),
        hint: format!("agents 目录中找不到 agent `{name}`"),
    })
}

fn deploy_skill(scope: &ScopeRoots<'_>, name: &str, plat: PlatformId) -> Result<String, CoreError> {
    let skill_md = locate_source(scope.default_root, scope.asset_root, AssetKind::Skill, name)?;
    let src = skill_link_src(&skill_md);
    let dest = asset_dest_for_at_base(plat, AssetKind::Skill, name, &skill_md, scope.deploy_base)
        .ok_or_else(|| {
        CoreError::InvalidPath(format!("无法算 skill `{name}` → {plat:?} 的 dest"))
    })?;
    materialize::deploy(&src, &dest)?;
    if plat == PlatformId::Hermes {
        let skills_parent = dest.parent().unwrap_or(&dest).to_path_buf();
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
    let dest =
        asset_dest_for_at_base(plat, kind, name, &src, scope.deploy_base).ok_or_else(|| {
            CoreError::InvalidPath(format!("无法算 {kind:?} `{name}` → {plat:?} 的 dest"))
        })?;
    materialize::deploy(&link_src, &dest)?;
    Ok(format!("{kind:?} `{name}` → {plat:?} OK ({dest})"))
}

fn deploy_mcp(scope: &ScopeRoots<'_>, name: &str, plat: PlatformId) -> Result<String, CoreError> {
    let mcp_path = asset_scope::locate_mcp_json(scope.default_root, scope.asset_root)?;
    let doc_root = mcp_asset_root(&mcp_path)?;
    let config =
        mcp_json::get_server_config(doc_root, name)?.ok_or_else(|| CoreError::AssetNotFound {
            kind: AssetKind::Mcp,
            name: name.into(),
            hint: "MCP server 在 mcp.json 中找不到".into(),
        })?;
    let dest = platform::for_scope(plat, scope.deploy_base)?.mcp_deploy_path();
    mcp_json::upsert_server_on_platform(plat, &dest, name, &config, Some(&mcp_path))?;
    Ok(format!("mcp `{name}` → {plat:?} OK ({dest})"))
}

fn retract_mcp(scope: &ScopeRoots<'_>, name: &str, plat: PlatformId) -> Result<String, CoreError> {
    let mcp_path = asset_scope::locate_mcp_json(scope.default_root, scope.asset_root)?;
    let dest = platform::for_scope(plat, scope.deploy_base)?.mcp_deploy_path();
    mcp_json::remove_server_on_platform(plat, &dest, name, Some(&mcp_path))?;
    Ok(format!("mcp `{name}` ← {plat:?} 已移除 ({dest})"))
}

fn get_mcp_detail(scope: &ScopeRoots<'_>, name: &str) -> Result<AssetFileDetail, CoreError> {
    let mcp_path = asset_scope::locate_mcp_json(scope.default_root, scope.asset_root)?;
    let doc_root = mcp_asset_root(&mcp_path)?;
    let config =
        mcp_json::get_server_config(doc_root, name)?.ok_or_else(|| CoreError::AssetNotFound {
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

fn collect_yaml_block_scalar_lines(lines: &[&str], start: usize, _folded: bool) -> (String, usize) {
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
        let text = trimmed.strip_prefix('>').map(str::trim).unwrap_or(trimmed);
        if !text.is_empty() {
            parts.push(text.to_string());
        }
        i += 1;
    }
    let joined = parts.join(" ");
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
mod deploy_from_platform_tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn deploy_skill_from_claude_to_cursor_preserves_claude() {
        let tmp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(tmp.path()).unwrap();
        let _home_guard = crate::test_env::EnvGuard::set("HOME", tmp.path().to_str().unwrap());

        let claude_skill = home.join(".claude/skills/find-skills");
        fs::create_dir_all(&claude_skill).unwrap();
        fs::write(
            claude_skill.join("SKILL.md"),
            "---\nname: find-skills\ndescription: demo\n---\n# Find\n",
        )
        .unwrap();

        let asset_root = home.join(".ai-config");
        fs::create_dir_all(asset_root.join("skills")).unwrap();
        mcp_json::ensure_mcp_json(&asset_root).unwrap();

        let scope = ScopeRoots {
            default_root: &asset_root,
            asset_root: &asset_root,
            deploy_base: home,
        };
        deploy_from_platform(
            &scope,
            AssetKind::Skill,
            "find-skills",
            PlatformId::Claude,
            PlatformId::Cursor,
        )
        .unwrap();

        let cursor_skill = home.join(".cursor/skills/find-skills");
        assert!(cursor_skill.join("SKILL.md").is_file());
        assert!(
            claude_skill.join("SKILL.md").is_file(),
            "Claude 侧原始安装应保留"
        );
        assert!(
            cursor_skill.join(".ai-config-deploy.json").is_file(),
            "目标平台应有下发标记"
        );
        assert!(
            !claude_skill.join(".ai-config-deploy.json").exists()
                || claude_skill.join("SKILL.md").is_file(),
            "Claude 侧不应因拷贝而被删除"
        );
    }
    #[test]
    fn retract_aiconfig_keeps_other_platform_copy() {
        let tmp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(tmp.path()).unwrap();
        let _home_guard = crate::test_env::EnvGuard::set("HOME", tmp.path().to_str().unwrap());

        let asset_root = home.join(".ai-config");
        let skill_dir = asset_root.join("skills/keep-me");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "# keep\n").unwrap();
        mcp_json::ensure_mcp_json(&asset_root).unwrap();

        let claude_skill = home.join(".claude/skills/keep-me");
        fs::create_dir_all(&claude_skill).unwrap();
        fs::write(claude_skill.join("SKILL.md"), "# keep\n").unwrap();

        let scope = ScopeRoots {
            default_root: &asset_root,
            asset_root: &asset_root,
            deploy_base: home,
        };
        let msg = retract(&scope, AssetKind::Skill, "keep-me", PlatformId::AiConfig).unwrap();
        assert!(
            msg.contains("← aiconfig"),
            "expected platform retract message, got: {msg}"
        );
        assert!(!msg.contains("已从源收回"));

        assert!(!skill_dir.exists());
        assert!(
            claude_skill.join("SKILL.md").is_file(),
            "收回 ai-config 平台副本不应删除 Claude 侧 skill"
        );
    }

    #[test]
    fn retract_hermes_skill_without_aiconfig_source() {
        let tmp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(tmp.path()).unwrap();
        let _home_guard = crate::test_env::EnvGuard::set("HOME", tmp.path().to_str().unwrap());

        let hermes_skill = home.join(".hermes/skills/npx-only");
        fs::create_dir_all(&hermes_skill).unwrap();
        fs::write(hermes_skill.join("SKILL.md"), "# npx only\n").unwrap();

        let asset_root = home.join(".ai-config");
        fs::create_dir_all(asset_root.join("skills")).unwrap();
        mcp_json::ensure_mcp_json(&asset_root).unwrap();

        let scope = ScopeRoots {
            default_root: &asset_root,
            asset_root: &asset_root,
            deploy_base: home,
        };
        retract(&scope, AssetKind::Skill, "npx-only", PlatformId::Hermes).unwrap();
        assert!(
            !hermes_skill.exists(),
            "应能删除无 ai-config 源的外部 Hermes skill"
        );
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
