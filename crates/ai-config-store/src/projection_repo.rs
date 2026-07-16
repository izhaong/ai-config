//! Projection ledger 的 SQLite 实现。

use ai_config_core::error::CoreError;
use ai_config_core::projection::ledger::ProjectionLedger;
use ai_config_core::projection::model::{
    LedgerMutation, ProjectionId, ProjectionMode, ProjectionRecord, ProjectionSurface,
};
use rusqlite::{params, OptionalExtension, Row};
use std::collections::HashSet;
use thiserror::Error;

pub struct ProjectionRepo<'a> {
    conn: &'a std::sync::Mutex<rusqlite::Connection>,
}

impl<'a> ProjectionRepo<'a> {
    pub(crate) fn new(conn: &'a std::sync::Mutex<rusqlite::Connection>) -> Self {
        Self { conn }
    }

    fn with_conn<R>(
        &self,
        f: impl FnOnce(&rusqlite::Connection) -> Result<R, StoreError>,
    ) -> Result<R, StoreError> {
        let guard = self
            .conn
            .lock()
            .map_err(|error| StoreError::LockPoisoned(error.to_string()))?;
        f(&guard)
    }

    fn get_impl(&self, id: &ProjectionId) -> Result<Option<ProjectionRecord>, StoreError> {
        let key = LedgerKey::from_id(id)?;
        self.with_conn(|conn| {
            let mut statement = conn.prepare(
                "SELECT scope_key, kind, name, surface_json, mode, source_path, target_path, \
                 entry_key, source_fingerprint, target_fingerprint, applied_at \
                 FROM projection_ledger \
                 WHERE scope_key = ?1 AND kind = ?2 AND name = ?3 AND surface_json = ?4",
            )?;
            let row = statement
                .query_row(
                    params![key.scope_key, key.kind, key.name, key.surface_json],
                    |row| row_to_record(row).map_err(to_sql_conversion_error),
                )
                .optional()?;
            Ok(row)
        })
    }

    fn get_many_impl(&self, ids: &[ProjectionId]) -> Result<Vec<ProjectionRecord>, StoreError> {
        self.with_conn(|conn| {
            let mut statement = conn.prepare(
                "SELECT scope_key, kind, name, surface_json, mode, source_path, target_path, \
                 entry_key, source_fingerprint, target_fingerprint, applied_at \
                 FROM projection_ledger \
                 WHERE scope_key = ?1 AND kind = ?2 AND name = ?3 AND surface_json = ?4",
            )?;
            let mut records = Vec::new();
            for id in ids {
                let key = LedgerKey::from_id(id)?;
                let row = statement
                    .query_row(
                        params![key.scope_key, key.kind, key.name, key.surface_json],
                        |row| row_to_record(row).map_err(to_sql_conversion_error),
                    )
                    .optional()?;
                if let Some(record) = row {
                    records.push(record);
                }
            }
            Ok(records)
        })
    }

    fn apply_batch_impl(&self, mutations: &[LedgerMutation]) -> Result<(), StoreError> {
        let mut seen = HashSet::new();
        for mutation in mutations {
            let id = mutation_id(mutation);
            if !seen.insert(id) {
                return Err(StoreError::DuplicateMutation(id.clone()));
            }
        }

        let mut conn = self
            .conn
            .lock()
            .map_err(|error| StoreError::LockPoisoned(error.to_string()))?;
        let transaction = conn.transaction()?;
        for mutation in mutations {
            match mutation {
                LedgerMutation::Upsert(record) => upsert(&transaction, record)?,
                LedgerMutation::Remove(id) => remove(&transaction, id)?,
            }
        }
        transaction.commit()?;
        Ok(())
    }
}

impl ProjectionLedger for ProjectionRepo<'_> {
    fn get(&self, id: &ProjectionId) -> Result<Option<ProjectionRecord>, CoreError> {
        self.get_impl(id).map_err(to_core_error)
    }

    fn get_many(&self, ids: &[ProjectionId]) -> Result<Vec<ProjectionRecord>, CoreError> {
        self.get_many_impl(ids).map_err(to_core_error)
    }

    fn apply_batch(&self, mutations: &[LedgerMutation]) -> Result<(), CoreError> {
        self.apply_batch_impl(mutations).map_err(to_core_error)
    }
}

fn mutation_id(mutation: &LedgerMutation) -> &ProjectionId {
    match mutation {
        LedgerMutation::Upsert(record) => &record.id,
        LedgerMutation::Remove(id) => id,
    }
}

