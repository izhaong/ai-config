//! T010 source-first migration inventory contract.
//!
//! Every fixture is rooted in a temporary HOME. The inventory command must never inspect the
//! developer's real HOME, follow an unknown external symlink, or mutate any inspected path.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

const BIN: &str = "ai-config";
const UNKNOWN_ROOT_SENTINEL: &str = "unknown-root-content-must-not-be-read";

struct InventoryFixture {
    home: TempDir,
    asset_root: PathBuf,
}

impl InventoryFixture {
    fn new() -> Self {
        let home = TempDir::new().expect("temporary HOME");
        let asset_root = home.path().join(".ai-config");
        fs::create_dir_all(&asset_root).expect("create canonical asset root");
        Self { home, asset_root }
    }

    fn home(&self) -> &Path {
        self.home.path()
    }

    fn canonical_skill(&self, name: &str, body: &str) -> PathBuf {
        let path = self.asset_root.join("skills").join(name);
        write(&path.join("SKILL.md"), body);
        path
    }

    fn inventory_output(&self) -> std::process::Output {
        let mut command = Command::cargo_bin(BIN).expect("CLI binary");
        command
            .env("HOME", self.home())
            .env("USERPROFILE", self.home())
            .env_remove("AI_CONFIG_ROOT")
            .env_remove("AI_CONFIG_SECRETS_DIR")
            .env_remove("HERMES_SKILLS_DIR")
            .arg("--root")
            .arg(&self.asset_root)
            .args(["migrate", "inventory", "--json"])
            .output()
            .expect("run migration inventory")
    }

    fn inventory(&self) -> Value {
        let output = self.inventory_output();
        assert!(
            output.status.success(),
            "migration inventory must be a successful read-only command: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        let report: Value =
            serde_json::from_slice(&output.stdout).expect("migration inventory JSON");
        assert_eq!(
            report["schema_version"],
            Value::from(1),
            "inventory schema must be explicitly versioned"
        );
        assert!(
            report["plan_digest"]
                .as_str()
                .is_some_and(|value| !value.is_empty()),
            "inventory must expose a deterministic review digest: {report:?}"
        );
        assert!(
            report["entries"].is_array(),
            "inventory must expose structured entries: {report:?}"
        );
        report
    }
}

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture parent");
    fs::write(path, content).expect("write fixture");
}

fn copy_skill(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create copied skill");
    fs::copy(source.join("SKILL.md"), destination.join("SKILL.md")).expect("copy SKILL.md");
}

fn entry_named<'a>(report: &'a Value, name: &str, path_fragment: &str) -> &'a Value {
    report["entries"]
        .as_array()
        .expect("inventory entries")
        .iter()
        .find(|entry| {
            entry["name"] == name
                && entry["path"]
                    .as_str()
                    .is_some_and(|path| path.contains(path_fragment))
        })
        .unwrap_or_else(|| {
            panic!(
                "missing inventory entry name={name:?} path_fragment={path_fragment:?}: {report:?}"
            )
        })
}

fn assert_classification(entry: &Value, classification: &str, provenance: &str, reason_code: &str) {
    assert_eq!(entry["kind"], "skill", "entry={entry:?}");
    assert_eq!(entry["classification"], classification, "entry={entry:?}");
    assert_eq!(entry["provenance"], provenance, "entry={entry:?}");
    assert_eq!(entry["reason_code"], reason_code, "entry={entry:?}");
    assert!(
        entry["content_digest"].is_string()
            || matches!(classification, "broken_link" | "unsafe_link"),
        "readable entries must expose a semantic digest: {entry:?}"
    );
    assert!(
        entry["currently_consumed"].is_boolean(),
        "legacy entries must disclose whether the platform still consumes the path: {entry:?}"
    );
}

fn tree_snapshot(root: &Path) -> BTreeMap<PathBuf, SnapshotEntry> {
    let mut snapshot = BTreeMap::new();
    snapshot_tree(root, root, &mut snapshot);
    snapshot
}

