//! `projects` 表 CRUD(对齐 PRD §3.1 视图反转 + §6.1 项目注册状态)。
//!
//! ## 关键约束
//! - **`add` 同名 → 报错**:UI 应在 prompt 前先查 `get_by_name`,避免无谓失败
//! - **`remove` 不存在 → 报错**:UI 应 disable `×` 按钮(避免无效操作)
//! - **不存 `core::model::Project` 序列化**:字段少且稳定,显式 SQL 列;serde 留待 W10 items/targets 表

use camino::Utf8Path;
use rusqlite::{params, OptionalExtension, Row};
use thiserror::Error;

use agent_manager_core::model::Project;

/// `projects` 表的 CRUD 句柄。
///
/// 借 `&Mutex<Connection>`(而不是 `&Connection`):每次 SQL 操作**前** `lock()`,
/// 短期持锁,操作完释放。`Connection` 本身不是 `Sync`,但 `Mutex` 是。
pub struct ProjectRepo<'a> {
    conn: &'a std::sync::Mutex<rusqlite::Connection>,
}

impl<'a> ProjectRepo<'a> {
    pub(crate) fn new(conn: &'a std::sync::Mutex<rusqlite::Connection>) -> Self {
        Self { conn }
    }

    /// 拿一个临时 `Connection` 借用(短锁)。在闭包里用 `?` 抛错会自动释放锁。
    fn with_conn<R>(
        &self,
        f: impl FnOnce(&rusqlite::Connection) -> Result<R, StoreError>,
    ) -> Result<R, StoreError> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockPoisoned(e.to_string()))?;
        f(&guard)
    }

    /// 列出全部已注册项目,按 `id` 升序(注册时间先后)。
    pub fn list(&self) -> Result<Vec<Project>, StoreError> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, root_path, registered_at FROM projects ORDER BY id ASC",
            )?;
            let rows = stmt.query_map([], row_to_project)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    /// 按 `name` 查询单个项目(`None` 表示未注册)。
    pub fn get_by_name(&self, name: &str) -> Result<Option<Project>, StoreError> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, root_path, registered_at FROM projects WHERE name = ?1",
            )?;
            let row = stmt.query_row(params![name], row_to_project).optional()?;
            Ok(row)
        })
    }

    /// 新增项目。**`name` 已存在 → `StoreError::ProjectExists`**(不更新,不忽略)。
    ///
    /// 成功返回的 `Project.id` 已被 SQLite 分配,`registered_at` 取调用瞬间的 UTC。
    pub fn add(&self, name: &str, root_path: &Utf8Path) -> Result<Project, StoreError> {
        let registered_at = chrono::Utc::now().to_rfc3339();
        let name_owned = name.to_string();
        let root_path_owned = root_path.to_path_buf();
        let registered_at_for_insert = registered_at.clone();
        let result = self.with_conn(|conn| {
            Ok(conn.execute(
                "INSERT INTO projects (name, root_path, registered_at) VALUES (?1, ?2, ?3)",
                params![
                    name_owned.as_str(),
                    root_path_owned.as_str(),
                    registered_at_for_insert.as_str()
                ],
            )?)
        });
        match result {
            Ok(_) => self.with_conn(|conn| {
                let id = conn.last_insert_rowid();
                Ok(Project {
                    id: u64::try_from(id).expect("SQLite rowid 是非负整数"),
                    name: name_owned,
                    root_path: root_path_owned,
                    registered_at: parse_rfc3339(&registered_at)?,
                })
            }),
            Err(StoreError::Sqlite(e)) => {
                if is_unique_violation(&e) {
                    Err(StoreError::ProjectExists {
                        name: name_owned,
                        hint: format!(
                            "项目名 `{name}` 已被本工具纳管;如需重新指向别的根目录,先 `agent-manager project remove {name}` 再 add"
                        ),
                    })
                } else {
                    Err(StoreError::Sqlite(e))
                }
            }
            Err(other) => Err(other),
        }
    }

    /// 移除项目(按 `name`)。**项目不存在 → `StoreError::ProjectNotFound`**(无副作用)。
    pub fn remove(&self, name: &str) -> Result<(), StoreError> {
        let name_owned = name.to_string();
        self.with_conn(|conn| {
            let affected = conn.execute(
                "DELETE FROM projects WHERE name = ?1",
                params![name_owned.as_str()],
            )?;
            if affected == 0 {
                Err(StoreError::ProjectNotFound {
                    name: name_owned,
                    hint: "可能同步多个 GUI 实例?其中一个已经删除".to_string(),
                })
            } else {
                Ok(())
            }
        })
    }
}

// ── helpers ───────────────────────────────────────────────────────

