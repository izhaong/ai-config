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

/// canonical asset 的来源层；同名项按 Global → Workspace → Project 覆盖。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SourceLayer {
    Global,
    Workspace,
    Project,
}

/// 一次投影请求允许写入的目标边界。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentScope {
    User,
    Workspace,
    Project,
}

/// Adapter 给 planner 返回的目标路径。聚合容器使用 `entry_key` 标识唯一受管条目。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectionTarget {
    pub path: Utf8PathBuf,
    pub entry_key: Option<String>,
}

/// apply 前的路径形态快照。与 `Equivalent` 的 semantic digest 分开保存。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FingerprintType {
    Missing,
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PathFingerprint {
    pub entry_type: FingerprintType,
    pub digest: Option<String>,
    pub link_target: Option<Utf8PathBuf>,
    pub mode: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_target_and_path_fingerprint_are_stably_serializable() {
        let target = ProjectionTarget {
            path: Utf8PathBuf::from("/repo/.agents/skills/demo"),
            entry_key: None,
        };
        let fingerprint = PathFingerprint {
            entry_type: FingerprintType::Directory,
            digest: Some("digest".to_owned()),
            link_target: None,
            mode: None,
        };

        assert_eq!(
            serde_json::to_value(target).unwrap(),
            serde_json::json!({"path":"/repo/.agents/skills/demo","entry_key":null})
        );
        assert_eq!(
            serde_json::to_value(fingerprint).unwrap(),
            serde_json::json!({"entry_type":"directory","digest":"digest","link_target":null,"mode":null})
        );
    }
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
