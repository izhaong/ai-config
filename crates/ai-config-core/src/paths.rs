//! 用户全局资产根:`~/.ai-config/`(skills / rules / mcp / agents)。
//!
//! - 默认始终读写 `~/.ai-config/`;不存在则创建子目录。
//! - 首次为空时,可从 `AI_CONFIG_SEED` 或安装包 Resources 合并拷贝(不覆盖已有文件)。
//! - 项目覆盖仍在 `<project>/.ai-config/`(结构相同,与全局合并)。
//! - `AI_CONFIG_ROOT` 可覆盖全局根(开发/测试);指向 ai-config 仓库根时回退 `~/.ai-config`。

use camino::{Utf8Path, Utf8PathBuf};
use walkdir::WalkDir;

use crate::error::CoreError;
use crate::mcp_json::{self};

/// 用户主目录下的全局资产目录名。
pub const USER_ASSET_DIR_NAME: &str = ".ai-config";

/// 安装包 Resources 内种子目录名(构建时由 `~/.ai-config` 或 `AI_CONFIG_SEED` 打入)。
pub const BUNDLE_SEED_DIR_NAMES: &[&str] = &[".ai-config", "seed"];

/// 子目录(相对资产根)。
pub const ASSET_SUBDIRS: &[&str] = &["skills", "rules", "agents"];

/// 读 home:`$HOME` / `$USERPROFILE`,失败时退回 `.`(单测稳定)。
pub fn home_dir() -> Utf8PathBuf {
    if let Ok(h) = std::env::var("HOME") {
        if !h.is_empty() {
            return Utf8PathBuf::from(h);
        }
    }
    if let Ok(h) = std::env::var("USERPROFILE") {
        if !h.is_empty() {
            return Utf8PathBuf::from(h);
        }
    }
    Utf8PathBuf::from(".")
}

/// 默认用户全局资产根 `~/.ai-config`。
pub fn user_home_asset_root() -> Utf8PathBuf {
    home_dir().join(USER_ASSET_DIR_NAME)
}

/// 是否为 ai-config 工程仓库根(含 `crates/ai-config-core`)。
pub fn is_ai_config_repo(p: &Utf8Path) -> bool {
    p.join("crates/ai-config-core").is_dir()
}

/// 将注册表或用户输入的路径规范为 `(仓库根, 资产根 …/.ai-config/)`。
///
/// - 仓库根 → 资产根 = `<repo>/.ai-config/`
/// - 用户直接选 `.ai-config/` → 仓库根 = 父目录
/// - 历史数据:路径本身已是资产根(含 `skills/`) → 仓库根与资产根相同
pub fn resolve_project_roots(stored: &Utf8Path) -> (Utf8PathBuf, Utf8PathBuf) {
    if stored
        .file_name()
        .map(|n| n == USER_ASSET_DIR_NAME)
        .unwrap_or(false)
    {
        let repo = stored
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| stored.to_path_buf());
        return (repo, stored.to_path_buf());
    }
    if is_asset_root(stored) {
        return (stored.to_path_buf(), stored.to_path_buf());
    }
    let asset = stored.join(USER_ASSET_DIR_NAME);
    (stored.to_path_buf(), asset)
}

/// 用户全局下发目标:各平台目录在 `$HOME` 下。
pub fn global_deploy_base() -> Utf8PathBuf {
    home_dir()
}

/// 项目下发目标:各平台目录在仓库根下(`<repo>/.cursor` 等)。
pub fn project_deploy_base(repo_root: &Utf8Path) -> Utf8PathBuf {
    repo_root.to_path_buf()
}

/// 候选路径是否已是「资产根」(直接含 `skills/`)。
pub fn is_asset_root(p: &Utf8Path) -> bool {
    p.join("skills").is_dir()
}

/// 将 CLI/GUI 传入路径规范为资产根。
///
/// - 已是资产根 → 原样
/// - ai-config 仓库根 → `~/.ai-config`(资产已迁出仓库)
pub fn resolve_asset_root(candidate: &Utf8Path) -> Utf8PathBuf {
    if is_asset_root(candidate) {
        return candidate.to_path_buf();
    }
    if is_ai_config_repo(candidate) {
        return user_home_asset_root();
    }
    candidate.to_path_buf()
}

/// 创建资产根下标准子目录(已存在则跳过)。
/// 适用于 `~/.ai-config/` 与 `<repo>/.ai-config/`（二者同构）。
pub fn ensure_user_asset_layout(root: &Utf8Path) -> Result<(), CoreError> {
    for sub in ASSET_SUBDIRS {
        std::fs::create_dir_all(root.join(sub).as_std_path())?;
    }
    Ok(())
}

/// 初始化资产根完整布局：skills / rules / agents 子目录 + `mcp.json`（全局与项目 `.ai-config` 共用）。
pub fn ensure_asset_layout(asset_root: &Utf8Path) -> Result<(), CoreError> {
    ensure_user_asset_layout(asset_root)?;
    let _ = mcp_json::ensure_mcp_json(asset_root);
    let _ = mcp_json::migrate_legacy_mcp_layout(asset_root);
    Ok(())
}

