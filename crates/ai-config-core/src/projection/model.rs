use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::{AssetKind, PlatformId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum ProjectionSurface {
    Platform(PlatformId),
    ProjectEntry,
    /// 多个平台读取同一物理目标时的唯一所有权面，例如 Cursor/Codex skills。
    SharedTarget {
        key: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ProjectionId {
    pub scope_key: String,
    pub kind: AssetKind,
    pub name: String,
    pub surface: ProjectionSurface,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionMode {
    DirectLink,
    GeneratedJson,
    GeneratedToml,
    GeneratedYaml,
    ExternalDirectory,
    CopyFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionState {
    Missing,
    ManagedLink,
    ManagedGenerated,
    Copied,
    Equivalent,
    Foreign,
    Conflict,
    Drifted,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectionRecord {
    pub id: ProjectionId,
    pub mode: ProjectionMode,
    pub source_path: Utf8PathBuf,
    pub target_path: Utf8PathBuf,
    pub entry_key: Option<String>,
    pub source_fingerprint: String,
    pub target_fingerprint: String,
    pub applied_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum LedgerMutation {
    Upsert(ProjectionRecord),
    Remove(ProjectionId),
}
