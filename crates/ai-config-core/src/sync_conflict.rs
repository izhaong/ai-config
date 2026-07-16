//! 跨平台同步冲突检测：以「当前浏览平台」为基准，对比目标平台是否已有不同内容。

use std::fs;

use camino::{Utf8Path, Utf8PathBuf};
use schemars::JsonSchema;
use serde::Serialize;

use crate::asset_ops::{self, ScopeRoots};
use crate::error::CoreError;
use crate::mcp_json;
use crate::model::{AssetKind, PlatformId};
use crate::platform::{self, platform_label};
use crate::platform_scan;

/// 某平台上该资产的可读内容与摘要。
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PlatformAssetVariant {
    pub platform: String,
    pub platform_label: String,
    pub content: String,
    pub content_summary: String,
    pub differs_from_baseline: bool,
}

/// 同步前冲突报告（目标平台已有同名但不同内容的副本）。
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SyncConflictReport {
    pub kind: String,
    pub name: String,
    pub baseline_platform: String,
    pub target_platform: String,
    pub variants: Vec<PlatformAssetVariant>,
}

struct ResolvedAsset {
    preview_path: Utf8PathBuf,
    normalized: String,
}

/// 检测 `baseline_plat → target_plat` 同步是否会覆盖不同内容。
/// 目标不存在或与基准一致时返回 `None`。
pub fn detect_sync_conflict(
    scope: &ScopeRoots<'_>,
    kind: AssetKind,
    name: &str,
    baseline_plat: PlatformId,
    target_plat: PlatformId,
) -> Result<Option<SyncConflictReport>, CoreError> {
    if kind == AssetKind::Prompt {
        return Err(CoreError::InvalidPath(
            "Prompt 仅能经 source-first projection planner；旧冲突检测 lifecycle 已禁用".into(),
        ));
    }
    if baseline_plat == target_plat {
        return Ok(None);
    }
    if !platform::supports_at_scope(target_plat, kind, scope.deploy_base) {
        return Ok(None);
    }

    let baseline = match resolve_platform_asset(scope, kind, name, baseline_plat)? {
        Some(v) => v,
        None => return Ok(None),
    };
    let target = match resolve_platform_asset(scope, kind, name, target_plat)? {
        Some(v) => v,
        None => return Ok(None),
    };
    if baseline.normalized == target.normalized {
        return Ok(None);
    }

    let mut variants = Vec::new();
    for plat in platform::ui_platform_ids() {
        if !platform::supports_at_scope(plat, kind, scope.deploy_base) {
            continue;
        }
        let Some(resolved) = resolve_platform_asset(scope, kind, name, plat)? else {
            continue;
        };
        let detail = asset_ops::read_platform_preview(kind, &resolved.preview_path, name)?;
        variants.push(PlatformAssetVariant {
            platform: platform_label(plat).to_string(),
            platform_label: platform_display_name(plat),
            content: detail.content,
            content_summary: detail.description,
            differs_from_baseline: resolved.normalized != baseline.normalized,
        });
    }

    Ok(Some(SyncConflictReport {
        kind: platform::asset_kind_label(kind).to_string(),
        name: name.to_string(),
        baseline_platform: platform_label(baseline_plat).to_string(),
        target_platform: platform_label(target_plat).to_string(),
        variants,
    }))
}

/// 按用户所选来源平台，将资产写入目标平台。
pub fn apply_sync_choice(
    scope: &ScopeRoots<'_>,
    kind: AssetKind,
    name: &str,
    source_plat: PlatformId,
    target_plat: PlatformId,
) -> Result<String, CoreError> {
    if kind == AssetKind::Prompt {
        return Err(CoreError::InvalidPath(
            "Prompt 仅能经 source-first projection planner；旧同步 lifecycle 已禁用".into(),
        ));
    }
    if source_plat == target_plat {
        return Err(CoreError::InvalidPath("来源与目标平台不能相同".into()));
    }
    match (source_plat, target_plat) {
        (PlatformId::AiConfig, t) => asset_ops::deploy(scope, kind, name, t),
        (s, PlatformId::AiConfig) => import_to_source(scope, kind, name, s),
        (s, t) if s.is_deploy_target() && t.is_deploy_target() => {
            asset_ops::deploy_from_platform(scope, kind, name, s, t)
        }
        _ => Err(CoreError::InvalidPath(format!(
            "无法从 `{}` 同步到 `{}`",
            platform_display_name(source_plat),
            platform_display_name(target_plat),
        ))),
    }
}