fn upsert(
    transaction: &rusqlite::Transaction<'_>,
    record: &ProjectionRecord,
) -> Result<(), StoreError> {
    let key = LedgerKey::from_id(&record.id)?;
    let mode = serde_json::to_string(&record.mode)
        .map_err(|error| StoreError::Serialization(error.to_string()))?;
    transaction.execute(
        "INSERT INTO projection_ledger (scope_key, kind, name, surface_json, mode, source_path, \
         target_path, entry_key, source_fingerprint, target_fingerprint, applied_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
         ON CONFLICT(scope_key, kind, name, surface_json) DO UPDATE SET \
           mode = excluded.mode, source_path = excluded.source_path, target_path = excluded.target_path, \
           entry_key = excluded.entry_key, source_fingerprint = excluded.source_fingerprint, \
           target_fingerprint = excluded.target_fingerprint, applied_at = excluded.applied_at",
        params![
            key.scope_key,
            key.kind,
            key.name,
            key.surface_json,
            mode,
            record.source_path.as_str(),
            record.target_path.as_str(),
            record.entry_key.as_deref(),
            record.source_fingerprint,
            record.target_fingerprint,
            record.applied_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

fn remove(transaction: &rusqlite::Transaction<'_>, id: &ProjectionId) -> Result<(), StoreError> {
    let key = LedgerKey::from_id(id)?;
    transaction.execute(
        "DELETE FROM projection_ledger WHERE scope_key = ?1 AND kind = ?2 AND name = ?3 AND surface_json = ?4",
        params![key.scope_key, key.kind, key.name, key.surface_json],
    )?;
    Ok(())
}

fn row_to_record(row: &Row<'_>) -> Result<ProjectionRecord, StoreError> {
    let kind: String = row.get(1)?;
    let surface_json: String = row.get(3)?;
    let mode: String = row.get(4)?;
    let applied_at: String = row.get(10)?;
    Ok(ProjectionRecord {
        id: ProjectionId {
            scope_key: row.get(0)?,
            kind: serde_json::from_str(&kind)
                .map_err(|error| StoreError::Serialization(error.to_string()))?,
            name: row.get(2)?,
            surface: serde_json::from_str::<ProjectionSurface>(&surface_json)
                .map_err(|error| StoreError::Serialization(error.to_string()))?,
        },
        mode: serde_json::from_str::<ProjectionMode>(&mode)
            .map_err(|error| StoreError::Serialization(error.to_string()))?,
        source_path: row.get::<_, String>(5)?.into(),
        target_path: row.get::<_, String>(6)?.into(),
        entry_key: row.get(7)?,
        source_fingerprint: row.get(8)?,
        target_fingerprint: row.get(9)?,
        applied_at: chrono::DateTime::parse_from_rfc3339(&applied_at)
            .map_err(|error| StoreError::Serialization(error.to_string()))?
            .with_timezone(&chrono::Utc),
    })
}

fn to_sql_conversion_error(error: StoreError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

fn to_core_error(error: StoreError) -> CoreError {
    CoreError::ProjectionLedger(error.to_string())
}

struct LedgerKey {
    scope_key: String,
    kind: String,
    name: String,
    surface_json: String,
}

impl LedgerKey {
    fn from_id(id: &ProjectionId) -> Result<Self, StoreError> {
        Ok(Self {
            scope_key: id.scope_key.clone(),
            kind: serde_json::to_string(&id.kind)
                .map_err(|error| StoreError::Serialization(error.to_string()))?,
            name: id.name.clone(),
            surface_json: serde_json::to_string(&id.surface)
                .map_err(|error| StoreError::Serialization(error.to_string()))?,
        })
    }
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("SQLite 错误: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("账本序列化错误: {0}")]
    Serialization(String),

    #[error("一个账本批次不能多次修改同一投影: {0:?}")]
    DuplicateMutation(ProjectionId),

    #[error("store Mutex 中毒(持有线程 panic 过):{0}")]
    LockPoisoned(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_config_core::model::{AssetKind, PlatformId};
    use ai_config_core::projection::model::{ProjectionMode, ProjectionSurface};
    use camino::Utf8PathBuf;

    fn record(name: &str) -> ProjectionRecord {
        ProjectionRecord {
            id: ProjectionId {
                scope_key: "user".to_owned(),
                kind: AssetKind::Mcp,
                name: name.to_owned(),
                surface: ProjectionSurface::Platform(PlatformId::Claude),
            },
            mode: ProjectionMode::GeneratedJson,
            source_path: Utf8PathBuf::from("/source"),
            target_path: Utf8PathBuf::from("/target"),
            entry_key: Some(name.to_owned()),
            source_fingerprint: "source".to_owned(),
            target_fingerprint: "target".to_owned(),
            applied_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn sqlite_batch_rejects_duplicate_ids_without_partial_write() {
        let directory = tempfile::tempdir().unwrap();
        let store = crate::Store::open_at(&directory.path().join("store.sqlite")).unwrap();
        let ledger = store.projections();
        let existing = record("existing");
        ledger
            .apply_batch(&[LedgerMutation::Upsert(existing.clone())])
            .unwrap();

        let new = record("new");
        assert!(ledger
            .apply_batch(&[
                LedgerMutation::Upsert(new.clone()),
                LedgerMutation::Upsert(new.clone()),
            ])
            .is_err());

        assert_eq!(ledger.get(&existing.id).unwrap(), Some(existing));
        assert_eq!(ledger.get(&new.id).unwrap(), None);
    }
}