fn row_to_project(row: &Row<'_>) -> rusqlite::Result<Project> {
    let id: i64 = row.get(0)?;
    let name: String = row.get(1)?;
    let root_path: String = row.get(2)?;
    let registered_at_str: String = row.get(3)?;
    let registered_at = chrono::DateTime::parse_from_rfc3339(&registered_at_str)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
        })?;
    Ok(Project {
        id: u64::try_from(id).map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "projects.id 居然是负数,schema 损坏",
                )),
            )
        })?,
        name,
        root_path: camino::Utf8PathBuf::from(root_path),
        registered_at,
    })
}

fn parse_rfc3339(s: &str) -> Result<chrono::DateTime<chrono::Utc>, StoreError> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .map_err(|e| StoreError::BadTimestamp(e.to_string()))
}

fn is_unique_violation(e: &rusqlite::Error) -> bool {
    // SQLite UNIQUE 失败:
    //   - 主码 `SQLITE_CONSTRAINT` = 19 → ErrorCode::ConstraintViolation
    //   - 扩展码 `SQLITE_CONSTRAINT_UNIQUE` = 2067
    // 没有 message 字段(rusqlite 0.31 的 SqliteFailure 只带 code/extended_code),
    // 所以用主码 + 扩展码双判。
    if let rusqlite::Error::SqliteFailure(err, _msg) = e {
        return err.code == rusqlite::ErrorCode::ConstraintViolation && err.extended_code == 2067;
    }
    false
}

// ── 本地错误类型(store 内部用,不入 core) ───────────────────────

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("SQLite 错误: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("项目 `{name}` 已存在")]
    ProjectExists { name: String, hint: String },

    #[error("项目 `{name}` 不存在")]
    ProjectNotFound { name: String, hint: String },

    #[error("时间戳解析失败: {0}")]
    BadTimestamp(String),

    #[error("store Mutex 中毒(持有线程 panic 过):{0}")]
    LockPoisoned(String),
}

impl StoreError {
    /// 错误是否表示"重试也无用"(供上层决定是否打日志)。
    pub fn is_user_error(&self) -> bool {
        matches!(
            self,
            Self::ProjectExists { .. } | Self::ProjectNotFound { .. }
        )
    }
}

// ── 单元测试 ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// 测试用 Store(临时目录,完事自动清)。
    fn fixture() -> (TempDir, super::super::Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("store.sqlite");
        let store = super::super::Store::open_at(&path).expect("Store::open_at");
        (dir, store)
    }

    #[test]
    fn add_list_get_remove_roundtrip() {
        let (_dir, store) = fixture();
        let repo = store.projects();

        // 初始空
        assert_eq!(repo.list().unwrap().len(), 0);

        // add
        let p = repo
            .add("zh-cloud", Utf8Path::new("/Users/zh/Code/zh-cloud"))
            .expect("add");
        assert_eq!(p.name, "zh-cloud");
        assert!(p.id > 0, "id 已被分配");
        assert_eq!(repo.list().unwrap().len(), 1);

        // get_by_name 命中
        let got = repo.get_by_name("zh-cloud").expect("get").expect("Some");
        assert_eq!(got.id, p.id);
        assert_eq!(got.root_path.as_str(), "/Users/zh/Code/zh-cloud");

        // 第二个 add
        let p2 = repo
            .add(
                "agent-manager",
                Utf8Path::new("/Users/zh/Code/zh-cloud/agent-manager"),
            )
            .expect("add 2");
        assert_eq!(repo.list().unwrap().len(), 2);
        assert_ne!(p.id, p2.id);

        // remove
        repo.remove("zh-cloud").expect("remove");
        assert_eq!(repo.list().unwrap().len(), 1);
        assert!(repo.get_by_name("zh-cloud").expect("get").is_none());
    }

    #[test]
    fn add_duplicate_name_returns_project_exists() {
        let (_dir, store) = fixture();
        let repo = store.projects();
        repo.add("dup", Utf8Path::new("/a")).expect("first");
        let err = repo.add("dup", Utf8Path::new("/b")).unwrap_err();
        assert!(
            matches!(err, StoreError::ProjectExists { .. }),
            "got {err:?}"
        );
        assert!(err.is_user_error());
    }

    #[test]
    fn remove_missing_returns_project_not_found() {
        let (_dir, store) = fixture();
        let err = store.projects().remove("ghost").unwrap_err();
        assert!(
            matches!(err, StoreError::ProjectNotFound { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn list_preserves_insertion_order() {
        let (_dir, store) = fixture();
        let repo = store.projects();
        for n in ["alpha", "beta", "gamma"] {
            repo.add(n, Utf8Path::new(&format!("/{n}"))).expect("add");
        }
        let names: Vec<String> = repo.list().unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["alpha", "beta", "gamma"]);
    }
}
