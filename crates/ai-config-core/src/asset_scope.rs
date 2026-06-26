//! 作用域内资产定位与批量收回（GUI / CLI 共用）。

use camino::{Utf8Path, Utf8PathBuf};

use crate::error::CoreError;
use crate::materialize;
use crate::mcp_json;
use crate::model::AssetKind;
use crate::platform;
use crate::platform_scan::{find_source_path, scan_source_for_scope};
use crate::sync::asset_dest_for_at_base;

/// 定位当前作用域的 `mcp.json` 源路径。
pub fn locate_mcp_json(
    default_root: &Utf8Path,
    asset_root: &Utf8Path,
) -> Result<Utf8PathBuf, CoreError> {
    let scan = scan_source_for_scope(default_root, asset_root)?;
    scan.mcp_json.ok_or_else(|| CoreError::AssetNotFound {
        kind: AssetKind::Mcp,
        name: mcp_json::MCP_ASSET_NAME.into(),
        hint: "源中找不到 mcp.json".into(),
    })
}

/// 解析资产类型字符串（CLI / GUI 入参）。
pub fn parse_asset_kind(s: &str) -> Result<AssetKind, CoreError> {
    match s {
        "skill" | "Skill" => Ok(AssetKind::Skill),
        "rule" | "Rule" => Ok(AssetKind::Rule),
        "mcp" | "Mcp" => Ok(AssetKind::Mcp),
        "agent" | "Agent" => Ok(AssetKind::Agent),
        "command" | "Command" => Ok(AssetKind::Command),
        "hook" | "Hook" => Ok(AssetKind::Hook),
        other => Err(CoreError::InvalidPath(format!("未知资产类型 `{other}`"))),
    }
}

/// 在当前作用域资产根里定位一条资产的源路径。
pub fn locate_source(
    default_root: &Utf8Path,
    asset_root: &Utf8Path,
    kind: AssetKind,
    name: &str,
) -> Result<Utf8PathBuf, CoreError> {
    let scan = scan_source_for_scope(default_root, asset_root)?;
    find_source_path(kind, name, &scan).ok_or_else(|| CoreError::AssetNotFound {
        kind,
        name: name.into(),
        hint: format!("在作用域 `{asset_root}` 中找不到该资产"),
    })
}

/// 删除源前尽力收回各平台链接；失败不阻断删源。
pub fn retract_all_platforms_best_effort(
    default_root: &Utf8Path,
    asset_root: &Utf8Path,
    deploy_base: &Utf8Path,
    kind: AssetKind,
    name: &str,
) {
    if kind == AssetKind::Mcp {
        let Ok(mcp_path) = locate_source(default_root, asset_root, kind, name) else {
            return;
        };
        let Ok(asset_mcp_root) = mcp_path.parent().ok_or(()) else {
            return;
        };
        if mcp_json::get_server_config(asset_mcp_root, name)
            .ok()
            .flatten()
            .is_none()
        {
            return;
        }
        for plat in platform::deploy_platform_ids() {
            if !platform::supports_at_scope(plat, kind, deploy_base) {
                continue;
            }
            if let Ok(adapter) = platform::for_scope(plat, deploy_base) {
                let source_mcp = mcp_json::mcp_json_path(asset_mcp_root);
                let _ = mcp_json::remove_server_on_platform(
                    plat,
                    &adapter.mcp_deploy_path(),
                    name,
                    Some(&source_mcp),
                );
            }
        }
        return;
    }
    if kind == AssetKind::Hook {
        for plat in platform::deploy_platform_ids() {
            if platform::supports_at_scope(plat, kind, deploy_base) {
                let _ = crate::hook_adapter::retract(asset_root, deploy_base, name, plat);
            }
        }
        return;
    }
    let Ok(src) = locate_source(default_root, asset_root, kind, name) else {
        return;
    };
    for plat in platform::deploy_platform_ids() {
        if !platform::supports_at_scope(plat, kind, deploy_base) {
            continue;
        }
        let Some(dest) = asset_dest_for_at_base(plat, kind, name, &src, deploy_base) else {
            continue;
        };
        remove_platform_copy_best_effort(&dest);
    }
}

fn remove_platform_copy_best_effort(dest: &Utf8Path) {
    if materialize::retract(dest).is_ok() {
        return;
    }
    if std::fs::symlink_metadata(dest.as_std_path()).is_err() {
        return;
    }
    if dest.is_dir() {
        let _ = std::fs::remove_dir_all(dest.as_std_path());
    } else {
        let _ = std::fs::remove_file(dest.as_std_path());
    }
}