fn import_to_source(
    scope: &ScopeRoots<'_>,
    kind: AssetKind,
    name: &str,
    from_plat: PlatformId,
) -> Result<String, CoreError> {
    if from_plat == PlatformId::AiConfig {
        return Err(CoreError::InvalidPath("不能从 ai-config 导入到自身".into()));
    }
    let dest = match kind {
        AssetKind::Skill => platform_scan::import_skill_from_platform(
            name,
            from_plat,
            scope.default_root,
            scope.asset_root,
            scope.deploy_base,
        )?,
        AssetKind::Rule => platform_scan::import_rule_from_platform(
            name,
            from_plat,
            scope.default_root,
            scope.asset_root,
            scope.deploy_base,
        )?,
        AssetKind::Command => platform_scan::import_command_from_platform(
            name,
            from_plat,
            scope.default_root,
            scope.asset_root,
            scope.deploy_base,
        )?,
        AssetKind::Agent => platform_scan::import_agent_from_platform(
            name,
            from_plat,
            scope.default_root,
            scope.asset_root,
            scope.deploy_base,
        )?,
        AssetKind::Mcp => platform_scan::import_mcp_from_platform(
            name,
            from_plat,
            scope.default_root,
            scope.asset_root,
            scope.deploy_base,
        )?,
        AssetKind::Hook => {
            let path = platform_scan::import_hook_from_platform(
                name,
                from_plat,
                scope.default_root,
                scope.asset_root,
                scope.deploy_base,
            )?;
            return Ok(format!("Hook `{name}` 已导入到 {path}"));
        }
        AssetKind::Prompt => {
            return Err(CoreError::InvalidPath(
                "Prompt 仅能经 source-first projection planner；旧导入 lifecycle 已禁用".into(),
            ));
        }
    };
    Ok(format!(
        "{kind:?} `{name}` 已从 {} 导入到 {dest}",
        platform_display_name(from_plat),
    ))
}

fn platform_display_name(plat: PlatformId) -> String {
    match plat {
        PlatformId::AiConfig => "ai-config".to_string(),
        other => platform_label(other).to_string(),
    }
}

fn resolve_platform_asset(
    scope: &ScopeRoots<'_>,
    kind: AssetKind,
    name: &str,
    plat: PlatformId,
) -> Result<Option<ResolvedAsset>, CoreError> {
    let adapter = platform::for_scope_with_asset(plat, scope.deploy_base, scope.asset_root)?;
    let preview_path = match kind {
        AssetKind::Skill => {
            let dir = adapter.skills_dir().join(name);
            if !dir.join("SKILL.md").is_file() {
                return Ok(None);
            }
            dir
        }
        AssetKind::Rule => {
            let path = adapter.rules_dir().join(format!("{name}.mdc"));
            if !path.is_file() {
                return Ok(None);
            }
            path
        }
        AssetKind::Command => {
            let path = adapter.commands_dir().join(format!("{name}.md"));
            if !path.is_file() {
                return Ok(None);
            }
            path
        }
        AssetKind::Agent => {
            let dir = adapter.agents_dir().join(name);
            if dir.is_dir() {
                dir
            } else if let Some(file) = find_agent_on_platform(&adapter.agents_dir(), name)? {
                file
            } else {
                return Ok(None);
            }
        }
        AssetKind::Mcp => {
            let deploy_path = adapter.mcp_deploy_path();
            if platform_scan::read_platform_mcp_server_config(adapter.as_ref(), name).is_err() {
                return Ok(None);
            }
            deploy_path
        }
        AssetKind::Hook => {
            let path = crate::hook_adapter::platform_script_path(scope.deploy_base, plat, name);
            if !path.is_file() {
                return Ok(None);
            }
            path
        }
        AssetKind::Prompt => {
            return Err(CoreError::InvalidPath(
                "Prompt 仅能经 source-first projection planner；旧解析 lifecycle 已禁用".into(),
            ));
        }
    };

    let normalized = normalized_asset_fingerprint(scope, kind, name, plat, &preview_path)?;
    Ok(Some(ResolvedAsset {
        preview_path,
        normalized,
    }))
}

