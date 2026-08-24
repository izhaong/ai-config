//! `settings` KV 表 — GUI 偏好与 Git 同步配置。

use thiserror::Error;

use ai_config_core::git::{GitSyncConfig, DEFAULT_BRANCH};

pub const KEY_GIT_REMOTE_URL: &str = "git_remote_url";
pub const KEY_GIT_BRANCH: &str = "git_branch";

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("store Mutex 中毒: {0}")]
    LockPoisoned(String),

    #[error("SQLite 错误: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub struct SettingsRepo<'a> {
    conn: &'a std::sync::Mutex<rusqlite::Connection>,
}

impl<'a> SettingsRepo<'a> {
    pub(crate) fn new(conn: &'a std::sync::Mutex<rusqlite::Connection>) -> Self {
        Self { conn }
    }

    fn with_conn<R>(
        &self,
        f: impl FnOnce(&rusqlite::Connection) -> Result<R, SettingsError>,
    ) -> Result<R, SettingsError> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| SettingsError::LockPoisoned(e.to_string()))?;
        f(&guard)
    }

    pub fn get(&self, key: &str) -> Result<Option<String>, SettingsError> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
            let row = stmt
                .query_row(rusqlite::params![key], |r| r.get::<_, String>(0))
                .optional()?;
            Ok(row)
        })
    }

    pub fn set(&self, key: &str, value: &str) -> Result<(), SettingsError> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![key, value],
            )?;
            Ok(())
        })
    }

    pub fn delete(&self, key: &str) -> Result<(), SettingsError> {
        self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM settings WHERE key = ?1",
                rusqlite::params![key],
            )?;
            Ok(())
        })
    }

    pub fn git_config(&self) -> Result<GitSyncConfig, SettingsError> {
        let remote_url = self.get(KEY_GIT_REMOTE_URL)?;
        let branch = self
            .get(KEY_GIT_BRANCH)?
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_BRANCH.to_string());
        Ok(GitSyncConfig {
            remote_url: remote_url.filter(|s| !s.trim().is_empty()),
            branch,
        })
    }

    pub fn set_git_config(&self, config: &GitSyncConfig) -> Result<(), SettingsError> {
        match config
            .remote_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(url) => self.set(KEY_GIT_REMOTE_URL, url)?,
            None => {
                let _ = self.delete(KEY_GIT_REMOTE_URL);
            }
        }
        let branch = if config.branch.trim().is_empty() {
            DEFAULT_BRANCH
        } else {
            config.branch.trim()
        };
        self.set(KEY_GIT_BRANCH, branch)?;
        Ok(())
    }
}

use rusqlite::OptionalExtension;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    #[test]
    fn git_config_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_at(&dir.path().join("s.sqlite")).unwrap();
        let repo = store.settings();
        repo.set_git_config(&GitSyncConfig {
            remote_url: Some("https://gitea.example/a.git".into()),
            branch: "develop".into(),
        })
        .unwrap();
        let cfg = repo.git_config().unwrap();
        assert_eq!(
            cfg.remote_url.as_deref(),
            Some("https://gitea.example/a.git")
        );
        assert_eq!(cfg.branch, "develop");
    }
}