/// 由仓库根推导项目资产根 `<repo>/.ai-config/`。
pub fn project_asset_root(repo_root: &Utf8Path) -> Utf8PathBuf {
    resolve_project_roots(repo_root).1
}

/// 确保 `path` 的父目录存在(skill/rule/agent symlink 与平台 `mcp.json` 写入前调用)。
pub fn ensure_parent_dir(path: &Utf8Path) -> Result<(), CoreError> {
    if let Some(parent) = path.parent() {
        if !parent.as_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent.as_std_path()).map_err(CoreError::Io)?;
        }
    }
    Ok(())
}

/// 目录是否含用户内容(忽略 `.` 开头项)。
fn dir_has_user_content(dir: &Utf8Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir.as_std_path()) else {
        return false;
    };
    rd.flatten()
        .any(|e| e.file_name().to_str().is_some_and(|n| !n.starts_with('.')))
}

/// 全局资产根是否仍为空(需要种子拷贝)。
pub fn user_assets_need_seed(root: &Utf8Path) -> bool {
    !dir_has_user_content(&root.join("skills"))
        && !dir_has_user_content(&root.join("rules"))
        && !mcp_has_content(root)
        && !dir_has_user_content(&root.join("agents"))
}

fn mcp_has_content(root: &Utf8Path) -> bool {
    mcp_json::load_mcp_document(root)
        .ok()
        .flatten()
        .and_then(|doc| {
            doc.get("mcpServers")
                .and_then(|v| v.as_object())
                .map(|m| !m.is_empty())
        })
        .unwrap_or(false)
}

/// 收集可能的种子目录(按优先级):`AI_CONFIG_SEED`、安装包 Resources。
pub fn collect_seed_sources() -> Vec<Utf8PathBuf> {
    let mut out = Vec::new();

    if let Ok(seed) = std::env::var("AI_CONFIG_SEED") {
        if !seed.is_empty() {
            out.push(Utf8PathBuf::from(seed));
        }
    }

    if let Some(p) = bundled_seed_next_to_exe() {
        out.push(p);
    }

    out
}

/// 安装包内 Resources 或二进制旁的种子路径。
fn bundled_seed_next_to_exe() -> Option<Utf8PathBuf> {
    let exe = Utf8PathBuf::from_path_buf(std::env::current_exe().ok()?).ok()?;
    let macos_resources = exe.parent()?.parent()?.join("Resources");
    for name in BUNDLE_SEED_DIR_NAMES {
        let candidate = macos_resources.join(name);
        if candidate.join("skills").is_dir() {
            return Some(candidate);
        }
    }
    if let Some(dir) = exe.parent() {
        for name in BUNDLE_SEED_DIR_NAMES {
            let candidate = dir.join(name);
            if candidate.join("skills").is_dir() {
                return Some(candidate);
            }
        }
    }
    None
}

/// 将种子目录的 `skills|rules|mcp|agents` 合并拷贝到目标(已存在文件不覆盖)。
pub fn copy_seed_into(seed: &Utf8Path, dest: &Utf8Path) -> Result<(), CoreError> {
    for top in ["skills", "rules", "mcp", "agents"] {
        let src_top = seed.join(top);
        if !src_top.is_dir() {
            continue;
        }
        copy_dir_merge(&src_top, &dest.join(top))?;
    }
    Ok(())
}

fn copy_dir_merge(src: &Utf8Path, dest: &Utf8Path) -> Result<(), CoreError> {
    for entry in WalkDir::new(src.as_std_path()) {
        let entry = entry.map_err(|e| CoreError::Io(e.into()))?;
        let rel = entry.path().strip_prefix(src.as_std_path()).map_err(|e| {
            CoreError::TemplateRender {
                template: src.as_str().to_owned(),
                reason: format!("strip_prefix: {e}"),
                hint: "种子目录路径异常".to_owned(),
            }
        })?;
        let rel_utf8 = rel.to_string_lossy();
        let target = dest.join(rel_utf8.as_ref());
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(target.as_std_path())?;
            continue;
        }
        if target.exists() {
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent.as_std_path())?;
        }
        std::fs::copy(entry.path(), target.as_std_path()).map_err(CoreError::Io)?;
    }
    Ok(())
}

/// 初始化用户全局资产根:解析路径 → 建目录 → 必要时种子拷贝。
pub fn init_user_asset_root() -> Result<Utf8PathBuf, CoreError> {
    let root = effective_global_asset_root();
    ensure_user_asset_layout(&root)?;
    let _ = mcp_json::ensure_mcp_json(&root);
    let _ = mcp_json::migrate_legacy_mcp_layout(&root);
    if user_assets_need_seed(&root) {
        for seed in collect_seed_sources() {
            if seed.join("skills").is_dir() {
                tracing::info!("种子拷贝: {seed} → {root}");
                copy_seed_into(&seed, &root)?;
                break;
            }
        }
    }
    Ok(root)
}

