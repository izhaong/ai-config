//! 作用域内资产定位与批量收回（GUI / CLI 共用）。

use camino::{Utf8Path, Utf8PathBuf};

use crate::error::CoreError;
use crate::link;
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
        // T001: a same-name entry in a platform aggregate config is not ownership
        // evidence. T002's projection ledger will enable narrowly-owned removal.
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
        let expected_src = crate::sync::link_src_for_create(kind, &src);
        remove_platform_copy_best_effort(&dest, &expected_src);
    }
}

fn remove_platform_copy_best_effort(dest: &Utf8Path, expected_src: &Utf8Path) {
    if let Ok(metadata) = std::fs::symlink_metadata(dest.as_std_path()) {
        if metadata.file_type().is_symlink() {
            if matches!(
                link::check(dest, expected_src),
                link::LinkHealth::Linked { .. }
            ) {
                let _ = link::unlink(dest);
            }
            return;
        }
    }
    let _ = materialize::retract(dest);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::asset_dest_for_at_base;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn source_delete_preserves_unmanaged_platform_skill_directory() {
        let temp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let asset_root = root.join(".agents");
        let source = asset_root.join("skills/demo/SKILL.md");
        fs::create_dir_all(source.parent().unwrap().as_std_path()).unwrap();
        fs::write(
            source.as_std_path(),
            "---\nname: demo\n---\nmanaged source\n",
        )
        .unwrap();

        let dest = asset_dest_for_at_base(
            crate::model::PlatformId::Cursor,
            AssetKind::Skill,
            "demo",
            &source,
            root,
        )
        .unwrap();
        fs::create_dir_all(dest.as_std_path()).unwrap();
        fs::write(dest.join("SKILL.md").as_std_path(), "third-party copy\n").unwrap();

        retract_all_platforms_best_effort(&asset_root, &asset_root, root, AssetKind::Skill, "demo");

        assert!(
            dest.join("SKILL.md").exists(),
            "source deletion must not remove an unmanaged platform directory"
        );
    }

    #[cfg(unix)]
    #[test]
    fn source_delete_preserves_platform_skill_symlink_to_foreign_source() {
        let temp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let asset_root = root.join(".agents");
        let source = asset_root.join("skills/demo/SKILL.md");
        fs::create_dir_all(source.parent().unwrap().as_std_path()).unwrap();
        fs::write(
            source.as_std_path(),
            "---\nname: demo\n---\nmanaged source\n",
        )
        .unwrap();

        let foreign_source = root.join("third-party/demo");
        fs::create_dir_all(foreign_source.as_std_path()).unwrap();
        fs::write(
            foreign_source.join("SKILL.md").as_std_path(),
            "third-party source\n",
        )
        .unwrap();

        let dest = asset_dest_for_at_base(
            crate::model::PlatformId::Cursor,
            AssetKind::Skill,
            "demo",
            &source,
            root,
        )
        .unwrap();
        fs::create_dir_all(dest.parent().unwrap().as_std_path()).unwrap();
        std::os::unix::fs::symlink(foreign_source.as_std_path(), dest.as_std_path()).unwrap();

        retract_all_platforms_best_effort(&asset_root, &asset_root, root, AssetKind::Skill, "demo");

        assert!(
            std::fs::symlink_metadata(dest.as_std_path()).is_ok(),
            "source deletion must not remove a platform symlink to a foreign source"
        );
    }

    #[test]
    fn source_delete_preserves_same_name_platform_mcp_without_ownership_record() {
        let temp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let asset_root = root.join(".agents");
        fs::create_dir_all(asset_root.as_std_path()).unwrap();
        fs::write(
            asset_root.join("mcp.json").as_std_path(),
            r#"{"mcpServers":{"demo":{"command":"managed-command"}}}"#,
        )
        .unwrap();

        let platform_mcp = root.join(".cursor/mcp.json");
        fs::create_dir_all(platform_mcp.parent().unwrap().as_std_path()).unwrap();
        fs::write(
            platform_mcp.as_std_path(),
            r#"{"mcpServers":{"demo":{"command":"foreign-command"},"keep":{"command":"keep"}}}"#,
        )
        .unwrap();

        retract_all_platforms_best_effort(&asset_root, &asset_root, root, AssetKind::Mcp, "demo");

        let after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(platform_mcp.as_std_path()).unwrap()).unwrap();
        assert_eq!(
            after["mcpServers"]["demo"]["command"], "foreign-command",
            "source deletion must not remove a same-name MCP server without ownership evidence"
        );
        assert_eq!(after["mcpServers"]["keep"]["command"], "keep");
    }
}
