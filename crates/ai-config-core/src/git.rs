//! `~/.agent-manager` 目录的 Git 版本控制（本地仓库 + 可选远程同步）。
//!
//! 通过系统 `git` 命令操作；未安装 git 时返回 `git_available: false` 而不 panic。

use std::process::Command;

use camino::Utf8Path;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 写入资产根时的默认 `.gitignore` 内容（本地状态与密钥不入库）。
pub const DEFAULT_GITIGNORE: &str = r#"# agent-manager — 本地状态与密钥不入库
*.db
*.db-wal
*.db-shm
secrets.env
daemon.*
.DS_Store
"#;

/// 默认分支名（无远程时仅本地提交）。
pub const DEFAULT_BRANCH: &str = "main";

#[derive(Debug, Error)]
pub enum GitError {
    #[error("未检测到 git 命令，请安装 Git 后重试")]
    GitNotAvailable,

    #[error("git 命令失败: {cmd}\n{stderr}")]
    CommandFailed { cmd: String, stderr: String },

    #[error("路径非法: {0}")]
    InvalidPath(String),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
}

/// 启动时 `ensure_repo` 的结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitEnsureOutcome {
    pub git_available: bool,
    pub was_repo: bool,
    pub just_initialized: bool,
    pub initial_commit: bool,
}

/// 供 GUI / CLI 展示的仓库状态。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct GitRepoStatus {
    pub git_available: bool,
    pub is_repo: bool,
    pub just_initialized: bool,
    pub branch: Option<String>,
    pub dirty: bool,
    pub dirty_count: u32,
    pub has_remote: bool,
    pub remote_url: Option<String>,
    pub ahead: u32,
    pub behind: u32,
}

/// 同步操作摘要。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitSyncOutcome {
    pub committed: bool,
    pub pulled: bool,
    pub pushed: bool,
    pub message: String,
}

/// Git 同步配置（持久化在 store，此处为传输结构）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitSyncConfig {
    #[serde(default)]
    pub remote_url: Option<String>,
    #[serde(default = "default_branch")]
    pub branch: String,
}

fn default_branch() -> String {
    DEFAULT_BRANCH.to_string()
}

impl Default for GitSyncConfig {
    fn default() -> Self {
        Self {
            remote_url: None,
            branch: DEFAULT_BRANCH.to_string(),
        }
    }
}

/// 系统是否可执行 `git`。
pub fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn is_git_dir(root: &Utf8Path) -> bool {
    root.join(".git").exists()
}

