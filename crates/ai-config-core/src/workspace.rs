//! 聚合仓（monorepo）工作区：解析 `.gitmodules`、子仓资产继承。

use std::fs;

use camino::{Utf8Path, Utf8PathBuf};

use crate::error::CoreError;
use crate::paths::{self, SyncRoots};

/// 子仓是否已有独立 `.ai-config` 内容（非空 skills / rules / mcp 等）。
pub fn member_has_local_assets(member_asset: &Utf8Path) -> bool {
    if !member_asset.is_dir() {
        return false;
    }
    if member_asset.join("mcp.json").is_file() {
        return true;
    }
    for sub in ["skills", "rules", "commands", "agents", "hooks"] {
        let dir = member_asset.join(sub);
        if dir.is_dir() {
            if let Ok(mut entries) = fs::read_dir(dir.as_std_path()) {
                if entries.any(|e| {
                    e.ok()
                        .map(|x| x.file_name().to_string_lossy().starts_with('.'))
                        .map(|hidden| !hidden)
                        .unwrap_or(false)
                }) {
                    return true;
                }
            }
        }
    }
    member_asset.join("hooks.json").is_file()
}

/// 工作区成员的有效资产根：子仓优先，否则继承 workspace `.ai-config`。
pub fn effective_asset_root_for_member(
    member_repo: &Utf8Path,
    workspace_root: &Utf8Path,
) -> Utf8PathBuf {
    let (_, member_asset) = paths::resolve_project_roots(member_repo);
    if member_has_local_assets(&member_asset) {
        return member_asset;
    }
    let workspace_asset = paths::project_asset_root(workspace_root);
    if workspace_asset.is_dir() {
        return workspace_asset;
    }
    member_asset
}

/// 解析单成员 install/sync 作用域（下发到成员仓库根）。
pub fn resolve_member_sync_roots(
    member_repo: &Utf8Path,
    workspace_root: &Utf8Path,
) -> SyncRoots {
    let global_default = paths::discover_global_asset_root();
    let (repo_root, _) = paths::resolve_project_roots(member_repo);
    let asset_root = effective_asset_root_for_member(&repo_root, workspace_root);
    SyncRoots {
        repo_root: repo_root.clone(),
        asset_root,
        global_default,
        deploy_base: paths::project_deploy_base(&repo_root),
    }
}

/// 从 `.gitmodules` 解析子模块路径（保持文件顺序）。
pub fn parse_gitmodules_paths(workspace_root: &Utf8Path) -> Result<Vec<Utf8PathBuf>, CoreError> {
    let path = workspace_root.join(".gitmodules");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(path.as_std_path()).map_err(CoreError::Io)?;
    let mut out = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("path = ") {
            let p = rest.trim();
            if !p.is_empty() {
                out.push(workspace_root.join(p));
            }
        }
    }
    Ok(out)
}

/// 工作区成员：父仓根 + 已 checkout 的子模块目录。
pub fn discover_members(workspace_root: &Utf8Path) -> Result<Vec<Utf8PathBuf>, CoreError> {
    let mut members = vec![workspace_root.to_path_buf()];
    for sub in parse_gitmodules_paths(workspace_root)? {
        if sub.is_dir() {
            members.push(sub);
        }
    }
    Ok(members)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn parse_gitmodules_extracts_paths() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::write(
            root.join(".gitmodules"),
            r#"
[submodule "a"]
	path = pkg/a
[submodule "b"]
	path = pkg/b
"#,
        )
        .unwrap();
        let paths = parse_gitmodules_paths(&root).unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], root.join("pkg/a"));
        assert_eq!(paths[1], root.join("pkg/b"));
    }

    #[test]
    fn discover_members_skips_missing_checkout() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(root.join("pkg/a")).unwrap();
        fs::write(
            root.join(".gitmodules"),
            "[submodule \"a\"]\n\tpath = pkg/a\n[submodule \"b\"]\n\tpath = pkg/b\n",
        )
        .unwrap();
        let members = discover_members(&root).unwrap();
        assert_eq!(members.len(), 2);
        assert!(members.iter().any(|p| p.ends_with("pkg/a")));
        assert!(!members.iter().any(|p| p.ends_with("pkg/b")));
    }

    #[test]
    fn effective_asset_inherits_workspace() {
        let tmp = TempDir::new().unwrap();
        let ws = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(ws.join(".ai-config/skills/foo")).unwrap();
        fs::write(ws.join(".ai-config/skills/foo/SKILL.md"), "x").unwrap();
        let member = ws.join("child");
        fs::create_dir_all(&member).unwrap();
        let asset = effective_asset_root_for_member(&member, &ws);
        assert_eq!(asset, ws.join(".ai-config"));
    }

    #[test]
    fn effective_asset_prefers_member_when_present() {
        let tmp = TempDir::new().unwrap();
        let ws = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(ws.join(".ai-config/skills/global")).unwrap();
        fs::write(
            ws.join(".ai-config/skills/global/SKILL.md"),
            "g",
        )
        .unwrap();
        let member = ws.join("child");
        fs::create_dir_all(member.join(".ai-config/skills/local")).unwrap();
        fs::write(
            member.join(".ai-config/skills/local/SKILL.md"),
            "l",
        )
        .unwrap();
        let asset = effective_asset_root_for_member(&member, &ws);
        assert_eq!(asset, member.join(".ai-config"));
    }
}