fn effective_global_asset_root() -> Utf8PathBuf {
    if let Ok(env) = std::env::var("AI_CONFIG_ROOT") {
        if !env.is_empty() {
            return resolve_asset_root(Utf8Path::new(&env));
        }
    }
    user_home_asset_root()
}

/// 自动发现全局资产根(对外统一入口):默认 `~/.ai-config` 并保证目录存在。
pub fn discover_global_asset_root() -> Utf8PathBuf {
    init_user_asset_root().unwrap_or_else(|e| {
        tracing::warn!("init ~/.ai-config 失败: {e}, 仍使用默认路径");
        let root = user_home_asset_root();
        let _ = ensure_user_asset_layout(&root);
        root
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn ensure_creates_standard_subdirs() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        ensure_user_asset_layout(&root).unwrap();
        for sub in ASSET_SUBDIRS {
            assert!(root.join(sub).is_dir(), "missing {sub}");
        }
    }

    #[test]
    fn ensure_parent_dir_creates_nested_path() {
        let tmp = TempDir::new().unwrap();
        let path = Utf8PathBuf::from_path_buf(tmp.path().join(".cursor/agents/foo.md")).unwrap();
        assert!(!path.parent().unwrap().exists());
        ensure_parent_dir(&path).unwrap();
        assert!(path.parent().unwrap().is_dir());
    }

    #[test]
    fn user_assets_need_seed_when_empty() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        ensure_user_asset_layout(&root).unwrap();
        assert!(user_assets_need_seed(&root));
        fs::create_dir_all(root.join("skills/foo")).unwrap();
        fs::write(root.join("skills/foo/SKILL.md"), "# x").unwrap();
        assert!(!user_assets_need_seed(&root));
    }

    #[test]
    fn copy_seed_merge_skips_existing() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let seed = base.join("seed");
        let dest = base.join("dest");
        fs::create_dir_all(seed.join("skills/a")).unwrap();
        fs::write(seed.join("skills/a/SKILL.md"), "SEED").unwrap();
        fs::create_dir_all(dest.join("skills/b")).unwrap();
        fs::write(dest.join("skills/b/SKILL.md"), "KEEP").unwrap();
        let seed_u = Utf8PathBuf::from_path_buf(seed).unwrap();
        let dest_u = Utf8PathBuf::from_path_buf(dest.clone()).unwrap();
        copy_seed_into(&seed_u, &dest_u).unwrap();
        assert_eq!(
            fs::read_to_string(dest.join("skills/a/SKILL.md")).unwrap(),
            "SEED"
        );
        assert_eq!(
            fs::read_to_string(dest.join("skills/b/SKILL.md")).unwrap(),
            "KEEP"
        );
    }

    #[test]
    fn init_uses_ai_config_root_override() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        std::env::set_var("AI_CONFIG_ROOT", root.as_str());
        let got = init_user_asset_root().unwrap();
        assert_eq!(got, root);
        assert!(root.join("skills").is_dir());
        std::env::remove_var("AI_CONFIG_ROOT");
    }

    #[test]
    fn ensure_asset_layout_matches_global_shape() {
        let tmp = TempDir::new().unwrap();
        let asset = Utf8PathBuf::from_path_buf(tmp.path().join(".ai-config")).unwrap();
        ensure_asset_layout(&asset).unwrap();
        for sub in ASSET_SUBDIRS {
            assert!(asset.join(sub).is_dir(), "missing {sub}");
        }
        assert!(asset.join("mcp.json").is_file());
    }

    #[test]
    fn project_asset_root_from_repo() {
        let tmp = TempDir::new().unwrap();
        let repo = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        assert_eq!(project_asset_root(&repo), repo.join(".ai-config"));
    }

    #[test]
    fn resolve_asset_root_repo_falls_back_to_user_home() {
        let tmp = TempDir::new().unwrap();
        let repo = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(repo.join("crates/ai-config-core")).unwrap();
        assert_eq!(resolve_asset_root(&repo), user_home_asset_root());
    }

    #[test]
    fn resolve_project_roots_from_repo() {
        let tmp = TempDir::new().unwrap();
        let repo = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(repo.join(".ai-config/skills")).unwrap();
        let (r, a) = resolve_project_roots(&repo);
        assert_eq!(r, repo);
        assert_eq!(a, repo.join(".ai-config"));
    }

    #[test]
    fn resolve_project_roots_from_dot_ai_config() {
        let tmp = TempDir::new().unwrap();
        let asset = Utf8PathBuf::from_path_buf(tmp.path().join(".ai-config")).unwrap();
        fs::create_dir_all(asset.join("skills")).unwrap();
        let (r, a) = resolve_project_roots(&asset);
        assert_eq!(a, asset);
        assert_eq!(
            r,
            Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap()
        );
    }
}