fn snapshot_tree(root: &Path, path: &Path, snapshot: &mut BTreeMap<PathBuf, SnapshotEntry>) {
    let metadata = fs::symlink_metadata(path).expect("lstat fixture entry");
    let relative = path
        .strip_prefix(root)
        .expect("snapshot path under root")
        .to_path_buf();
    let file_type = metadata.file_type();
    let payload = if file_type.is_symlink() {
        SnapshotPayload::Symlink(fs::read_link(path).expect("read symlink target"))
    } else if file_type.is_file() {
        SnapshotPayload::File(fs::read(path).expect("read fixture file"))
    } else if file_type.is_dir() {
        SnapshotPayload::Directory
    } else {
        SnapshotPayload::Other
    };
    snapshot.insert(
        relative,
        SnapshotEntry {
            mode: metadata.permissions().mode(),
            payload,
        },
    );
    if !file_type.is_dir() {
        return;
    }
    let mut children = fs::read_dir(path)
        .expect("read fixture directory")
        .map(|entry| entry.expect("fixture directory entry").path())
        .collect::<Vec<_>>();
    children.sort();
    for child in children {
        snapshot_tree(root, &child, snapshot);
    }
}

#[derive(Debug, PartialEq, Eq)]
struct SnapshotEntry {
    mode: u32,
    payload: SnapshotPayload,
}

#[derive(Debug, PartialEq, Eq)]
enum SnapshotPayload {
    Directory,
    File(Vec<u8>),
    Symlink(PathBuf),
    Other,
}

#[test]
fn migration_inventory_classifies_allowlisted_legacy_external_and_unsafe_entries_without_writing() {
    let fixture = InventoryFixture::new();
    let marker_source = fixture.canonical_skill("marker-copy", "canonical marker copy\n");
    let equal_source = fixture.canonical_skill("equal-copy", "canonical equal copy\n");
    fixture.canonical_skill("different-copy", "canonical different copy\n");
    let correct_source = fixture.canonical_skill("correct-link", "canonical correct link\n");
    fixture.canonical_skill("wrong-link", "canonical wrong link\n");

    let marker_copy = fixture.home().join(".codex/skills/marker-copy");
    copy_skill(&marker_source, &marker_copy);
    write(
        &marker_copy.join(".ai-config-deploy.json"),
        r#"{"version":1,"source":"legacy-ai-config"}"#,
    );

    let equal_copy = fixture.home().join(".cursor/skills/equal-copy");
    copy_skill(&equal_source, &equal_copy);
    write(
        &fixture
            .home()
            .join(".claude/skills/different-copy/SKILL.md"),
        "different platform content\n",
    );

    let current_skills = fixture.home().join(".agents/skills");
    fs::create_dir_all(&current_skills).expect("create current skill target root");
    symlink(&correct_source, current_skills.join("correct-link"))
        .expect("create correct canonical link");

    let unknown_root = fixture.home().join("unknown-owner");
    write(
        &unknown_root.join("wrong-link/SKILL.md"),
        UNKNOWN_ROOT_SENTINEL,
    );
    symlink(
        unknown_root.join("wrong-link"),
        current_skills.join("wrong-link"),
    )
    .expect("create wrong-source link");
    symlink(
        fixture.home().join("missing-owner/broken-link"),
        current_skills.join("broken-link"),
    )
    .expect("create broken link");

    let cc_switch_skill = fixture.home().join(".cc-switch/skills/external-skill");
    write(
        &cc_switch_skill.join("SKILL.md"),
        "externally managed skill\n",
    );
    symlink(&cc_switch_skill, current_skills.join("external-skill"))
        .expect("create .cc-switch owned link");

    write(
        &fixture
            .home()
            .join(".codex/skills/.system/builtin-skill/SKILL.md"),
        "plugin builtin\n",
    );

    let before = tree_snapshot(fixture.home());
    let report = fixture.inventory();
    assert_eq!(
        tree_snapshot(fixture.home()),
        before,
        "inventory must not mutate canonical, platform, external, or plugin paths"
    );

    assert_classification(
        entry_named(&report, "marker-copy", ".codex/skills"),
        "legacy_marker_candidate",
        "platform_legacy",
        "legacy_marker_present",
    );
    let marker_entry = entry_named(&report, "marker-copy", ".codex/skills");
    assert_eq!(marker_entry["ownership_state"], "foreign");
    assert_eq!(marker_entry["owned"], false);
    assert_eq!(marker_entry["selectable"], false);
    assert_classification(
        entry_named(&report, "equal-copy", ".cursor/skills"),
        "equivalent",
        "platform_legacy",
        "unmarked_equal_copy",
    );
    assert_classification(
        entry_named(&report, "different-copy", ".claude/skills"),
        "foreign",
        "platform_current",
        "different_content",
    );
    assert_classification(
        entry_named(&report, "correct-link", ".agents/skills"),
        "managed_link",
        "platform_current",
        "canonical_symlink",
    );
    let correct_link = entry_named(&report, "correct-link", ".agents/skills");
    assert_eq!(correct_link["ownership_state"], "managed_link");
    assert_eq!(correct_link["owned"], true);
    assert_eq!(correct_link["followed"], false);
    assert_classification(
        entry_named(&report, "wrong-link", ".agents/skills"),
        "unsafe_link",
        "platform_current",
        "unknown_root_link",
    );
    assert_classification(
        entry_named(&report, "broken-link", ".agents/skills"),
        "broken_link",
        "platform_current",
        "broken_symlink",
    );
    assert_classification(
        entry_named(&report, "external-skill", ".cc-switch/skills"),
        "external_owned",
        "cc_switch",
        "cc_switch_owned",
    );
    assert_classification(
        entry_named(&report, "builtin-skill", ".codex/skills/.system"),
        "external_owned",
        "plugin_builtin",
        "plugin_or_builtin",
    );
    let builtin_entries = report["entries"]
        .as_array()
        .expect("inventory entries")
        .iter()
        .filter(|entry| entry["name"] == "builtin-skill")
        .collect::<Vec<_>>();
    assert_eq!(
        builtin_entries.len(),
        1,
        "builtin must not be duplicated as a legacy candidate: {report:?}"
    );
    assert_eq!(builtin_entries[0]["owned"], false);
    assert_eq!(builtin_entries[0]["selectable"], false);
}