fn run_git(root: &Utf8Path, args: &[&str]) -> Result<String, GitError> {
    if !git_available() {
        return Err(GitError::GitNotAvailable);
    }
    let output = Command::new("git")
        .args(args)
        .current_dir(root.as_std_path())
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(GitError::CommandFailed {
            cmd: format!("git {}", args.join(" ")),
            stderr,
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_git_ok(root: &Utf8Path, args: &[&str]) -> Result<(), GitError> {
    run_git(root, args).map(|_| ())
}

fn write_default_gitignore(root: &Utf8Path) -> Result<(), GitError> {
    let path = root.join(".gitignore");
    if path.exists() {
        return Ok(());
    }
    std::fs::write(path.as_std_path(), DEFAULT_GITIGNORE)?;
    Ok(())
}

fn has_any_commit(root: &Utf8Path) -> bool {
    run_git(root, &["rev-parse", "HEAD"]).is_ok()
}

/// 确保资产根为 Git 仓库；不存在则 `git init` 并写入 `.gitignore`。
///
/// 若目录已有用户内容且无提交，会创建初始提交以便本地版本追踪。
pub fn ensure_repo(root: &Utf8Path, default_branch: &str) -> Result<GitEnsureOutcome, GitError> {
    if !git_available() {
        return Ok(GitEnsureOutcome {
            git_available: false,
            was_repo: false,
            just_initialized: false,
            initial_commit: false,
        });
    }

    let was_repo = is_git_dir(root);
    let mut just_initialized = false;

    if !was_repo {
        let branch = if default_branch.is_empty() {
            DEFAULT_BRANCH
        } else {
            default_branch
        };
        run_git_ok(root, &["init", "-b", branch])?;
        write_default_gitignore(root)?;
        just_initialized = true;
    }

    let initial_commit = if just_initialized || !has_any_commit(root) {
        if repo_has_trackable_files(root) {
            run_git_ok(root, &["add", "-A"])?;
            // 可能全是 ignore 的文件；允许空提交失败
            run_git_ok(root, &["commit", "-m", "chore: agent-manager 初始提交"]).is_ok()
        } else {
            false
        }
    } else {
        false
    };

    Ok(GitEnsureOutcome {
        git_available: true,
        was_repo,
        just_initialized,
        initial_commit,
    })
}

fn repo_has_trackable_files(root: &Utf8Path) -> bool {
    run_git(root, &["status", "--porcelain"])
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

/// 读取当前分支名。
pub fn current_branch(root: &Utf8Path) -> Result<Option<String>, GitError> {
    if !is_git_dir(root) {
        return Ok(None);
    }
    match run_git(root, &["rev-parse", "--abbrev-ref", "HEAD"]) {
        Ok(b) if b == "HEAD" => Ok(None),
        Ok(b) => Ok(Some(b)),
        Err(GitError::CommandFailed { .. }) if !has_any_commit(root) => Ok(None),
        Err(e) => Err(e),
    }
}

/// 工作区是否有未提交变更。
pub fn is_dirty(root: &Utf8Path) -> Result<(bool, u32), GitError> {
    if !is_git_dir(root) {
        return Ok((false, 0));
    }
    let out = run_git(root, &["status", "--porcelain"])?;
    if out.is_empty() {
        return Ok((false, 0));
    }
    let count = out.lines().filter(|l| !l.is_empty()).count() as u32;
    Ok((true, count))
}

/// 读取 `origin` 远程 URL（无则 `None`）。
pub fn remote_url(root: &Utf8Path) -> Result<Option<String>, GitError> {
    if !is_git_dir(root) {
        return Ok(None);
    }
    match run_git(root, &["remote", "get-url", "origin"]) {
        Ok(url) if !url.is_empty() => Ok(Some(url)),
        Ok(_) => Ok(None),
        Err(GitError::CommandFailed { stderr, .. }) if stderr.contains("No such remote") => {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// 设置或更新 `origin` 远程；`url` 为空则移除远程。
pub fn apply_remote(root: &Utf8Path, url: Option<&str>) -> Result<(), GitError> {
    if !is_git_dir(root) {
        return Err(GitError::InvalidPath(format!(
            "{} 不是 Git 仓库",
            root.as_str()
        )));
    }
    match url.map(str::trim).filter(|s| !s.is_empty()) {
        Some(url) => {
            if remote_url(root)?.is_some() {
                run_git_ok(root, &["remote", "set-url", "origin", url])?;
            } else {
                run_git_ok(root, &["remote", "add", "origin", url])?;
            }
        }
        None => {
            let _ = run_git(root, &["remote", "remove", "origin"]);
        }
    }
    Ok(())
}

/// 与 upstream 的 ahead/behind 计数（无 upstream 则均为 0）。
pub fn ahead_behind(root: &Utf8Path) -> Result<(u32, u32), GitError> {
    if !is_git_dir(root) || remote_url(root)?.is_none() {
        return Ok((0, 0));
    }
    let out = run_git(
        root,
        &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
    );
    match out {
        Ok(s) => {
            let mut parts = s.split_whitespace();
            let behind = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            let ahead = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            Ok((ahead, behind))
        }
        Err(GitError::CommandFailed { .. }) => Ok((0, 0)),
        Err(e) => Err(e),
    }
}

/// 汇总仓库状态。
pub fn repo_status(root: &Utf8Path, just_initialized: bool) -> GitRepoStatus {
    if !git_available() {
        return GitRepoStatus {
            git_available: false,
            ..Default::default()
        };
    }
    if !is_git_dir(root) {
        return GitRepoStatus {
            git_available: true,
            is_repo: false,
            just_initialized,
            ..Default::default()
        };
    }

    let branch = current_branch(root).ok().flatten();
    let (dirty, dirty_count) = is_dirty(root).unwrap_or((false, 0));
    let remote = remote_url(root).ok().flatten();
    let has_remote = remote.is_some();
    let (ahead, behind) = ahead_behind(root).unwrap_or((0, 0));

    GitRepoStatus {
        git_available: true,
        is_repo: true,
        just_initialized,
        branch,
        dirty,
        dirty_count,
        has_remote,
        remote_url: remote,
        ahead,
        behind,
    }
}

/// 提交全部变更（有变更时）。
pub fn commit_all(root: &Utf8Path, message: &str) -> Result<bool, GitError> {
    if !is_git_dir(root) {
        return Err(GitError::InvalidPath(format!(
            "{} 不是 Git 仓库",
            root.as_str()
        )));
    }
    let (dirty, _) = is_dirty(root)?;
    if !dirty {
        return Ok(false);
    }
    run_git_ok(root, &["add", "-A"])?;
    run_git_ok(root, &["commit", "-m", message])?;
    Ok(true)
}

/// 从远程拉取（需已配置 origin 与 upstream 或显式分支）。
pub fn pull(root: &Utf8Path, branch: &str) -> Result<bool, GitError> {
    if remote_url(root)?.is_none() {
        return Ok(false);
    }
    run_git_ok(root, &["pull", "--rebase", "origin", branch])?;
    Ok(true)
}

/// 推送到远程。
pub fn push(root: &Utf8Path, branch: &str, set_upstream: bool) -> Result<bool, GitError> {
    if remote_url(root)?.is_none() {
        return Ok(false);
    }
    if set_upstream {
        run_git_ok(root, &["push", "-u", "origin", branch])?;
    } else {
        run_git_ok(root, &["push", "origin", branch])?;
    }
    Ok(true)
}

/// 完整同步：提交 → pull → push。
pub fn sync_repo(
    root: &Utf8Path,
    config: &GitSyncConfig,
    commit_message: &str,
) -> Result<GitSyncOutcome, GitError> {
    let committed = commit_all(root, commit_message)?;
    let branch = if config.branch.is_empty() {
        DEFAULT_BRANCH
    } else {
        config.branch.as_str()
    };

    let has_remote = config
        .remote_url
        .as_ref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
        || remote_url(root)?.is_some();

    if !has_remote {
        return Ok(GitSyncOutcome {
            committed,
            pulled: false,
            pushed: false,
            message: if committed {
                "已提交到本地 Git 仓库（未配置远程）".to_string()
            } else {
                "工作区干净，无远程配置".to_string()
            },
        });
    }

    let pulled = if has_remote {
        pull(root, branch)?
    } else {
        false
    };
    let pushed = if has_remote {
        push(root, branch, false)?
    } else {
        false
    };

    let message = match (committed, pulled, pushed) {
        (true, true, true) => "已提交、拉取并推送".to_string(),
        (true, false, true) => "已提交并推送".to_string(),
        (true, true, false) => "已提交并拉取（推送跳过或失败）".to_string(),
        (true, false, false) => "已提交到本地".to_string(),
        (false, true, true) => "已拉取并推送".to_string(),
        (false, true, false) => "已拉取".to_string(),
        (false, false, true) => "已推送".to_string(),
        (false, false, false) => "已是最新，无需同步".to_string(),
    };

    Ok(GitSyncOutcome {
        committed,
        pulled,
        pushed,
        message,
    })
}

/// 资产根路径（供 GUI 展示）。
pub fn asset_root_display(root: &Utf8Path) -> String {
    root.as_str().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;
    use tempfile::tempdir;

    fn skip_without_git() -> bool {
        !git_available()
    }

    #[test]
    fn ensure_repo_initializes_and_commits() {
        if skip_without_git() {
            return;
        }
        let dir = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        std::fs::create_dir_all(root.join("skills").as_std_path()).unwrap();
        std::fs::write(root.join("skills/demo.md").as_std_path(), "# demo").unwrap();

        let out = ensure_repo(&root, "main").expect("ensure");
        assert!(out.git_available);
        assert!(out.just_initialized);
        assert!(is_git_dir(&root));
        assert!(root.join(".gitignore").exists());
    }

    #[test]
    fn apply_remote_add_and_remove() {
        if skip_without_git() {
            return;
        }
        let dir = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        ensure_repo(&root, "main").unwrap();

        apply_remote(&root, Some("https://example.com/repo.git")).unwrap();
        assert_eq!(
            remote_url(&root).unwrap().as_deref(),
            Some("https://example.com/repo.git")
        );

        apply_remote(&root, None).unwrap();
        assert!(remote_url(&root).unwrap().is_none());
    }

    #[test]
    fn commit_all_skips_when_clean() {
        if skip_without_git() {
            return;
        }
        let dir = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        ensure_repo(&root, "main").unwrap();
        let committed = commit_all(&root, "test").unwrap();
        assert!(!committed);
    }
}
