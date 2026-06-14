//! ai-config 本机状态记录(SQLite)。
//!
//! ## 模块边界(对齐 ARCHITECTURE §3 / PRD §8.2 #2 事实源唯一)
//!
//! - **`Store`** 持有 `rusqlite::Connection`(同步阻塞 IO,Tauri command 端用 `tokio::task::spawn_blocking` 包裹)
//! - **`Store::migrate`** 跑 [`schema::DDL`] 全部语句(idempotent,IF NOT EXISTS)
//! - **`Store::projects()`** 返回 [`project_repo::ProjectRepo`] 句柄(只借用 conn)
//!
//! ## 默认路径(PRD §6.1 本机状态)
//!
//! - macOS:`~/Library/Application Support/ai-config/store.sqlite`
//! - Linux:`~/.config/ai-config/store.sqlite`(`$XDG_CONFIG_HOME` 优先)
//! - Windows:`%APPDATA%\ai-config\store.sqlite`
//!
//! 走 `dirs` crate,跨平台一致;测试用 `Store::open_at(...)` 注入临时路径。

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use camino::Utf8PathBuf;
use rusqlite::Connection;
use thiserror::Error;

pub mod project_repo;
pub mod schema;

// `ProjectRepo` 重导出到顶级,使用方写 `use ai_config_store::ProjectRepo;` 即可。
// `StoreError` 不重导出 — lib.rs 自己定义了**完整**的 `StoreError`(包含
// `Open` / `DataDirUnknown` / `Sqlite` / `Project` 转发),使用方只需要面对一种
// 错误类型,无需知道内部模块边界。
pub use project_repo::ProjectRepo;

// ── 公共类型 ─────────────────────────────────────────────────────

/// 本机状态库的根句柄。`Connection` 本身**不是** `Sync`(内部用 `RefCell` 做
/// statement cache),所以本类型用 `Mutex<Connection>` 序列化所有访问 — 简单
/// 正确,代价是同一时刻只有一个线程能跑 SQL。GUI / CLI 都单进程,不是瓶颈;
/// 未来真有多进程并发需求时再换 `r2d2_sqlite` 池。
pub struct Store {
    conn: std::sync::Mutex<Connection>,
    path: Utf8PathBuf,
}

impl Store {
    /// 按 OS 约定路径打开(或创建)store。
    ///
    /// `dirs::data_dir()` 失败 → `StoreError::DataDirUnknown`(常见于 sandbox / chroot)。
    /// 父目录会自动 `mkdir -p`;SQLite 文件首次创建时跑 `migrate`。
    pub fn open() -> Result<Self, StoreError> {
        let path = default_path()?;
        Self::open_at(path.as_path())
    }

    /// 按任意路径打开(测试 / 自定义位置用)。
    pub fn open_at(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| StoreError::Open {
                    path: path.to_path_buf(),
                    reason: format!("创建父目录失败: {e}"),
                })?;
            }
        }
        let conn = Connection::open(path).map_err(|e| StoreError::Open {
            path: path.to_path_buf(),
            reason: format!("打开 SQLite 失败: {e}"),
        })?;
        let store = Self {
            conn: std::sync::Mutex::new(conn),
            path: Utf8PathBuf::from_path_buf(path.to_path_buf()).map_err(|p| StoreError::Open {
                path: p,
                reason: "路径含非 UTF-8 字符,SQLite metadata 拒绝".to_string(),
            })?,
        };
        store.migrate()?;
        tracing::info!(path = %store.path, "ai-config store 已就绪");
        Ok(store)
    }

    /// 跑 [`schema::DDL`] 全部 DDL(幂等)。**会短暂持有 `conn` 锁**。
    pub fn migrate(&self) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockPoisoned(e.to_string()))?;
        for stmt in schema::DDL {
            conn.execute_batch(stmt)?;
        }
        Ok(())
    }

    /// 借用 connection 拿 `projects` CRUD 句柄。**会**短期持锁完成所有 DB 操作。
    pub fn projects(&self) -> ProjectRepo<'_> {
        ProjectRepo::new(&self.conn)
    }

    /// store 实际文件路径(供 `--store-path` debug 子命令 / 错误信息用)。
    pub fn path(&self) -> &Utf8PathBuf {
        &self.path
    }
}

// ── 默认路径 ─────────────────────────────────────────────────────

/// `$XDG_DATA_HOME/ai-config/store.sqlite` 或 OS 约定,fallback 到 `~/.ai-config/store.sqlite`。
///
/// 优先级:
/// 1. `$XDG_DATA_HOME/ai-config/store.sqlite`(Linux)
/// 2. `dirs::data_dir()`(OS 默认:macOS=`~/Library/Application Support`,Win=`%APPDATA%`)
/// 3. `~/.ai-config/store.sqlite`(sandbox / chroot fallback)
pub fn default_path() -> Result<PathBuf, StoreError> {
    if let Some(base) = dirs::data_dir() {
        return Ok(base.join("ai-config").join("store.sqlite"));
    }
    let home = dirs::home_dir().ok_or(StoreError::DataDirUnknown)?;
    Ok(home.join(".ai-config").join("store.sqlite"))
}

// ── 本地错误 ─────────────────────────────────────────────────────

/// Store 错误。**故意不**复用 `CoreError`:`CoreError` 是产品错误(带 hint 字段、
/// 带 exit_code 映射),Store 错误更接近 IO 层,直接走 `?` 给上层包装。
///
/// 上层(GUI 的 Tauri command)需要把 `StoreError` 翻成人话时,用
/// `Display` 即可;若需要 exit_code,统一映射到 5(FS_ERROR)。
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("无法定位数据目录:$XDG_DATA_HOME / dirs::data_dir() / home_dir 全部为 None")]
    DataDirUnknown,

    #[error("打开 store `{path}` 失败:{reason}")]
    Open { path: PathBuf, reason: String },

    /// 持有 store 锁的线程 panic 过,Mutex 中毒。本工具**只**在 IO 失败时
    /// 短暂持锁,理论上不会发生;出现就重启 GUI 即可。
    #[error("store Mutex 中毒(持有线程 panic 过):{0}")]
    LockPoisoned(String),

    // `project_repo::StoreError` 转发
    #[error("项目仓库错误: {0}")]
    Project(#[from] project_repo::StoreError),

    // 给 `?` 用的 from impl(rusqlite error 在 project_repo 内部已转)
    #[error("SQLite 错误: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

// ── 单元测试(本文件只测 path 解析,CRUD 在 project_repo 测) ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_at_creates_db_and_runs_migrate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite");
        let store = Store::open_at(&path).expect("open_at");
        // migrate 完 projects 表应存在
        let n: i64 = store
            .conn
            .lock()
            .expect("test 不应该 panic 后持锁")
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='projects'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(n, 1, "projects 表应已被 migrate 创建");
    }

    #[test]
    fn default_path_is_under_data_dir_or_home() {
        let p = default_path().expect("default_path");
        assert!(p.ends_with("ai-config/store.sqlite") || p.ends_with("ai-config\\store.sqlite"));
    }

    #[test]
    fn open_at_missing_parent_dir_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deep/nested/store.sqlite");
        let _ = Store::open_at(&path).expect("open_at should mkdir -p");
    }
}
