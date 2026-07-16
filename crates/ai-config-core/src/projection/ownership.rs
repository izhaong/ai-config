use camino::Utf8PathBuf;

use super::model::{ProjectionId, ProjectionMode, ProjectionRecord, ProjectionState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionExpectation {
    pub id: ProjectionId,
    pub mode: ProjectionMode,
    pub canonical_source: Utf8PathBuf,
    pub source_content_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionObservation {
    Missing,
    Link { canonical_target: Utf8PathBuf },
    Regular { content_digest: String },
    Generated { target_fingerprint: String },
    Unreadable,
}

pub fn classify_projection(
    expected: &ProjectionExpectation,
    actual: &ProjectionObservation,
    record: Option<&ProjectionRecord>,
) -> ProjectionState {
    match actual {
        ProjectionObservation::Missing => ProjectionState::Missing,
        ProjectionObservation::Link { canonical_target }
            if canonical_target == &expected.canonical_source =>
        {
            ProjectionState::ManagedLink
        }
        ProjectionObservation::Link { .. } if record_matches_expectation(expected, record) => {
            ProjectionState::Drifted
        }
        ProjectionObservation::Regular { content_digest }
            if content_digest == &expected.source_content_digest =>
        {
            ProjectionState::Equivalent
        }
        ProjectionObservation::Generated { target_fingerprint }
            if record_matches_generated(expected, target_fingerprint, record) =>
        {
            ProjectionState::ManagedGenerated
        }
        _ => ProjectionState::Foreign,
    }
}

fn record_matches_expectation(
    expected: &ProjectionExpectation,
    record: Option<&ProjectionRecord>,
) -> bool {
    record.is_some_and(|record| record.id == expected.id && record.mode == expected.mode)
}

fn record_matches_generated(
    expected: &ProjectionExpectation,
    target_fingerprint: &str,
    record: Option<&ProjectionRecord>,
) -> bool {
    record_matches_expectation(expected, record)
        && record.is_some_and(|record| record.target_fingerprint == target_fingerprint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AssetKind, PlatformId};
    use crate::projection::model::ProjectionSurface;
    use camino::{Utf8Path, Utf8PathBuf};
    use std::fs;
    use tempfile::TempDir;

    fn expectation(source: Utf8PathBuf) -> ProjectionExpectation {
        expectation_with_digest(source, "source-digest".to_owned())
    }

    fn expectation_with_digest(
        source: Utf8PathBuf,
        source_content_digest: String,
    ) -> ProjectionExpectation {
        ProjectionExpectation {
            id: ProjectionId {
                scope_key: "project:/fixture".to_owned(),
                kind: AssetKind::Skill,
                name: "demo".to_owned(),
                surface: ProjectionSurface::Platform(PlatformId::Cursor),
            },
            mode: ProjectionMode::DirectLink,
            canonical_source: source,
            source_content_digest,
        }
    }

    #[cfg(unix)]
    #[test]
    fn exact_symlink_is_managed_link_without_ledger_record() {
        let temp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let source = root.join("source/demo");
        let dest = root.join("platform/demo");
        fs::create_dir_all(source.as_std_path()).unwrap();
        fs::create_dir_all(dest.parent().unwrap().as_std_path()).unwrap();
        std::os::unix::fs::symlink(source.as_std_path(), dest.as_std_path()).unwrap();
        let canonical_source =
            Utf8PathBuf::from_path_buf(fs::canonicalize(source.as_std_path()).unwrap()).unwrap();
        let canonical_target =
            Utf8PathBuf::from_path_buf(fs::canonicalize(dest.as_std_path()).unwrap()).unwrap();

        assert_eq!(
            classify_projection(
                &expectation(canonical_source),
                &ProjectionObservation::Link { canonical_target },
                None,
            ),
            ProjectionState::ManagedLink
        );
    }

    #[test]
    fn equal_regular_copy_is_equivalent_without_ledger_record() {
        let temp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let source = root.join("source/demo");
        let copy = root.join("platform/demo");
        fs::create_dir_all(source.as_std_path()).unwrap();
        fs::create_dir_all(copy.as_std_path()).unwrap();
        fs::write(source.join("SKILL.md").as_std_path(), "same content\n").unwrap();
        fs::write(copy.join("SKILL.md").as_std_path(), "same content\n").unwrap();
        let source_digest = crate::projection::fingerprint::directory_digest(&source).unwrap();
        let copy_digest = crate::projection::fingerprint::directory_digest(&copy).unwrap();
        let canonical_source =
            Utf8PathBuf::from_path_buf(fs::canonicalize(source.as_std_path()).unwrap()).unwrap();

        assert_eq!(
            classify_projection(
                &expectation_with_digest(canonical_source, source_digest.clone()),
                &ProjectionObservation::Regular {
                    content_digest: copy_digest,
                },
                None,
            ),
            ProjectionState::Equivalent
        );
    }

    #[test]
    fn wrong_source_link_is_foreign_without_record_and_drifted_with_matching_record() {
        let expected = expectation(Utf8PathBuf::from("/canonical/source/demo"));
        let actual = ProjectionObservation::Link {
            canonical_target: Utf8PathBuf::from("/foreign/source/demo"),
        };
        let record = ProjectionRecord {
            id: expected.id.clone(),
            mode: expected.mode,
            source_path: expected.canonical_source.clone(),
            target_path: Utf8PathBuf::from("/platform/demo"),
            entry_key: None,
            source_fingerprint: expected.source_content_digest.clone(),
            target_fingerprint: "old-target-fingerprint".to_owned(),
            applied_at: chrono::Utc::now(),
        };

        assert_eq!(
            classify_projection(&expected, &actual, None),
            ProjectionState::Foreign
        );
        assert_eq!(
            classify_projection(&expected, &actual, Some(&record)),
            ProjectionState::Drifted
        );
    }

    #[test]
    fn missing_target_is_missing_without_ownership_inference() {
        let expected = expectation(Utf8PathBuf::from("/canonical/source/demo"));

        assert_eq!(
            classify_projection(&expected, &ProjectionObservation::Missing, None),
            ProjectionState::Missing
        );
    }

    #[test]
    fn generated_target_requires_a_matching_ledger_fingerprint() {
        let expected = ProjectionExpectation {
            mode: ProjectionMode::GeneratedJson,
            ..expectation(Utf8PathBuf::from("/canonical/source/demo"))
        };
        let actual = ProjectionObservation::Generated {
            target_fingerprint: "generated-fingerprint".to_owned(),
        };
        let record = ProjectionRecord {
            id: expected.id.clone(),
            mode: expected.mode,
            source_path: expected.canonical_source.clone(),
            target_path: Utf8PathBuf::from("/platform/config.json"),
            entry_key: Some("demo".to_owned()),
            source_fingerprint: expected.source_content_digest.clone(),
            target_fingerprint: "generated-fingerprint".to_owned(),
            applied_at: chrono::Utc::now(),
        };

        assert_eq!(
            classify_projection(&expected, &actual, None),
            ProjectionState::Foreign
        );
        assert_eq!(
            classify_projection(&expected, &actual, Some(&record)),
            ProjectionState::ManagedGenerated
        );
    }

    #[test]
    fn unreadable_target_is_foreign_without_ownership_inference() {
        let expected = expectation(Utf8PathBuf::from("/canonical/source/demo"));

        assert_eq!(
            classify_projection(&expected, &ProjectionObservation::Unreadable, None),
            ProjectionState::Foreign
        );
    }

    #[test]
    fn wrong_source_link_with_mismatched_ledger_mode_is_foreign() {
        let expected = expectation(Utf8PathBuf::from("/canonical/source/demo"));
        let record = ProjectionRecord {
            id: expected.id.clone(),
            mode: ProjectionMode::GeneratedJson,
            source_path: expected.canonical_source.clone(),
            target_path: Utf8PathBuf::from("/platform/demo"),
            entry_key: None,
            source_fingerprint: expected.source_content_digest.clone(),
            target_fingerprint: "old-target-fingerprint".to_owned(),
            applied_at: chrono::Utc::now(),
        };

        assert_eq!(
            classify_projection(
                &expected,
                &ProjectionObservation::Link {
                    canonical_target: Utf8PathBuf::from("/foreign/source/demo"),
                },
                Some(&record),
            ),
            ProjectionState::Foreign
        );
    }
}