#[test]
fn migration_inventory_detects_case_only_collisions_and_digest_includes_dotfiles() {
    let fixture = InventoryFixture::new();
    fixture.canonical_skill("CaseSkill", "same visible content\n");
    let legacy = fixture.home().join(".cursor/skills/caseskill");
    write(&legacy.join("SKILL.md"), "same visible content\n");
    write(&legacy.join(".hidden"), "first hidden value\n");

    let first = fixture.inventory();
    let first_entry = entry_named(&first, "caseskill", ".cursor/skills");
    assert_eq!(first_entry["classification"], "case_collision");
    assert_eq!(first_entry["reason_code"], "case_only_name_collision");
    assert_eq!(first_entry["blocking"], true);
    let first_digest = first_entry["content_digest"]
        .as_str()
        .expect("legacy directory content digest")
        .to_owned();

    write(&legacy.join(".hidden"), "second hidden value\n");
    let second = fixture.inventory();
    let second_entry = entry_named(&second, "caseskill", ".cursor/skills");
    let second_digest = second_entry["content_digest"]
        .as_str()
        .expect("updated legacy directory content digest");

    assert_ne!(
        first_digest, second_digest,
        "dotfiles must participate in the semantic inventory digest"
    );
    assert_ne!(
        first["plan_digest"], second["plan_digest"],
        "a content-precondition change must alter the review digest"
    );
}

#[test]
fn migration_inventory_scans_only_allowlisted_roots_and_never_follows_unknown_symlinks() {
    let fixture = InventoryFixture::new();
    fixture.canonical_skill("canonical", "canonical\n");
    write(
        &fixture.home().join(".codex/skills/known-legacy/SKILL.md"),
        "known legacy\n",
    );
    write(
        &fixture
            .home()
            .join(".codex/skills/.system/known-plugin/SKILL.md"),
        "known plugin\n",
    );
    write(
        &fixture
            .home()
            .join(".random-ai/skills/ignored-random/SKILL.md"),
        "must not be inventoried\n",
    );
    write(
        &fixture
            .home()
            .join(".codex/not-a-contract-root/ignored-codex/SKILL.md"),
        "must not be inventoried\n",
    );

    let outside = fixture.home().join("unknown-owner/escaped-tree");
    write(
        &outside.join("nested/escaped-skill/SKILL.md"),
        UNKNOWN_ROOT_SENTINEL,
    );
    let legacy_root = fixture.home().join(".cursor/skills");
    fs::create_dir_all(&legacy_root).expect("create legacy root");
    symlink(&outside, legacy_root.join("escape")).expect("create unknown-root symlink");

    let output = fixture.inventory_output();
    assert!(
        output.status.success(),
        "inventory must report unsafe links without traversing them: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 inventory JSON");
    assert!(
        !stdout.contains(UNKNOWN_ROOT_SENTINEL),
        "unknown-root link contents must never be read or serialized"
    );
    let report: Value = serde_json::from_str(&stdout).expect("inventory JSON");
    let serialized = serde_json::to_string(&report["entries"]).expect("serialize entries");
    assert!(serialized.contains("known-legacy"));
    assert!(serialized.contains("known-plugin"));
    assert!(!serialized.contains("ignored-random"));
    assert!(!serialized.contains("ignored-codex"));
    assert!(!serialized.contains("escaped-skill"));

    let unsafe_entry = entry_named(&report, "escape", ".cursor/skills");
    assert_eq!(unsafe_entry["classification"], "unsafe_link");
    assert_eq!(unsafe_entry["reason_code"], "unknown_root_link");
    assert_eq!(unsafe_entry["ownership_state"], "foreign");
    assert_eq!(unsafe_entry["content_digest"], Value::Null);
    assert_eq!(unsafe_entry["followed"], false);
    assert_eq!(unsafe_entry["owned"], false);
    assert_eq!(unsafe_entry["selectable"], false);
}

