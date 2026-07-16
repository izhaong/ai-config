use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use crate::error::CoreError;

use super::model::{LedgerMutation, ProjectionId, ProjectionRecord};

/// 可丢失但安全降级的投影所有权账本。
///
/// 丢失账本不能赋予任何目标所有权；调用方必须按 `Foreign` 处理未知生成物。
pub trait ProjectionLedger: Send + Sync {
    fn get(&self, id: &ProjectionId) -> Result<Option<ProjectionRecord>, CoreError>;
    fn get_many(&self, ids: &[ProjectionId]) -> Result<Vec<ProjectionRecord>, CoreError>;
    fn apply_batch(&self, mutations: &[LedgerMutation]) -> Result<(), CoreError>;
}

#[derive(Default)]
pub struct MemoryProjectionLedger {
    records: Mutex<HashMap<ProjectionId, ProjectionRecord>>,
}

impl ProjectionLedger for MemoryProjectionLedger {
    fn get(&self, id: &ProjectionId) -> Result<Option<ProjectionRecord>, CoreError> {
        let records = self
            .records
            .lock()
            .map_err(|_| CoreError::ProjectionLedger("memory ledger lock poisoned".to_owned()))?;
        Ok(records.get(id).cloned())
    }

    fn get_many(&self, ids: &[ProjectionId]) -> Result<Vec<ProjectionRecord>, CoreError> {
        let records = self
            .records
            .lock()
            .map_err(|_| CoreError::ProjectionLedger("memory ledger lock poisoned".to_owned()))?;
        Ok(ids
            .iter()
            .filter_map(|id| records.get(id).cloned())
            .collect())
    }

    fn apply_batch(&self, mutations: &[LedgerMutation]) -> Result<(), CoreError> {
        let mut seen = HashSet::new();
        for mutation in mutations {
            let id = match mutation {
                LedgerMutation::Upsert(record) => &record.id,
                LedgerMutation::Remove(id) => id,
            };
            if !seen.insert(id) {
                return Err(CoreError::ProjectionLedger(format!(
                    "batch contains more than one mutation for projection {:?}",
                    id
                )));
            }
        }

        let mut records = self
            .records
            .lock()
            .map_err(|_| CoreError::ProjectionLedger("memory ledger lock poisoned".to_owned()))?;
        let mut next = records.clone();
        for mutation in mutations {
            match mutation {
                LedgerMutation::Upsert(record) => {
                    next.insert(record.id.clone(), record.clone());
                }
                LedgerMutation::Remove(id) => {
                    next.remove(id);
                }
            }
        }
        *records = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AssetKind, PlatformId};
    use crate::projection::model::{ProjectionMode, ProjectionSurface};
    use camino::Utf8PathBuf;

    fn id(name: &str) -> ProjectionId {
        ProjectionId {
            scope_key: "user".to_owned(),
            kind: AssetKind::Skill,
            name: name.to_owned(),
            surface: ProjectionSurface::Platform(PlatformId::Cursor),
        }
    }

    fn record(name: &str) -> ProjectionRecord {
        ProjectionRecord {
            id: id(name),
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
    fn batch_is_atomic_when_a_later_mutation_is_invalid() {
        let ledger = MemoryProjectionLedger::default();
        let existing = record("existing");
        ledger
            .apply_batch(&[LedgerMutation::Upsert(existing.clone())])
            .unwrap();

        let result = ledger.apply_batch(&[
            LedgerMutation::Upsert(record("new")),
            LedgerMutation::Upsert(record("new")),
        ]);

        assert!(result.is_err());
        assert_eq!(ledger.get(&existing.id).unwrap(), Some(existing));
        assert_eq!(ledger.get(&id("new")).unwrap(), None);
    }
}
