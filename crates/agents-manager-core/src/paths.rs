//! 用户全局资产根:`~/.agents-manager/`(skills / rules / mcp / agents)。
//!
//! - 默认始终读写 `~/.agents-manager/`;不存在则创建子目录。
//! - 首次为空时,可从 `AGENTS_MANAGER_SEED` 或安装包 Resources 合并拷贝(不覆盖已有文件)。
//! - 项目覆盖仍在 `<project>/.agents-manager/`(结构相同,与全局合并)。
//! - `AGENTS_MANAGER_ROOT` 可覆盖全局根(开发/测试);指向 agents-manager 仓库根时回退 `~/.agents-manager`。

use camino::{Utf8Path, Utf8PathBuf};
use walkdir::WalkDir;

use crate::error::CoreError;
use crate::mcp_json::{self};

/// 用户主目录下的全局资产目录名。
pub const USER_ASSET_DIR_NAME: &str = ".agents-manager";

/// 安装包 Resources 内种子目录名(构建时由 `~/.agents-manager` 或 `AGENTS_MANAGER_SEED` 打入)。
pub const BUNDLE_SEED_DIR_NAMES: &[&str] = &[".agents-manager", "seed"];

/// 子目录(相对资产根)。
pub const ASSET_SUBDIRS: &[&str] = &["skills", "rules", "agents", "commands", "hooks"];

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

/// 默认用户全局资产根 `~/.agents-manager`。
pub fn user_home_asset_root() -> Utf8PathBuf {
    home_dir().join(USER_ASSET_DIR_NAME)
}

/// 是否为 agents-manager 工程仓库根(含 `crates/agents-manager-core`)。
pub fn is_agents_manager_repo(p: &Utf8Path) -> bool {
    p.join("crates/agents-manager-core").is_dir()
}

/// 将注册表或用户输入的路径规范为 `(仓库根, 资产根 …/.agents-manager/)`。
///
/// - 仓库根 → 资产根 = `<repo>/.agents-manager/`
/// - 用户直接选 `.agents-manager/` → 仓库根 = 父目录
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

/// install / sync 作用域解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncRoots {
    /// 仓库根（全局 install 时为 `$HOME` 或资产根父目录）。
    pub repo_root: Utf8PathBuf,
    /// 扫描资产用的根（`<repo>/.agents-manager/` 或 `~/.agents-manager/`）。
    pub asset_root: Utf8PathBuf,
    /// `scan_with_override` 的 default 侧（项目模式为 `~/.agents-manager`，全局为 `asset_root`）。
    pub global_default: Utf8PathBuf,
    /// IDE 平台配置下发根（`$HOME` 或 `<repo>`）。
    pub deploy_base: Utf8PathBuf,
}

/// 解析 CLI `--root` / `AGENTS_MANAGER_ROOT` 对应的 install/sync 作用域。
///
/// - 全局：`~/.agents-manager` 或纯资产根 → 下发到 `$HOME`，合并源为自身。
/// - 项目：仓库根 → 资产 `<repo>/.agents-manager`，default 合并 `~/.agents-manager`，下发到 `<repo>`。
pub fn resolve_sync_roots(candidate: &Utf8Path) -> SyncRoots {
    let user_global = discover_global_asset_root_read_only();
    let normalized = resolve_asset_root(candidate);
    let (repo_root, asset_root) = resolve_project_roots(&normalized);

    let is_project =
        asset_root != repo_root && !is_asset_root(&repo_root) && asset_root != user_global;
    let global_default = if is_project {
        user_global
    } else {
        asset_root.clone()
    };

    let deploy_base = if is_project {
        project_deploy_base(&repo_root)
    } else if repo_root == asset_root && is_asset_root(&asset_root) {
        global_deploy_base()
    } else {
        project_deploy_base(&repo_root)
    };

    SyncRoots {
        repo_root,
        asset_root,
        global_default,
        deploy_base,
    }
}

/// 下发根是否为项目作用域（非 `$HOME` 全局）。
pub fn is_project_deploy_base(deploy_base: &Utf8Path) -> bool {
    deploy_base != home_dir()
}

