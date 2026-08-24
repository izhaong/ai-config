use std::fs;
use std::path::{Path, PathBuf};

use agent_manager_core::model::{AssetKind, PlatformId};
use agent_manager_core::projection::ledger::ProjectionLedger;
use agent_manager_core::projection::model::{ProjectionMode, ProjectionSurface};
use agent_manager_store::Store;
use rusqlite::{params, Connection};

#[test]
fn read_only_open_reads_legacy_ledger_without_changing_its_bytes_schema_or_directory() {
    let directory = tempfile::tempdir().expect("temporary ledger directory");
    let ledger_path = directory.path().join("projection-ledger.sqlite");
    create_legacy_ledger(&ledger_path);

    let before_bytes = fs::read(&ledger_path).expect("legacy ledger bytes");
    let before_schema = schema_sql(&ledger_path);
    let before_entries = directory_entries(directory.path());

    let store = Store::open_read_only_at(&ledger_path).expect("read-only legacy ledger");
    let records = store
        .projections()
        .list_scope("project:/fixture")
        .expect("read legacy projection records");

    assert_eq!(
        records.len(),
        1,
        "legacy ownership evidence remains readable"
    );
    assert_eq!(
        fs::read(&ledger_path).expect("legacy ledger bytes"),
        before_bytes
    );
    assert_eq!(schema_sql(&ledger_path), before_schema);
    assert_eq!(directory_entries(directory.path()), before_entries);
}

#[cfg(unix)]
#[test]
fn read_only_open_rejects_a_symlinked_ledger_without_creating_ownership_or_sidecars() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temporary ledger directory");
    let target_path = directory.path().join("real-ledger.sqlite");
    let ledger_path = directory.path().join("projection-ledger.sqlite");
    create_legacy_ledger(&target_path);

    let before_target_bytes = fs::read(&target_path).expect("target ledger bytes");
    let before_entries = directory_entries(directory.path());
    symlink(&target_path, &ledger_path).expect("symlink ledger");
    let entries_with_symlink = directory_entries(directory.path());

    let error = match Store::open_read_only_at(&ledger_path) {
        Ok(_) => panic!("symlink ledger must be unavailable"),
        Err(error) => error,
    };

    assert!(
        error.to_string().contains("symbolic link"),
        "symlink rejection must remain explicit: {error}"
    );
    assert_eq!(
        fs::read(&target_path).expect("target ledger bytes"),
        before_target_bytes
    );
    assert_eq!(directory_entries(directory.path()), entries_with_symlink);
    assert!(
        !directory_entries(directory.path()).iter().any(|entry| entry
            .file_name()
            .is_some_and(|name| name == "projection-ledger.sqlite-wal"
                || name == "projection-ledger.sqlite-shm")),
        "read-only rejection must not create SQLite sidecars"
    );
    assert_ne!(
        before_entries, entries_with_symlink,
        "fixture must contain a ledger symlink"
    );
}

fn create_legacy_ledger(path: &Path) {
    let connection = Connection::open(path).expect("create legacy ledger");
    let kind = serde_json::to_string(&AssetKind::Skill).expect("serialize legacy kind");
    let surface = serde_json::to_string(&ProjectionSurface::Platform(PlatformId::Cursor))
        .expect("serialize legacy surface");
    let mode =
        serde_json::to_string(&ProjectionMode::GeneratedJson).expect("serialize legacy mode");
    connection
        .execute_batch(
            "
            CREATE TABLE projection_ledger (
                scope_key TEXT NOT NULL,
                kind TEXT NOT NULL,
                name TEXT NOT NULL,
                surface_json TEXT NOT NULL,
                mode TEXT NOT NULL,
                source_path TEXT NOT NULL,
                target_path TEXT NOT NULL,
                entry_key TEXT,
                source_fingerprint TEXT NOT NULL,
                target_fingerprint TEXT NOT NULL,
                applied_at TEXT NOT NULL,
                PRIMARY KEY (scope_key, kind, name, surface_json)
            );
            ",
        )
        .expect("legacy schema");
    connection
        .execute(
            "INSERT INTO projection_ledger (
                scope_key, kind, name, surface_json, mode, source_path, target_path,
                entry_key, source_fingerprint, target_fingerprint, applied_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                "project:/fixture",
                kind,
                "demo",
                surface,
                mode,
                "/source/skills/demo",
                "/target/.agents/skills/demo",
                Option::<String>::None,
                "source-fingerprint",
                "target-fingerprint",
                "2026-07-17T00:00:00Z",
            ],
        )
        .expect("legacy ledger record");
}

fn schema_sql(path: &Path) -> Vec<String> {
    let connection = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("open ledger schema read-only");
    let mut statement = connection
        .prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type, name")
        .expect("prepare schema query");
    statement
        .query_map([], |row| row.get(0))
        .expect("query schema")
        .collect::<Result<Vec<String>, _>>()
        .expect("collect schema")
}

fn directory_entries(path: &Path) -> Vec<PathBuf> {
    let mut entries = fs::read_dir(path)
        .expect("read ledger directory")
        .map(|entry| entry.expect("directory entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    entries
}