#[test]
fn unknown_root_symlink_is_lstat_only_even_when_target_content_is_unreadable() {
    let fixture = InventoryFixture::new();
    fixture.canonical_skill("blocked", "canonical\n");
    let outside = TempDir::new().expect("external unknown root");
    let target = outside.path().join("blocked");
    write(&target.join("SKILL.md"), UNKNOWN_ROOT_SENTINEL);
    fs::set_permissions(target.join("SKILL.md"), fs::Permissions::from_mode(0o000))
        .expect("make unknown content unreadable");
    let current = fixture.home().join(".agents/skills");
    fs::create_dir_all(&current).expect("create current skills root");
    symlink(&target, current.join("blocked")).expect("link to unreadable unknown root");

    let report = fixture.inventory();
    let entry = entry_named(&report, "blocked", ".agents/skills");
    assert_eq!(entry["classification"], "unsafe_link");
    assert_eq!(entry["ownership_state"], "foreign");
    assert_eq!(entry["content_digest"], Value::Null);
    assert_eq!(entry["followed"], false);
    assert_eq!(entry["owned"], false);
    assert_eq!(entry["selectable"], false);
}

#[test]
fn cc_switch_platform_link_is_external_owned_but_never_ai_config_owned_or_selectable() {
    let fixture = InventoryFixture::new();
    let external = fixture.home().join(".cc-switch/skills/shared");
    write(&external.join("SKILL.md"), "cc-switch managed\n");
    let current = fixture.home().join(".agents/skills");
    fs::create_dir_all(&current).expect("create current skills root");
    symlink(&external, current.join("shared")).expect("link current target to cc-switch");

    let report = fixture.inventory();
    let platform_entry = entry_named(&report, "shared", ".agents/skills");
    assert_eq!(platform_entry["classification"], "external_owned");
    assert_eq!(platform_entry["ownership_state"], "foreign");
    assert_eq!(platform_entry["reason_code"], "cc_switch_owned");
    assert_eq!(platform_entry["content_digest"], Value::Null);
    assert_eq!(platform_entry["followed"], false);
    assert_eq!(platform_entry["owned"], false);
    assert_eq!(platform_entry["selectable"], false);
}

#[test]
fn migration_inventory_is_strictly_read_only_and_deterministic() {
    let fixture = InventoryFixture::new();
    let canonical = fixture.canonical_skill("stable", "stable canonical content\n");
    let legacy = fixture.home().join(".cursor/skills/stable");
    copy_skill(&canonical, &legacy);
    write(&legacy.join(".hidden"), "stable hidden content\n");
    fs::set_permissions(legacy.join("SKILL.md"), fs::Permissions::from_mode(0o640))
        .expect("set fixture mode");

    let before = tree_snapshot(fixture.home());
    let first = fixture.inventory();
    let after_first = tree_snapshot(fixture.home());
    let second = fixture.inventory();
    let after_second = tree_snapshot(fixture.home());

    assert_eq!(after_first, before, "first inventory must be zero-write");
    assert_eq!(
        after_second, before,
        "repeated inventory must be zero-write"
    );
    assert_eq!(
        first, second,
        "unchanged inventory must preserve action ordering, fingerprints, and plan digest"
    );
    assert!(
        !fixture.home().join(".config/ai-config/apply.lock").exists()
            && !fixture.home().join(".config/ai-config/backups").exists()
            && !fixture.home().join(".config/ai-config/state.db").exists(),
        "inventory must not create locks, backups, or state databases"
    );
}