/// 候选路径是否已是「资产根」(直接含 `skills/`)。
pub fn is_asset_root(p: &Utf8Path) -> bool {
    p.join("skills").is_dir()
}

/// 将 CLI/GUI 传入路径规范为资产根。
///
/// - 已是资产根 → 原样
/// - agents-manager 仓库根 → `~/.agents-manager`(资产已迁出仓库)
pub fn resolve_asset_root(candidate: &Utf8Path) -> Utf8PathBuf {
    if is_asset_root(candidate) {
        return candidate.to_path_buf();
    }
    if is_agents_manager_repo(candidate) {
        return user_home_asset_root();
    }
    candidate.to_path_buf()
}

/// 创建资产根下标准子目录(已存在则跳过)。
/// 适用于 `~/.agents-manager/` 与 `<repo>/.agents-manager/`（二者同构）。
pub fn ensure_user_asset_layout(root: &Utf8Path) -> Result<(), CoreError> {
    for sub in ASSET_SUBDIRS {
        std::fs::create_dir_all(root.join(sub).as_std_path())?;
    }
    Ok(())
}

/// 初始化资产根标准目录（全局与项目 `.agents-manager` 共用）。
///
/// 不会创建或迁移 MCP 配置；这些操作必须由显式的 source-first 流程执行。
pub fn ensure_asset_layout(asset_root: &Utf8Path) -> Result<(), CoreError> {
    ensure_user_asset_layout(asset_root)?;
    Ok(())
}

/// 由仓库根推导项目资产根 `<repo>/.agents-manager/`。
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
        && !dir_has_user_content(&root.join("commands"))
        && !dir_has_user_content(&root.join("hooks"))
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

/// 收集可能的种子目录(按优先级):`AGENTS_MANAGER_SEED`、安装包 Resources。
pub fn collect_seed_sources() -> Vec<Utf8PathBuf> {
    let mut out = Vec::new();

    if let Ok(seed) = std::env::var("AGENTS_MANAGER_SEED") {
        if !seed.is_empty() {
            out.push(Utf8PathBuf::from(seed));
        }
    }

    if let Some(p) = bundled_seed_next_to_exe() {
        out.push(p);
    }

    if let Some(p) = dev_repo_asset_seed() {
        out.push(p);
    }

    out
}

/// 开发态:当前工作目录或二进制旁的 agents-manager 仓库 `.agents-manager/`。
fn dev_repo_asset_seed() -> Option<Utf8PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.to_path_buf());
            if let Some(grand) = parent.parent() {
                candidates.push(grand.to_path_buf());
            }
        }
    }
    for base in candidates {
        let utf = Utf8PathBuf::from_path_buf(base).ok()?;
        if !is_agents_manager_repo(&utf) {
            continue;
        }
        let asset = utf.join(USER_ASSET_DIR_NAME);
        if asset.join("hooks.json").is_file()
            || asset.join("hooks").join("hooks.json").is_file()
            || asset.join("skills").is_dir()
        {
            return Some(asset);
        }
    }
    None
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
    for top in ["skills", "rules", "mcp", "agents", "commands", "hooks"] {
        let src_top = seed.join(top);
        if !src_top.is_dir() {
            continue;
        }
        copy_dir_merge(&src_top, &dest.join(top))?;
    }
    let seed_manifest = seed.join("hooks.json");
    let dest_manifest = dest.join("hooks.json");
    if seed_manifest.is_file() && !dest_manifest.exists() {
        if let Some(parent) = dest_manifest.parent() {
            std::fs::create_dir_all(parent.as_std_path())?;
        }
        std::fs::copy(seed_manifest.as_std_path(), dest_manifest.as_std_path())
            .map_err(CoreError::Io)?;
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
    if user_assets_need_seed(&root) {
        for seed in collect_seed_sources() {
            if seed.join("skills").is_dir() {
                tracing::info!("种子拷贝: {seed} → {root}");
                copy_seed_into(&seed, &root)?;
                break;
            }
        }
    }
    seed_hooks_if_missing(&root)?;
    Ok(root)
}