fn find_agent_on_platform(
    agents_dir: &Utf8Path,
    name: &str,
) -> Result<Option<Utf8PathBuf>, CoreError> {
    let direct = agents_dir.join(format!("{name}.md"));
    if direct.is_file() {
        return Ok(Some(direct));
    }
    for ext in ["yaml", "yml", "toml"] {
        let path = agents_dir.join(format!("{name}.{ext}"));
        if path.is_file() {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn normalized_asset_fingerprint(
    _scope: &ScopeRoots<'_>,
    kind: AssetKind,
    name: &str,
    _plat: PlatformId,
    preview_path: &Utf8Path,
) -> Result<String, CoreError> {
    match kind {
        AssetKind::Mcp => {
            let config = mcp_json::get_server_config_from_deploy_file(preview_path, name)?;
            serde_json::to_string(&config).map_err(|e| CoreError::Io(e.into()))
        }
        AssetKind::Skill => read_file_normalized(&preview_path.join("SKILL.md")),
        AssetKind::Rule | AssetKind::Command => read_file_normalized(preview_path),
        AssetKind::Agent => {
            if preview_path.is_dir() {
                let skill_md = preview_path.join("SKILL.md");
                if skill_md.is_file() {
                    read_file_normalized(&skill_md)
                } else {
                    read_file_normalized(preview_path)
                }
            } else {
                read_file_normalized(preview_path)
            }
        }
        AssetKind::Hook => read_file_normalized(preview_path),
        AssetKind::Prompt => Err(CoreError::InvalidPath(
            "Prompt 仅能经 source-first projection planner；旧指纹 lifecycle 已禁用".into(),
        )),
    }
}

fn read_file_normalized(path: &Utf8Path) -> Result<String, CoreError> {
    let raw = fs::read_to_string(path.as_std_path()).map_err(CoreError::Io)?;
    Ok(raw.replace("\r\n", "\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scope<'a>(home: &'a Utf8Path, asset_root: &'a Utf8Path) -> ScopeRoots<'a> {
        ScopeRoots {
            default_root: asset_root,
            asset_root,
            deploy_base: home,
        }
    }

    #[test]
    fn no_conflict_when_target_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let home = Utf8Path::from_path(tmp.path()).unwrap();
        let _guard = crate::test_env::EnvGuard::set("HOME", tmp.path().to_str().unwrap());
        let asset_root = home.join(".ai-config");
        crate::paths::ensure_user_asset_layout(&asset_root).unwrap();

        let cursor_skill = home.join(".cursor/skills/demo");
        fs::create_dir_all(&cursor_skill).unwrap();
        fs::write(cursor_skill.join("SKILL.md"), "# demo\n").unwrap();

        let s = scope(home, &asset_root);
        let report = detect_sync_conflict(
            &s,
            AssetKind::Skill,
            "demo",
            PlatformId::Cursor,
            PlatformId::Claude,
        )
        .unwrap();
        assert!(report.is_none());
    }

    #[test]
    fn conflict_when_same_name_different_skill_content() {
        let tmp = tempfile::tempdir().unwrap();
        let home = Utf8Path::from_path(tmp.path()).unwrap();
        let _guard = crate::test_env::EnvGuard::set("HOME", tmp.path().to_str().unwrap());
        let asset_root = home.join(".ai-config");
        crate::paths::ensure_user_asset_layout(&asset_root).unwrap();

        let cursor_skill = home.join(".cursor/skills/demo");
        fs::create_dir_all(&cursor_skill).unwrap();
        fs::write(cursor_skill.join("SKILL.md"), "# from cursor\n").unwrap();

        let claude_skill = home.join(".claude/skills/demo");
        fs::create_dir_all(&claude_skill).unwrap();
        fs::write(claude_skill.join("SKILL.md"), "# from claude\n").unwrap();

        let s = scope(home, &asset_root);
        let report = detect_sync_conflict(
            &s,
            AssetKind::Skill,
            "demo",
            PlatformId::Cursor,
            PlatformId::Claude,
        )
        .unwrap()
        .expect("should conflict");

        assert_eq!(report.variants.len(), 2);
        let cursor_v = report
            .variants
            .iter()
            .find(|v| v.platform == "cursor")
            .unwrap();
        let claude_v = report
            .variants
            .iter()
            .find(|v| v.platform == "claude")
            .unwrap();
        assert!(!cursor_v.differs_from_baseline);
        assert!(claude_v.differs_from_baseline);
    }

    #[test]
    fn apply_sync_choice_overwrites_with_selected_source() {
        let tmp = tempfile::tempdir().unwrap();
        let home = Utf8Path::from_path(tmp.path()).unwrap();
        let _guard = crate::test_env::EnvGuard::set("HOME", tmp.path().to_str().unwrap());
        let asset_root = home.join(".ai-config");
        crate::paths::ensure_user_asset_layout(&asset_root).unwrap();

        let cursor_skill = home.join(".cursor/skills/demo");
        fs::create_dir_all(&cursor_skill).unwrap();
        fs::write(cursor_skill.join("SKILL.md"), "# winner\n").unwrap();

        let claude_skill = home.join(".claude/skills/demo");
        fs::create_dir_all(&claude_skill).unwrap();
        fs::write(claude_skill.join("SKILL.md"), "# loser\n").unwrap();

        let s = scope(home, &asset_root);
        apply_sync_choice(
            &s,
            AssetKind::Skill,
            "demo",
            PlatformId::Cursor,
            PlatformId::Claude,
        )
        .unwrap();

        let after = fs::read_to_string(claude_skill.join("SKILL.md")).unwrap();
        assert!(after.contains("winner"));
    }
}
