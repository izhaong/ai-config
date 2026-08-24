//! agents-manager 资产目录文件监听：`notify` + `notify-debouncer-mini`（200ms）。
//!
//! 监听每个资产根下的 `skills/`、`rules/`、`agents/` 与 `mcp.json`。

use std::sync::Arc;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, Debouncer};
use thiserror::Error;

const DEBOUNCE_MS: u64 = 200;

/// 需要监听的资产根目录列表（`~/.agents` 或项目 `.agents-manager` 等价路径）。
#[derive(Debug, Clone, Default)]
pub struct WatchRoots {
    pub asset_roots: Vec<Utf8PathBuf>,
}

/// 监听启动失败。
#[derive(Debug, Error)]
pub enum WatcherError {
    #[error("notify 初始化失败: {0}")]
    Notify(#[from] notify::Error),
    #[error("debouncer 初始化失败: {0}")]
    Debouncer(String),
}

/// 持有 debouncer 生命周期；`Drop` 时自动停止底层 watcher。
pub struct WatcherHandle {
    _debouncer: Debouncer<notify::RecommendedWatcher>,
}

/// 启动去抖文件监听；`on_change` 在 200ms 静默后于 notify 回调线程触发。
pub fn start_debounced(
    roots: WatchRoots,
    on_change: impl Fn() + Send + Sync + 'static,
) -> Result<WatcherHandle, WatcherError> {
    let callback = Arc::new(on_change);
    let mut debouncer = new_debouncer(Duration::from_millis(DEBOUNCE_MS), move |res| match res {
        Ok(_events) => callback(),
        Err(e) => tracing::warn!("watcher debounce 错误: {e}"),
    })
    .map_err(|e| WatcherError::Debouncer(e.to_string()))?;

    register_roots(&mut debouncer, &roots)?;

    Ok(WatcherHandle {
        _debouncer: debouncer,
    })
}

fn register_roots(
    debouncer: &mut Debouncer<notify::RecommendedWatcher>,
    roots: &WatchRoots,
) -> Result<(), WatcherError> {
    let mut seen = std::collections::HashSet::new();
    for root in &roots.asset_roots {
        if !seen.insert(root.as_str()) {
            continue;
        }
        register_asset_root(debouncer, root)?;
    }
    Ok(())
}

fn register_asset_root(
    debouncer: &mut Debouncer<notify::RecommendedWatcher>,
    root: &Utf8Path,
) -> Result<(), WatcherError> {
    watch_subdir(debouncer, root, "skills")?;
    watch_subdir(debouncer, root, "rules")?;
    watch_subdir(debouncer, root, "agents")?;
    watch_mcp_json(debouncer, root)?;
    Ok(())
}

fn watch_subdir(
    debouncer: &mut Debouncer<notify::RecommendedWatcher>,
    root: &Utf8Path,
    name: &str,
) -> Result<(), WatcherError> {
    let dir = root.join(name);
    if !dir.is_dir() {
        return Ok(());
    }
    debouncer
        .watcher()
        .watch(dir.as_std_path(), RecursiveMode::Recursive)?;
    tracing::debug!("watcher: 监听 {dir}");
    Ok(())
}

fn watch_mcp_json(
    debouncer: &mut Debouncer<notify::RecommendedWatcher>,
    root: &Utf8Path,
) -> Result<(), WatcherError> {
    if !root.is_dir() {
        return Ok(());
    }
    // 监听资产根目录本身，覆盖 mcp.json 原地编辑与编辑器原子替换(rename)。
    debouncer
        .watcher()
        .watch(root.as_std_path(), RecursiveMode::NonRecursive)?;
    tracing::debug!("watcher: 监听 {root} (mcp.json 及根级变更)");
    Ok(())
}

/// 资产根去重合并。
pub fn dedupe_roots(mut roots: Vec<Utf8PathBuf>) -> Vec<Utf8PathBuf> {
    let mut seen = std::collections::HashSet::new();
    roots.retain(|p| seen.insert(p.clone()));
    roots
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Instant;

    #[test]
    fn debounced_callback_fires_on_mcp_json_write() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).expect("utf8 path");
        fs::write(root.join("mcp.json"), r#"{"mcpServers":{}}"#).expect("mcp.json");

        let hits = Arc::new(AtomicUsize::new(0));
        let hits_cb = Arc::clone(&hits);
        let _handle = start_debounced(
            WatchRoots {
                asset_roots: vec![root.clone()],
            },
            move || {
                hits_cb.fetch_add(1, Ordering::SeqCst);
            },
        )
        .expect("start watcher");

        fs::write(
            root.join("mcp.json"),
            r#"{"mcpServers":{"foo":{"command":"uvx"}}}"#,
        )
        .expect("rewrite mcp");

        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if hits.load(Ordering::SeqCst) >= 1 {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!("mcp.json change did not fire within 3s");
    }

    #[test]
    fn debounced_callback_fires_on_agent_create() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).expect("utf8 path");
        fs::create_dir_all(root.join("agents")).expect("agents dir");

        let hits = Arc::new(AtomicUsize::new(0));
        let hits_cb = Arc::clone(&hits);
        let _handle = start_debounced(
            WatchRoots {
                asset_roots: vec![root.clone()],
            },
            move || {
                hits_cb.fetch_add(1, Ordering::SeqCst);
            },
        )
        .expect("start watcher");

        let agent = root.join("agents").join("foo.md");
        fs::write(&agent, "# test\n").expect("write agent");

        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if hits.load(Ordering::SeqCst) >= 1 {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!("debounced callback did not fire within 3s");
    }
}