/// `~/.agents-manager/hooks.json` 缺失时,从种子目录合并 hooks 清单与脚本。
fn seed_hooks_if_missing(root: &Utf8Path) -> Result<(), CoreError> {
    if root.join("hooks.json").is_file() {
        return Ok(());
    }
    for seed in collect_seed_sources() {
        let seed_manifest = seed.join("hooks.json");
        if !seed_manifest.is_file() {
            continue;
        }
        tracing::info!(
            "hooks 种子合并: {seed_manifest} → {}",
            root.join("hooks.json")
        );
        if let Some(parent) = root.parent() {
            std::fs::create_dir_all(parent.as_std_path()).map_err(CoreError::Io)?;
        }
        std::fs::copy(
            seed_manifest.as_std_path(),
            root.join("hooks.json").as_std_path(),
        )
        .map_err(CoreError::Io)?;
        let seed_hooks = seed.join("hooks");
        if seed_hooks.is_dir() {
            copy_dir_merge(&seed_hooks, &root.join("hooks"))?;
        }
        break;
    }
    Ok(())
}

fn effective_global_asset_root() -> Utf8PathBuf {
    if let Ok(env) = std::env::var("AGENTS_MANAGER_ROOT") {
        if !env.is_empty() {
            return resolve_asset_root(Utf8Path::new(&env));
        }
    }
    user_home_asset_root()
}

/// 只读发现全局资产根：只解析环境与路径，绝不创建目录、迁移或播种资产。
pub fn discover_global_asset_root_read_only() -> Utf8PathBuf {
    effective_global_asset_root()
}

/// 自动发现全局资产根(对外统一入口):默认 `~/.agents-manager` 并保证目录存在。
pub fn discover_global_asset_root() -> Utf8PathBuf {
    init_user_asset_root().unwrap_or_else(|e| {
        tracing::warn!("init ~/.agents-manager 失败: {e}, 仍使用默认路径");
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
    fn seed_hooks_if_missing_merges_when_skills_already_exist() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let seed = base.join("seed");
        let dest = base.join("dest");
        fs::create_dir_all(seed.join("hooks")).unwrap();
        fs::write(
            seed.join("hooks.json"),
            r#"{"version":1,"hooks":{"afterShellExecution":[{"command":"./hooks/a.sh"}]}}"#,
        )
        .unwrap();
        fs::write(seed.join("hooks/a.sh"), "#!/bin/sh\n").unwrap();
        fs::create_dir_all(dest.join("skills/foo")).unwrap();
        fs::write(dest.join("skills/foo/SKILL.md"), "# x").unwrap();
        ensure_user_asset_layout(&Utf8PathBuf::from_path_buf(dest.clone()).unwrap()).unwrap();
        assert!(!user_assets_need_seed(
            &Utf8PathBuf::from_path_buf(dest.clone()).unwrap()
        ));
        let seed_u = Utf8PathBuf::from_path_buf(seed).unwrap();
        let dest_u = Utf8PathBuf::from_path_buf(dest.clone()).unwrap();
        std::env::set_var("AGENTS_MANAGER_SEED", seed_u.as_str());
        seed_hooks_if_missing(&dest_u).unwrap();
        std::env::remove_var("AGENTS_MANAGER_SEED");
        assert!(dest.join("hooks.json").is_file());
        assert!(dest.join("hooks/a.sh").is_file());
        assert!(dest.join("skills/foo/SKILL.md").is_file());
    }

    #[test]
    fn init_uses_agents_manager_root_override() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let _root_guard = crate::test_env::EnvGuard::set("AGENTS_MANAGER_ROOT", root.as_str());
        let got = init_user_asset_root().unwrap();
        assert_eq!(got, root);
        assert!(root.join("skills").is_dir());
    }

    #[test]
    fn ensure_asset_layout_matches_global_shape() {
        let tmp = TempDir::new().unwrap();
        let asset = Utf8PathBuf::from_path_buf(tmp.path().join(".agents-manager")).unwrap();
        ensure_asset_layout(&asset).unwrap();
        for sub in ASSET_SUBDIRS {
            assert!(asset.join(sub).is_dir(), "missing {sub}");
        }
        assert!(
            !asset.join("mcp.json").exists(),
            "ordinary layout setup must not create a legacy MCP aggregate"
        );
    }

    #[test]
    fn project_asset_root_from_repo() {
        let tmp = TempDir::new().unwrap();
        let repo = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        assert_eq!(project_asset_root(&repo), repo.join(".agents-manager"));
    }

    #[test]
    fn resolve_asset_root_repo_falls_back_to_user_home() {
        let tmp = TempDir::new().unwrap();
        let repo = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(repo.join("crates/agents-manager-core")).unwrap();
        assert_eq!(resolve_asset_root(&repo), user_home_asset_root());
    }

    #[test]
    fn resolve_project_roots_from_repo() {
        let tmp = TempDir::new().unwrap();
        let repo = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(repo.join(".agents-manager/skills")).unwrap();
        let (r, a) = resolve_project_roots(&repo);
        assert_eq!(r, repo);
        assert_eq!(a, repo.join(".agents-manager"));
    }

    #[test]
    fn resolve_project_roots_from_dot_agents_manager() {
        let tmp = TempDir::new().unwrap();
        let asset = Utf8PathBuf::from_path_buf(tmp.path().join(".agents-manager")).unwrap();
        fs::create_dir_all(asset.join("skills")).unwrap();
        let (r, a) = resolve_project_roots(&asset);
        assert_eq!(a, asset);
        assert_eq!(
            r,
            Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap()
        );
    }

    #[test]
    fn resolve_sync_roots_project_repo() {
        let tmp = TempDir::new().unwrap();
        let home = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let _env_guard = crate::test_env::EnvGuard::set_many(&[
            ("HOME", Some(home.as_str())),
            ("AGENTS_MANAGER_ROOT", None),
        ]);
        fs::create_dir_all(home.join(".agents-manager/skills")).unwrap();
        let repo = home.join("myproj");
        fs::create_dir_all(repo.join(".agents-manager/skills")).unwrap();
        let roots = resolve_sync_roots(&repo);
        assert_eq!(roots.repo_root, repo);
        assert_eq!(roots.asset_root, repo.join(".agents-manager"));
        assert_eq!(roots.deploy_base, repo);
        assert_ne!(roots.global_default, roots.asset_root);
        assert!(roots.global_default.ends_with(".agents-manager"));
    }

    #[test]
    fn resolve_sync_roots_global_asset_root() {
        let tmp = TempDir::new().unwrap();
        let home = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let _env_guard = crate::test_env::EnvGuard::set_many(&[
            ("HOME", Some(home.as_str())),
            ("AGENTS_MANAGER_ROOT", None),
        ]);
        let asset = home.join(".agents-manager");
        fs::create_dir_all(asset.join("skills/foo")).unwrap();
        let roots = resolve_sync_roots(&asset);
        assert_eq!(roots.repo_root, home);
        assert_eq!(roots.asset_root, asset);
        assert_eq!(roots.global_default, asset);
        assert_eq!(roots.deploy_base, home);
    }

    #[test]
    fn resolve_sync_roots_does_not_initialize_global_asset_root() {
        let tmp = TempDir::new().unwrap();
        let home = Utf8PathBuf::from_path_buf(tmp.path().join("home")).unwrap();
        let repo = Utf8PathBuf::from_path_buf(tmp.path().join("repo")).unwrap();
        fs::create_dir_all(repo.as_std_path()).unwrap();
        let _env_guard = crate::test_env::EnvGuard::set_many(&[
            ("HOME", Some(home.as_str())),
            ("AGENTS_MANAGER_ROOT", None),
        ]);

        let _ = resolve_sync_roots(&repo);

        assert!(
            !home.join(".agents-manager").exists(),
            "scope resolution is read-only and must not seed the global asset root"
        );
    }
}
