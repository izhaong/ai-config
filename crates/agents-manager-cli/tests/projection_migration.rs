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

const BIN: &str = "agents-manager";
const UNKNOWN_ROOT_SENTINEL: &str = "unknown-root-content-must-not-be-read";

struct InventoryFixture {
    home: TempDir,
    asset_root: PathBuf,
}

impl InventoryFixture {
    fn new() -> Self {
        let home = TempDir::new().expect("temporary HOME");
        let asset_root = home.path().join(".agents-manager");
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
            .env_remove("AGENTS_MANAGER_ROOT")
            .env_remove("AGENTS_MANAGER_SECRETS_DIR")
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
        &marker_copy.join(".agents-manager-deploy.json"),
        r#"{"version":1,"source":"legacy-agents-manager"}"#,
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
    assert_eq!(marker_entry["currently_consumed"], false);
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
fn cc_switch_platform_link_is_external_owned_but_never_agents_manager_owned_or_selectable() {
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
        !fixture.home().join(".config/agents-manager/apply.lock").exists()
            && !fixture.home().join(".config/agents-manager/backups").exists()
            && !fixture.home().join(".config/agents-manager/state.db").exists(),
        "inventory must not create locks, backups, or state databases"
    );
}

fn rules_inventory_output(home: &Path, root: &Path) -> std::process::Output {
    let mut command = Command::cargo_bin(BIN).expect("CLI binary");
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("AGENTS_MANAGER_ROOT")
        .env_remove("AGENTS_MANAGER_SECRETS_DIR")
        .env_remove("HERMES_SKILLS_DIR")
        .arg("--root")
        .arg(root)
        .args(["migrate", "inventory", "--json"])
        .output()
        .expect("run rules migration inventory")
}

fn rules_inventory(home: &Path, root: &Path) -> Value {
    let output = rules_inventory_output(home, root);
    assert!(
        output.status.success(),
        "rules inventory must succeed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    serde_json::from_slice(&output.stdout).expect("rules inventory JSON")
}

fn assert_rule_entry_source(entry: &Value, source_layer: &str, canonical_path: &Path, scope: &str) {
    assert_eq!(entry["kind"], "rule", "entry={entry:?}");
    assert_eq!(entry["source_layer"], source_layer, "entry={entry:?}");
    assert_eq!(
        entry["canonical_path"],
        canonical_path.to_string_lossy().as_ref(),
        "entry={entry:?}"
    );
    assert_eq!(entry["scope"], scope, "entry={entry:?}");
}

#[test]
fn global_rules_inventory_respects_current_legacy_and_unsupported_platform_contracts() {
    let home = TempDir::new().expect("temporary HOME");
    let asset_root = home.path().join(".agents-manager");
    write(
        &asset_root.join("skills/Demo/SKILL.md"),
        "same spelling in another asset kind\n",
    );
    let lowercase_rule = asset_root.join("rules/demo.mdc");
    write(&lowercase_rule, "rule names collide only within Rule\n");
    let equal_source = asset_root.join("rules/equal.mdc");
    let different_source = asset_root.join("rules/different.mdc");
    let correct_source = asset_root.join("rules/correct.mdc");
    let unknown_source = asset_root.join("rules/unknown.mdc");
    let policy_source = asset_root.join("rules/policy.mdc");
    write(&equal_source, "equal rule\n");
    write(&different_source, "canonical different rule\n");
    write(&correct_source, "correct linked rule\n");
    write(&unknown_source, "canonical unknown rule\n");
    write(&policy_source, "instruction rule, not execution policy\n");

    write(&home.path().join(".claude/rules/equal.md"), "equal rule\n");
    write(
        &home.path().join(".claude/rules/different.md"),
        "foreign different rule\n",
    );
    fs::create_dir_all(home.path().join(".claude/rules")).expect("create Claude rules root");
    symlink(
        &correct_source,
        home.path().join(".claude/rules/correct.md"),
    )
    .expect("create correct Claude rule link");

    let outside = TempDir::new().expect("unknown external owner");
    let outside_rule = outside.path().join("unknown.md");
    write(&outside_rule, "unknown external rule\n");
    fs::set_permissions(&outside_rule, fs::Permissions::from_mode(0o000))
        .expect("make unknown target unreadable");
    symlink(&outside_rule, home.path().join(".claude/rules/unknown.md"))
        .expect("create unknown Claude rule link");

    write(
        &home.path().join(".codex/rules/policy.rules"),
        "allow_prefix(command = [\"git\", \"status\"])\n",
    );
    write(
        &home.path().join(".cursor/rules/must-not-scan.mdc"),
        "global Cursor rules have no supported file target\n",
    );

    let report = rules_inventory(home.path(), &asset_root);
    let skill = entry_named(&report, "Demo", "/skills/");
    let rule = entry_named(&report, "demo", "/rules/");
    assert_ne!(skill["classification"], "case_collision");
    assert_ne!(rule["classification"], "case_collision");
    let equal = entry_named(&report, "equal", ".claude/rules");
    assert_eq!(equal["classification"], "equivalent");
    assert_eq!(equal["ownership_state"], "equivalent");
    assert_eq!(equal["reason_code"], "unmarked_equal_copy");
    assert_rule_entry_source(equal, "global", &equal_source, "global");

    let different = entry_named(&report, "different", ".claude/rules");
    assert_eq!(different["classification"], "foreign");
    assert_eq!(different["ownership_state"], "foreign");
    assert_eq!(different["reason_code"], "different_content");
    assert_rule_entry_source(different, "global", &different_source, "global");

    let correct = entry_named(&report, "correct", ".claude/rules");
    assert_eq!(correct["classification"], "managed_link");
    assert_eq!(correct["ownership_state"], "managed_link");
    assert_eq!(correct["reason_code"], "canonical_symlink");
    assert_eq!(correct["followed"], false);
    assert_rule_entry_source(correct, "global", &correct_source, "global");

    let unknown = entry_named(&report, "unknown", ".claude/rules");
    assert_eq!(unknown["classification"], "unsafe_link");
    assert_eq!(unknown["ownership_state"], "foreign");
    assert_eq!(unknown["reason_code"], "unknown_root_link");
    assert_eq!(unknown["content_digest"], Value::Null);
    assert_eq!(unknown["followed"], false);
    assert_eq!(unknown["owned"], false);
    assert_eq!(unknown["selectable"], false);
    assert_rule_entry_source(unknown, "global", &unknown_source, "global");

    let policy = entry_named(&report, "policy", ".codex/rules");
    assert_eq!(policy["classification"], "external_owned");
    assert_eq!(policy["ownership_state"], "foreign");
    assert_eq!(policy["provenance"], "platform_legacy");
    assert_eq!(policy["reason_code"], "codex_execution_policy");
    assert_eq!(policy["currently_consumed"], true);
    assert_eq!(policy["owned"], false);
    assert_eq!(policy["selectable"], false);
    assert_eq!(policy["followed"], false);
    assert_rule_entry_source(policy, "global", &policy_source, "global");

    let entries = serde_json::to_string(&report["entries"]).expect("serialize rule entries");
    assert!(
        !entries.contains(".cursor/rules/must-not-scan.mdc"),
        "global Cursor rules are unsupported and must not enter inventory"
    );
    assert_eq!(report["scope"], "global");
}

#[test]
fn project_rules_inventory_uses_project_overlay_and_never_scans_home_platform_paths() {
    let home = TempDir::new().expect("temporary HOME");
    let repo = TempDir::new().expect("temporary project");
    let global_source = home.path().join(".agents-manager/rules/shared.mdc");
    let global_only_source = home.path().join(".agents-manager/rules/global-only.mdc");
    let project_source = repo.path().join(".agents-manager/rules/shared.mdc");
    write(&global_source, "global shared rule\n");
    write(&global_only_source, "global inherited rule\n");
    write(&project_source, "project shared rule\n");

    write(
        &repo.path().join(".cursor/rules/shared.mdc"),
        "project shared rule\n",
    );
    write(
        &repo.path().join(".cursor/rules/global-only.mdc"),
        "global inherited rule\n",
    );
    write(
        &repo.path().join(".claude/rules/shared.md"),
        "project shared rule\n",
    );
    write(
        &repo.path().join(".codex/rules/project-policy.rules"),
        "allow_prefix(command = [\"cargo\", \"test\"])\n",
    );
    write(
        &repo
            .path()
            .join(".cc-switch/skills/project-external/SKILL.md"),
        "project scope must not scan user-only external roots\n",
    );
    write(
        &repo
            .path()
            .join(".codex/skills/.system/project-builtin/SKILL.md"),
        "project scope must not scan user-only builtin roots\n",
    );

    write(
        &home.path().join(".cursor/rules/shared.mdc"),
        "HOME platform rule must not be scanned\n",
    );
    write(
        &home.path().join(".claude/rules/shared.md"),
        "HOME Claude rule must not be scanned\n",
    );
    write(
        &home.path().join(".codex/rules/project-policy.rules"),
        "HOME Codex policy must not be scanned\n",
    );

    let report = rules_inventory(home.path(), repo.path());
    let cursor = entry_named(&report, "shared", ".cursor/rules");
    assert_eq!(cursor["classification"], "equivalent");
    assert_eq!(cursor["ownership_state"], "equivalent");
    assert_eq!(
        cursor["consumers"],
        serde_json::json!(["cursor", "hermes"]),
        "project Cursor rule target is shared with Hermes"
    );
    assert_rule_entry_source(cursor, "project", &project_source, "project");
    let inherited = entry_named(&report, "global-only", ".cursor/rules");
    assert_eq!(inherited["classification"], "equivalent");
    assert_rule_entry_source(inherited, "global", &global_only_source, "project");

    let claude = entry_named(&report, "shared", ".claude/rules");
    assert_eq!(claude["classification"], "equivalent");
    assert_eq!(claude["ownership_state"], "equivalent");
    assert_eq!(claude["consumers"], serde_json::json!(["claude"]));
    assert_rule_entry_source(claude, "project", &project_source, "project");

    let policy = entry_named(&report, "project-policy", ".codex/rules");
    assert_eq!(policy["classification"], "external_owned");
    assert_eq!(policy["ownership_state"], "foreign");
    assert_eq!(policy["reason_code"], "codex_execution_policy");
    assert_eq!(policy["owned"], false);
    assert_eq!(policy["selectable"], false);
    assert_eq!(policy["scope"], "project");

    for entry in report["entries"].as_array().expect("project entries") {
        let Some(path) = entry["path"].as_str() else {
            continue;
        };
        if path.contains("/.cursor/rules/")
            || path.contains("/.claude/rules/")
            || path.contains("/.codex/rules/")
        {
            assert!(
                Path::new(path).starts_with(repo.path()),
                "project inventory must not fall back to HOME platform paths: {entry:?}"
            );
        }
    }
    let serialized = serde_json::to_string(&report["entries"]).expect("serialize project entries");
    assert!(!serialized.contains("project-external"));
    assert!(!serialized.contains("project-builtin"));
    assert_eq!(report["scope"], "project");
    assert_eq!(
        cursor["canonical_path"],
        project_source.to_string_lossy().as_ref(),
        "project same-name source must override the global source"
    );
    assert_ne!(
        cursor["canonical_path"],
        global_source.to_string_lossy().as_ref()
    );
    let canonical_shared = report["entries"]
        .as_array()
        .expect("project entries")
        .iter()
        .filter(|entry| {
            entry["kind"] == "rule"
                && entry["name"] == "shared"
                && entry["provenance"] == "canonical"
        })
        .collect::<Vec<_>>();
    assert_eq!(
        canonical_shared.len(),
        2,
        "raw inventory must retain both global and project canonical provenance: {report:?}"
    );
    let layers = canonical_shared
        .iter()
        .map(|entry| entry["source_layer"].as_str().unwrap_or_default())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        layers,
        std::collections::BTreeSet::from(["global", "project"])
    );
}

const MCP_SECRET_SENTINEL: &str = "t010-mcp-inventory-secret-must-never-serialize";

fn write_canonical_mcp(
    asset_root: &Path,
    file_name: &str,
    server_name: &str,
    command: &str,
) -> PathBuf {
    let path = asset_root
        .join("mcp/servers")
        .join(format!("{file_name}.json"));
    write(
        &path,
        &format!(
            r#"{{
  "name": "{server_name}",
  "enabled": true,
  "targets": ["cursor", "codex", "claude", "hermes"],
  "transport": "stdio",
  "config": {{
    "command": "{command}",
    "args": [],
    "env": {{ "CATALOG_TOKEN": "${{CATALOG_TOKEN}}" }}
  }}
}}"#
        ),
    );
    path
}

fn assert_mcp_source(entry: &Value, source_layer: &str, canonical_path: &Path, scope: &str) {
    assert_eq!(entry["kind"], "mcp", "entry={entry:?}");
    assert_eq!(entry["source_layer"], source_layer, "entry={entry:?}");
    assert_eq!(
        entry["canonical_path"],
        canonical_path.to_string_lossy().as_ref(),
        "entry={entry:?}"
    );
    assert_eq!(entry["scope"], scope, "entry={entry:?}");
    assert_eq!(
        entry["secret_keys"],
        serde_json::json!(["CATALOG_TOKEN"]),
        "inventory may expose referenced key names, never values: {entry:?}"
    );
}

fn assert_named_mcp_container_entry(
    report: &Value,
    name: &str,
    path_fragment: &str,
    entry_key: &str,
    format: &str,
    consumer: &str,
) -> Value {
    let entry = entry_named(report, name, path_fragment).clone();
    assert_eq!(entry["entry_key"], entry_key, "entry={entry:?}");
    assert_eq!(entry["format"], format, "entry={entry:?}");
    assert_eq!(entry["consumers"], serde_json::json!([consumer]));
    assert_eq!(entry["classification"], "foreign", "entry={entry:?}");
    assert_eq!(entry["ownership_state"], "foreign", "entry={entry:?}");
    assert_eq!(entry["owned"], false, "entry={entry:?}");
    assert_eq!(entry["selectable"], false, "entry={entry:?}");
    assert_eq!(
        entry["blocking"], true,
        "an unowned generated entry that shadows canonical source must block takeover: {entry:?}"
    );
    assert!(
        entry["rendered_config"].is_null() && entry["config"].is_null(),
        "inventory must not serialize platform MCP bodies: {entry:?}"
    );
    entry
}

fn assert_mcp_issue<'a>(report: &'a Value, path_fragment: &str, reason_code: &str) -> &'a Value {
    let issue = report["issues"]
        .as_array()
        .expect("inventory issues")
        .iter()
        .find(|issue| {
            issue["path"]
                .as_str()
                .is_some_and(|path| path.contains(path_fragment))
                && issue["reason_code"] == reason_code
        })
        .unwrap_or_else(|| {
            panic!(
                "missing MCP issue path_fragment={path_fragment:?} reason={reason_code:?}: {report:?}"
            )
        });
    assert_eq!(issue["kind"], "mcp");
    assert_eq!(issue["blocking"], true);
    assert!(
        issue["message"].is_null() && issue["config"].is_null() && issue["content"].is_null(),
        "issues must never retain parser or config payloads: {issue:?}"
    );
    issue
}

#[test]
fn global_mcp_inventory_is_per_server_lossless_across_current_and_legacy_containers() {
    let home = TempDir::new().expect("temporary HOME");
    let asset_root = home.path().join(".agents-manager");
    let catalog_source = write_canonical_mcp(&asset_root, "catalog", "catalog", "catalog-global");

    write(
        &home.path().join(".cursor/mcp.json"),
        &format!(
            r#"{{
  "foreignTopLevel": {{ "preserve": true }},
  "mcpServers": {{
    "catalog": {{
      "command": "catalog-global",
      "env": {{ "CATALOG_TOKEN": "{MCP_SECRET_SENTINEL}" }}
    }},
    "foreign-cursor": {{ "command": "foreign-cursor", "unknown": 1 }}
  }}
}}"#
        ),
    );
    write(
        &home.path().join(".codex/config.toml"),
        &format!(
            r#"model = "gpt-test"

[mcp_servers.catalog]
command = "catalog-global"

[mcp_servers.catalog.env]
CATALOG_TOKEN = "{MCP_SECRET_SENTINEL}"

[mcp_servers.foreign_codex]
command = "foreign-codex"
unknown = "keep"
"#
        ),
    );
    write(
        &home.path().join(".claude.json"),
        &format!(
            r#"{{
  "theme": "dark",
  "projects": {{
    "/private/project": {{
      "mcpServers": {{
        "claude-local-only": {{ "command": "{MCP_SECRET_SENTINEL}" }}
      }}
    }}
  }},
  "mcpServers": {{
    "catalog": {{
      "command": "catalog-global",
      "env": {{ "CATALOG_TOKEN": "{MCP_SECRET_SENTINEL}" }}
    }},
    "foreign-claude": {{ "command": "foreign-claude", "futureField": true }}
  }}
}}"#
        ),
    );
    write(
        &home.path().join(".hermes/config.yaml"),
        &format!(
            r#"theme: dark
mcp_servers:
  catalog:
    command: catalog-global
    env:
      CATALOG_TOKEN: "{MCP_SECRET_SENTINEL}"
  foreign-hermes:
    command: foreign-hermes
    future_field: keep
"#
        ),
    );

    write(
        &home.path().join(".codex/mcp.json"),
        &format!(
            r#"{{
  "mcpServers": {{
    "catalog": {{ "command": "legacy-catalog" }},
    "legacy-only": {{
      "command": "legacy-only",
      "env": {{ "LEGACY_TOKEN": "{MCP_SECRET_SENTINEL}" }}
    }}
  }}
}}"#
        ),
    );
    write(
        &home.path().join(".claude/mcp.json"),
        r#"{
  "mcpServers": {
    "legacy-claude-only": { "command": "legacy-claude-only" }
  }
}"#,
    );
    write(
        &home.path().join(".hermes/mcp.json"),
        r#"{
  "mcpServers": {
    "legacy-hermes-only": { "command": "legacy-hermes-only" }
  }
}"#,
    );
    write(
        &asset_root.join("mcp.json"),
        r#"{
  "mcpServers": {
    "catalog": { "command": "old-source-catalog" },
    "source-legacy-only": { "command": "source-legacy-only" }
  }
}"#,
    );

    let before = tree_snapshot(home.path());
    let output = rules_inventory_output(home.path(), &asset_root);
    assert!(
        output.status.success(),
        "global MCP inventory must succeed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 MCP inventory JSON");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 MCP inventory stderr");
    assert!(
        !stdout.contains(MCP_SECRET_SENTINEL) && !stderr.contains(MCP_SECRET_SENTINEL),
        "MCP inventory must never serialize literal platform secret values"
    );
    assert_eq!(
        tree_snapshot(home.path()),
        before,
        "MCP inventory must be strictly read-only"
    );
    let report: Value = serde_json::from_str(&stdout).expect("global MCP inventory JSON");

    let cursor = assert_named_mcp_container_entry(
        &report,
        "catalog",
        ".cursor/mcp.json",
        "mcpServers.catalog",
        "json",
        "cursor",
    );
    assert_eq!(cursor["provenance"], "platform_current");
    assert_eq!(cursor["currently_consumed"], true);
    assert_mcp_source(&cursor, "global", &catalog_source, "global");

    let codex = assert_named_mcp_container_entry(
        &report,
        "catalog",
        ".codex/config.toml",
        "mcp_servers.catalog",
        "toml",
        "codex",
    );
    assert_eq!(codex["provenance"], "platform_current");
    assert_eq!(codex["currently_consumed"], true);
    assert_eq!(codex["trust_requirement"], "none");
    assert_mcp_source(&codex, "global", &catalog_source, "global");

    let claude = assert_named_mcp_container_entry(
        &report,
        "catalog",
        ".claude.json",
        "mcpServers.catalog",
        "json",
        "claude",
    );
    assert_eq!(claude["provenance"], "platform_current");
    assert_eq!(claude["currently_consumed"], true);
    assert_mcp_source(&claude, "global", &catalog_source, "global");

    let hermes = assert_named_mcp_container_entry(
        &report,
        "catalog",
        ".hermes/config.yaml",
        "mcp_servers.catalog",
        "yaml",
        "hermes",
    );
    assert_eq!(hermes["provenance"], "platform_current");
    assert_eq!(hermes["currently_consumed"], true);
    assert_mcp_source(&hermes, "global", &catalog_source, "global");

    for (name, path_fragment) in [
        ("foreign-cursor", ".cursor/mcp.json"),
        ("foreign_codex", ".codex/config.toml"),
        ("foreign-claude", ".claude.json"),
        ("foreign-hermes", ".hermes/config.yaml"),
    ] {
        let entry = entry_named(&report, name, path_fragment);
        assert_eq!(entry["kind"], "mcp", "entry={entry:?}");
        assert_eq!(entry["classification"], "foreign", "entry={entry:?}");
        assert_eq!(entry["ownership_state"], "foreign", "entry={entry:?}");
        assert_eq!(entry["source_layer"], Value::Null, "entry={entry:?}");
        assert_eq!(entry["canonical_path"], Value::Null, "entry={entry:?}");
        assert_eq!(entry["owned"], false, "entry={entry:?}");
        assert_eq!(entry["selectable"], false, "entry={entry:?}");
        assert_eq!(entry["currently_consumed"], true, "entry={entry:?}");
        assert_eq!(entry["provenance"], "platform_current", "entry={entry:?}");
    }
    assert!(
        !serde_json::to_string(&report)
            .expect("serialize MCP inventory")
            .contains("claude-local-only"),
        "Claude local project state in ~/.claude.json is inventory-only and must not be treated as user MCP"
    );

    let legacy_codex = entry_named(&report, "legacy-only", ".codex/mcp.json");
    assert_eq!(legacy_codex["kind"], "mcp");
    assert_eq!(legacy_codex["provenance"], "platform_legacy");
    assert_eq!(legacy_codex["reason_code"], "legacy_codex_mcp_json");
    assert_eq!(legacy_codex["currently_consumed"], false);
    assert_eq!(legacy_codex["classification"], "foreign");
    assert_eq!(legacy_codex["ownership_state"], "foreign");
    assert_eq!(legacy_codex["owned"], false);
    assert_eq!(legacy_codex["selectable"], false);

    for (name, fragment, reason) in [
        (
            "legacy-claude-only",
            ".claude/mcp.json",
            "legacy_claude_mcp_json",
        ),
        (
            "legacy-hermes-only",
            ".hermes/mcp.json",
            "legacy_hermes_mcp_json",
        ),
    ] {
        let legacy = entry_named(&report, name, fragment);
        assert_eq!(legacy["kind"], "mcp");
        assert_eq!(legacy["provenance"], "platform_legacy");
        assert_eq!(legacy["reason_code"], reason);
        assert_eq!(legacy["currently_consumed"], false);
        assert_eq!(legacy["ownership_state"], "foreign");
        assert_eq!(legacy["owned"], false);
        assert_eq!(legacy["selectable"], false);
    }

    let legacy_source = entry_named(&report, "source-legacy-only", "/.agents-manager/mcp.json");
    assert_eq!(legacy_source["kind"], "mcp");
    assert_eq!(legacy_source["classification"], "legacy_mcp_candidate");
    assert_eq!(legacy_source["provenance"], "canonical_legacy");
    assert_eq!(legacy_source["reason_code"], "legacy_monolithic_mcp");
    assert_eq!(legacy_source["currently_consumed"], false);
    assert_eq!(legacy_source["owned"], false);
    assert_eq!(legacy_source["selectable"], false);
}

#[test]
fn project_mcp_inventory_uses_effective_overlay_repo_containers_and_reports_hermes_unsupported() {
    let home = TempDir::new().expect("temporary HOME");
    let repo = TempDir::new().expect("temporary project");
    let global_root = home.path().join(".agents-manager");
    let project_root = repo.path().join(".agents-manager");
    let global_catalog = write_canonical_mcp(&global_root, "catalog", "catalog", "catalog-global");
    let global_only =
        write_canonical_mcp(&global_root, "global-only", "global-only", "global-only");
    let project_catalog =
        write_canonical_mcp(&project_root, "catalog", "catalog", "catalog-project");
    write_canonical_mcp(
        &project_root,
        "project-only",
        "project-only",
        "project-only",
    );
    write_canonical_mcp(&global_root, "CaseServer", "CaseServer", "case-global");
    write_canonical_mcp(&project_root, "caseserver", "caseserver", "case-project");
    write(
        &project_root.join("skills/CASESERver/SKILL.md"),
        "cross-kind spelling must not create an MCP collision\n",
    );

    write(
        &repo.path().join(".cursor/mcp.json"),
        r#"{
  "projectUnknown": true,
  "mcpServers": {
    "catalog": { "command": "catalog-project" },
    "global-only": { "command": "global-only" },
    "project-only": { "command": "project-only" },
    "foreign-project": { "command": "foreign-project", "future": true }
  }
}"#,
    );
    write(
        &repo.path().join(".codex/config.toml"),
        r#"project_setting = "preserve"

[mcp_servers.catalog]
command = "catalog-project"

[mcp_servers.global-only]
command = "global-only"
"#,
    );
    write(
        &repo.path().join(".mcp.json"),
        r#"{
  "mcpServers": {
    "catalog": { "command": "catalog-project" },
    "project-only": { "command": "project-only" }
  }
}"#,
    );
    write(
        &repo.path().join(".hermes/config.yaml"),
        "mcp_servers:\n  repo-guess:\n    command: must-not-scan\n",
    );

    write(
        &home.path().join(".cursor/mcp.json"),
        &format!("invalid-json {MCP_SECRET_SENTINEL}"),
    );
    write(
        &home.path().join(".codex/config.toml"),
        &format!("[invalid-toml {MCP_SECRET_SENTINEL}"),
    );
    write(
        &home.path().join(".claude.json"),
        &format!("invalid-json {MCP_SECRET_SENTINEL}"),
    );
    write(
        &home.path().join(".hermes/config.yaml"),
        &format!("mcp_servers: [ {MCP_SECRET_SENTINEL}"),
    );

    let home_before = tree_snapshot(home.path());
    let repo_before = tree_snapshot(repo.path());
    let output = rules_inventory_output(home.path(), repo.path());
    assert!(
        output.status.success(),
        "project MCP inventory must succeed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 project MCP inventory JSON");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 project MCP inventory stderr");
    assert!(
        !stdout.contains(MCP_SECRET_SENTINEL) && !stderr.contains(MCP_SECRET_SENTINEL),
        "project inventory must not read or serialize HOME platform MCP values"
    );
    assert_eq!(
        tree_snapshot(home.path()),
        home_before,
        "HOME must remain unchanged"
    );
    assert_eq!(
        tree_snapshot(repo.path()),
        repo_before,
        "project must remain unchanged"
    );
    let report: Value = serde_json::from_str(&stdout).expect("project MCP inventory JSON");
    assert_eq!(report["scope"], "project");
    for issue in report["issues"]
        .as_array()
        .expect("project inventory issues")
    {
        assert!(
            issue["path"]
                .as_str()
                .is_some_and(|path| Path::new(path).starts_with(repo.path())),
            "project inventory issues must never disclose that HOME platform paths were scanned: {issue:?}"
        );
    }

    let cursor_catalog = assert_named_mcp_container_entry(
        &report,
        "catalog",
        ".cursor/mcp.json",
        "mcpServers.catalog",
        "json",
        "cursor",
    );
    assert_mcp_source(&cursor_catalog, "project", &project_catalog, "project");
    assert_ne!(
        cursor_catalog["canonical_path"],
        global_catalog.to_string_lossy().as_ref(),
        "project same-name MCP must override global"
    );

    let inherited = assert_named_mcp_container_entry(
        &report,
        "global-only",
        ".cursor/mcp.json",
        "mcpServers.global-only",
        "json",
        "cursor",
    );
    assert_mcp_source(&inherited, "global", &global_only, "project");

    let codex_catalog = assert_named_mcp_container_entry(
        &report,
        "catalog",
        ".codex/config.toml",
        "mcp_servers.catalog",
        "toml",
        "codex",
    );
    assert_mcp_source(&codex_catalog, "project", &project_catalog, "project");
    assert_eq!(codex_catalog["trust_requirement"], "trusted_project");

    let claude_catalog = assert_named_mcp_container_entry(
        &report,
        "catalog",
        "/.mcp.json",
        "mcpServers.catalog",
        "json",
        "claude",
    );
    assert_mcp_source(&claude_catalog, "project", &project_catalog, "project");

    let foreign = entry_named(&report, "foreign-project", ".cursor/mcp.json");
    assert_eq!(foreign["classification"], "foreign");
    assert_eq!(foreign["source_layer"], Value::Null);
    assert_eq!(foreign["canonical_path"], Value::Null);
    assert_eq!(foreign["owned"], false);

    for entry in report["entries"].as_array().expect("project MCP entries") {
        let Some(path) = entry["path"].as_str() else {
            continue;
        };
        if entry["kind"] == "mcp"
            && (path.contains("/.cursor/mcp.json")
                || path.contains("/.codex/config.toml")
                || path.ends_with("/.mcp.json")
                || path.contains("/.claude.json")
                || path.contains("/.hermes/"))
        {
            assert!(
                Path::new(path).starts_with(repo.path()),
                "project inventory must never fall back to HOME MCP paths: {entry:?}"
            );
        }
        assert!(
            !path.contains("/.hermes/"),
            "Hermes project has no MCP inventory target: {entry:?}"
        );
    }

    let unsupported = report["unsupported"]
        .as_array()
        .expect("project inventory must disclose unsupported platform contracts");
    assert!(unsupported.iter().any(|entry| {
        entry["kind"] == "mcp"
            && entry["platform"] == "hermes"
            && entry["scope"] == "project"
            && entry["reason_code"] == "hermes_project_mcp_unsupported"
    }));

    let mcp_case_collisions = report["entries"]
        .as_array()
        .expect("project MCP entries")
        .iter()
        .filter(|entry| {
            entry["kind"] == "mcp"
                && matches!(entry["name"].as_str(), Some("CaseServer" | "caseserver"))
        })
        .collect::<Vec<_>>();
    assert_eq!(mcp_case_collisions.len(), 2);
    assert!(mcp_case_collisions.iter().all(|entry| {
        entry["classification"] == "case_collision"
            && entry["reason_code"] == "case_only_name_collision"
            && entry["blocking"] == true
    }));
    let cross_kind_skill = entry_named(&report, "CASESERver", "/skills/");
    assert_ne!(cross_kind_skill["classification"], "case_collision");
}

#[test]
fn mcp_inventory_reports_invalid_and_unsafe_containers_without_leaking_or_aborting_other_roots() {
    let home = TempDir::new().expect("temporary HOME");
    let asset_root = home.path().join(".agents-manager");
    write_canonical_mcp(&asset_root, "catalog", "catalog", "catalog-global");
    write(
        &home.path().join(".cursor/mcp.json"),
        &format!("invalid-json {MCP_SECRET_SENTINEL}"),
    );
    write(
        &home.path().join(".codex/config.toml"),
        "[mcp_servers.catalog]\ncommand = \"catalog-global\"\n",
    );

    let outside = TempDir::new().expect("external platform container owner");
    let outside_claude = outside.path().join("claude.json");
    write(
        &outside_claude,
        &format!(r#"{{"mcpServers":{{"outside":{{"command":"{MCP_SECRET_SENTINEL}"}}}}}}"#),
    );
    fs::create_dir_all(home.path()).expect("temporary HOME exists");
    symlink(&outside_claude, home.path().join(".claude.json"))
        .expect("create unknown-root Claude container link");
    let outside_hermes = outside.path().join("hermes");
    write(
        &outside_hermes.join("config.yaml"),
        &format!("mcp_servers:\n  outside-parent:\n    command: {MCP_SECRET_SENTINEL}\n"),
    );
    symlink(&outside_hermes, home.path().join(".hermes"))
        .expect("create unknown-root Hermes parent directory link");

    let output = rules_inventory_output(home.path(), &asset_root);
    assert!(
        output.status.success(),
        "one invalid/unsafe platform container must not hide other inventory roots: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 inventory JSON");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 inventory stderr");
    assert!(
        !stdout.contains(MCP_SECRET_SENTINEL) && !stderr.contains(MCP_SECRET_SENTINEL),
        "invalid or linked container contents must never leak"
    );
    let report: Value = serde_json::from_str(&stdout).expect("inventory JSON with issues");
    let codex = entry_named(&report, "catalog", ".codex/config.toml");
    assert_eq!(codex["kind"], "mcp");

    let issues = report["issues"]
        .as_array()
        .expect("inventory must expose structured fail-closed issues");
    for (fragment, reason) in [
        (".cursor/mcp.json", "invalid_mcp_container"),
        (".claude.json", "unsafe_mcp_container_symlink"),
        (".hermes/config.yaml", "unsafe_mcp_container_parent_symlink"),
    ] {
        let issue = issues
            .iter()
            .find(|issue| {
                issue["path"]
                    .as_str()
                    .is_some_and(|path| path.contains(fragment))
            })
            .unwrap_or_else(|| panic!("missing MCP issue for {fragment}: {report:?}"));
        assert_eq!(issue["kind"], "mcp");
        assert_eq!(issue["scope"], "global");
        assert_eq!(issue["reason_code"], reason);
        assert_eq!(issue["blocking"], true);
        assert!(
            issue["message"].is_null() && issue["config"].is_null() && issue["content"].is_null(),
            "issues must be code/path only and never retain parser or config payloads: {issue:?}"
        );
    }
    assert!(
        !serde_json::to_string(&report)
            .expect("serialize fail-closed inventory")
            .contains("outside-parent"),
        "a parent directory symlink must never be followed"
    );
}

#[test]
fn canonical_mcp_parent_links_mismatches_and_legacy_errors_are_reported_without_following() {
    let home = TempDir::new().expect("temporary HOME");
    let asset_root = home.path().join(".agents-manager");
    fs::create_dir_all(&asset_root).expect("create canonical root");
    let outside = TempDir::new().expect("external canonical owner");
    write_canonical_mcp(outside.path(), "escaped", "escaped", MCP_SECRET_SENTINEL);
    symlink(outside.path().join("mcp"), asset_root.join("mcp"))
        .expect("create canonical MCP parent link");
    write(
        &asset_root.join("mcp.json"),
        &format!("invalid legacy json {MCP_SECRET_SENTINEL}"),
    );
    write(
        &home.path().join(".codex/config.toml"),
        "[mcp_servers.valid-codex]\ncommand = \"valid-codex\"\n",
    );

    let output = rules_inventory_output(home.path(), &asset_root);
    assert!(
        output.status.success(),
        "unsafe canonical paths must be reported while valid platform roots continue: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 canonical issue inventory");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 canonical issue stderr");
    assert!(!stdout.contains(MCP_SECRET_SENTINEL) && !stderr.contains(MCP_SECRET_SENTINEL));
    let report: Value = serde_json::from_str(&stdout).expect("canonical issue inventory JSON");
    assert_eq!(
        entry_named(&report, "valid-codex", ".codex/config.toml")["kind"],
        "mcp"
    );
    assert_mcp_issue(
        &report,
        "/.agents-manager/mcp/servers",
        "unsafe_canonical_mcp_parent_symlink",
    );
    assert_mcp_issue(
        &report,
        "/.agents-manager/mcp.json",
        "invalid_legacy_canonical_mcp",
    );
    assert!(
        !serde_json::to_string(&report)
            .expect("serialize canonical issues")
            .contains("escaped"),
        "canonical MCP parent symlink contents must not be inspected"
    );

    let mismatch_home = TempDir::new().expect("mismatch HOME");
    let mismatch_root = mismatch_home.path().join(".agents-manager");
    write_canonical_mcp(
        &mismatch_root,
        "filename",
        "different-name",
        "must-not-be-effective",
    );
    let outside_legacy = outside.path().join("legacy-mcp.json");
    write(
        &outside_legacy,
        &format!(r#"{{"mcpServers":{{"legacy-outside":{{"command":"{MCP_SECRET_SENTINEL}"}}}}}}"#),
    );
    symlink(&outside_legacy, mismatch_root.join("mcp.json"))
        .expect("create legacy canonical MCP link");
    write(
        &mismatch_home.path().join(".cursor/mcp.json"),
        r#"{"mcpServers":{"valid-cursor":{"command":"valid-cursor"}}}"#,
    );

    let mismatch_output = rules_inventory_output(mismatch_home.path(), &mismatch_root);
    assert!(
        mismatch_output.status.success(),
        "filename mismatch and linked legacy source must be structured issues: stdout={} stderr={}",
        String::from_utf8_lossy(&mismatch_output.stdout),
        String::from_utf8_lossy(&mismatch_output.stderr),
    );
    let mismatch_stdout =
        String::from_utf8(mismatch_output.stdout).expect("UTF-8 mismatch inventory");
    assert!(!mismatch_stdout.contains(MCP_SECRET_SENTINEL));
    let mismatch_report: Value =
        serde_json::from_str(&mismatch_stdout).expect("mismatch inventory JSON");
    assert_eq!(
        entry_named(&mismatch_report, "valid-cursor", ".cursor/mcp.json")["kind"],
        "mcp"
    );
    assert_mcp_issue(
        &mismatch_report,
        "/mcp/servers/filename.json",
        "canonical_mcp_filename_name_mismatch",
    );
    assert_mcp_issue(
        &mismatch_report,
        "/.agents-manager/mcp.json",
        "unsafe_legacy_canonical_mcp_symlink",
    );
    assert!(
        !serde_json::to_string(&mismatch_report)
            .expect("serialize mismatch issues")
            .contains("different-name"),
        "a mismatched canonical server must not become effective"
    );
}

const AGENT_BODY_SENTINEL: &str = "t010-agent-body-must-never-serialize";
const AGENT_FRONTMATTER_SENTINEL: &str = "t010-agent-frontmatter-must-never-serialize";
const AGENT_EXTERNAL_SENTINEL: &str = "t010-agent-external-link-must-never-be-read";

fn write_canonical_agent(asset_root: &Path, name: &str, layer_label: &str) -> PathBuf {
    let path = asset_root.join("agents").join(format!("{name}.md"));
    write(
        &path,
        &format!(
            r#"---
name: {name}
description: {AGENT_FRONTMATTER_SENTINEL}-{layer_label}
tools:
  - {AGENT_FRONTMATTER_SENTINEL}-tool
model: {AGENT_FRONTMATTER_SENTINEL}-model
---
{AGENT_BODY_SENTINEL}-{layer_label}
"#
        ),
    );
    path
}

fn write_cursor_or_claude_agent(path: &Path, name: &str, platform: &str) {
    write(
        path,
        &format!(
            r#"---
name: {name}
description: {AGENT_FRONTMATTER_SENTINEL}-{platform}
tools:
  - {AGENT_FRONTMATTER_SENTINEL}-{platform}-tool
model: {AGENT_FRONTMATTER_SENTINEL}-{platform}-model
---
{AGENT_BODY_SENTINEL}-{platform}
"#
        ),
    );
}

fn write_codex_agent(path: &Path, name: &str, label: &str) {
    write(
        path,
        &format!(
            r#"name = "{name}"
description = "{AGENT_FRONTMATTER_SENTINEL}-{label}"
developer_instructions = "{AGENT_BODY_SENTINEL}-{label}"
model = "{AGENT_FRONTMATTER_SENTINEL}-{label}-model"
tools = ["{AGENT_FRONTMATTER_SENTINEL}-{label}-tool"]
"#
        ),
    );
}

fn assert_agent_source(entry: &Value, source_layer: &str, canonical_path: &Path, scope: &str) {
    assert_eq!(entry["kind"], "agent", "entry={entry:?}");
    assert_eq!(entry["source_layer"], source_layer, "entry={entry:?}");
    assert_eq!(
        entry["canonical_path"],
        canonical_path.to_string_lossy().as_ref(),
        "entry={entry:?}"
    );
    assert_eq!(entry["scope"], scope, "entry={entry:?}");
}

fn assert_generated_agent_entry(
    report: &Value,
    name: &str,
    path_fragment: &str,
    format: &str,
    consumer: &str,
) -> Value {
    let entry = entry_named(report, name, path_fragment).clone();
    assert_eq!(entry["kind"], "agent", "entry={entry:?}");
    assert_eq!(entry["format"], format, "entry={entry:?}");
    assert_eq!(entry["entry_key"], Value::Null, "entry={entry:?}");
    assert_eq!(entry["consumers"], serde_json::json!([consumer]));
    assert_eq!(entry["provenance"], "platform_current", "entry={entry:?}");
    assert_eq!(entry["reason_code"], "platform_agent_file_unowned");
    assert_eq!(entry["currently_consumed"], true, "entry={entry:?}");
    assert_eq!(entry["classification"], "foreign", "entry={entry:?}");
    assert_eq!(entry["ownership_state"], "foreign", "entry={entry:?}");
    assert_eq!(entry["owned"], false, "entry={entry:?}");
    assert_eq!(entry["selectable"], false, "entry={entry:?}");
    assert_eq!(
        entry["blocking"], true,
        "same-name generated file without ledger ownership must block takeover: {entry:?}"
    );
    assert!(
        entry["content_digest"]
            .as_str()
            .is_some_and(|digest| !digest.is_empty()),
        "generated agent file must have an opaque content fingerprint: {entry:?}"
    );
    for raw_field in [
        "body",
        "content",
        "frontmatter",
        "tools",
        "model",
        "developer_instructions",
        "rendered_config",
    ] {
        assert!(
            entry[raw_field].is_null(),
            "agent inventory must never serialize `{raw_field}` source/platform content: {entry:?}"
        );
    }
    entry
}

fn assert_hermes_agent_unsupported(
    report: &Value,
    name: &str,
    source_layer: &str,
    canonical_path: &Path,
    scope: &str,
) {
    let unsupported = report["unsupported"]
        .as_array()
        .expect("inventory unsupported contracts");
    let entry = unsupported
        .iter()
        .find(|entry| {
            entry["kind"] == "agent"
                && entry["platform"] == "hermes"
                && entry["name"] == name
                && entry["scope"] == scope
        })
        .unwrap_or_else(|| {
            panic!("missing Hermes Agent unsupported row name={name:?}: {report:?}")
        });
    assert_eq!(entry["reason_code"], "hermes_static_agent_unsupported");
    assert_eq!(entry["source_layer"], source_layer);
    assert_eq!(
        entry["canonical_path"],
        canonical_path.to_string_lossy().as_ref()
    );
}

fn assert_agent_report_redacted(stdout: &str, stderr: &str) {
    for sentinel in [
        AGENT_BODY_SENTINEL,
        AGENT_FRONTMATTER_SENTINEL,
        AGENT_EXTERNAL_SENTINEL,
    ] {
        assert!(
            !stdout.contains(sentinel) && !stderr.contains(sentinel),
            "Agent inventory must not emit source, generated, or linked body content: {sentinel}"
        );
    }
}

#[test]
fn global_agent_inventory_reports_native_generated_files_legacy_and_hermes_without_bodies() {
    let home = TempDir::new().expect("temporary HOME");
    let asset_root = home.path().join(".agents-manager");
    let reviewer_source = write_canonical_agent(&asset_root, "reviewer", "global");
    let case_source = write_canonical_agent(&asset_root, "CaseAgent", "global-case");
    write(
        &asset_root.join("skills/caseagent/SKILL.md"),
        "same spelling in another kind must not affect Agent collision\n",
    );

    write_cursor_or_claude_agent(
        &home.path().join(".cursor/agents/reviewer.md"),
        "reviewer",
        "cursor-global",
    );
    write_codex_agent(
        &home.path().join(".codex/agents/reviewer.toml"),
        "reviewer",
        "codex-global",
    );
    write_cursor_or_claude_agent(
        &home.path().join(".claude/agents/reviewer.md"),
        "reviewer",
        "claude-global",
    );
    write_cursor_or_claude_agent(
        &home.path().join(".cursor/agents/caseagent.md"),
        "caseagent",
        "cursor-case",
    );
    write(
        &home.path().join(".codex/subagents/reviewer.md"),
        &format!("{AGENT_BODY_SENTINEL}-legacy-codex-subagent\n"),
    );
    write(
        &home.path().join(".hermes/agents/must-not-scan.md"),
        &format!("{AGENT_BODY_SENTINEL}-hermes-static-directory-is-unsupported\n"),
    );

    let before = tree_snapshot(home.path());
    let output = rules_inventory_output(home.path(), &asset_root);
    assert!(
        output.status.success(),
        "global Agent inventory must succeed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 global Agent inventory");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 global Agent stderr");
    assert_agent_report_redacted(&stdout, &stderr);
    assert_eq!(
        tree_snapshot(home.path()),
        before,
        "Agent inventory must be strictly read-only"
    );
    let report: Value = serde_json::from_str(&stdout).expect("global Agent inventory JSON");

    let cursor = assert_generated_agent_entry(
        &report,
        "reviewer",
        ".cursor/agents/reviewer.md",
        "markdown",
        "cursor",
    );
    assert_agent_source(&cursor, "global", &reviewer_source, "global");
    let codex = assert_generated_agent_entry(
        &report,
        "reviewer",
        ".codex/agents/reviewer.toml",
        "toml",
        "codex",
    );
    assert_agent_source(&codex, "global", &reviewer_source, "global");
    let claude = assert_generated_agent_entry(
        &report,
        "reviewer",
        ".claude/agents/reviewer.md",
        "markdown",
        "claude",
    );
    assert_agent_source(&claude, "global", &reviewer_source, "global");

    for path_fragment in [".cursor/agents/reviewer.md", ".claude/agents/reviewer.md"] {
        let matching = report["entries"]
            .as_array()
            .expect("Agent entries")
            .iter()
            .filter(|entry| {
                entry["kind"] == "agent"
                    && entry["path"]
                        .as_str()
                        .is_some_and(|path| path.contains(path_fragment))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            matching.len(),
            1,
            "native Cursor/Claude agent path is current, not a duplicated legacy row: {matching:?}"
        );
        assert_eq!(matching[0]["provenance"], "platform_current");
    }

    let legacy = entry_named(&report, "reviewer", ".codex/subagents/reviewer.md");
    assert_eq!(legacy["kind"], "agent");
    assert_eq!(legacy["provenance"], "platform_legacy");
    assert_eq!(legacy["reason_code"], "legacy_codex_subagent");
    assert_eq!(legacy["currently_consumed"], false);
    assert_eq!(legacy["classification"], "foreign");
    assert_eq!(legacy["ownership_state"], "foreign");
    assert_eq!(legacy["blocking"], false);
    assert_eq!(legacy["owned"], false);
    assert_eq!(legacy["selectable"], false);

    assert_hermes_agent_unsupported(&report, "reviewer", "global", &reviewer_source, "global");
    let serialized = serde_json::to_string(&report).expect("serialize Agent inventory");
    assert!(
        !serialized.contains(".hermes/agents") && !serialized.contains("must-not-scan"),
        "Hermes has no static global Agent target and its guessed directory must not be scanned"
    );

    let collisions = report["entries"]
        .as_array()
        .expect("Agent entries")
        .iter()
        .filter(|entry| {
            entry["kind"] == "agent"
                && matches!(entry["name"].as_str(), Some("CaseAgent" | "caseagent"))
        })
        .collect::<Vec<_>>();
    assert_eq!(collisions.len(), 2, "Agent collision rows: {collisions:?}");
    assert!(collisions.iter().all(|entry| {
        entry["classification"] == "case_collision"
            && entry["reason_code"] == "case_only_name_collision"
            && entry["blocking"] == true
    }));
    let cross_kind_skill = entry_named(&report, "caseagent", "/skills/");
    assert_ne!(cross_kind_skill["classification"], "case_collision");
    assert_eq!(
        entry_named(&report, "CaseAgent", "/agents/")["canonical_path"],
        case_source.to_string_lossy().as_ref()
    );
}

#[test]
fn project_agent_inventory_uses_overlay_and_never_reads_home_parent_links_or_hermes() {
    let home = TempDir::new().expect("temporary HOME");
    let repo = TempDir::new().expect("temporary project");
    let global_root = home.path().join(".agents-manager");
    let project_root = repo.path().join(".agents-manager");
    let global_shared = write_canonical_agent(&global_root, "shared", "global-shared");
    let global_only = write_canonical_agent(&global_root, "global-only", "global-only");
    let project_shared = write_canonical_agent(&project_root, "shared", "project-shared");

    write_cursor_or_claude_agent(
        &repo.path().join(".cursor/agents/shared.md"),
        "shared",
        "cursor-project",
    );
    write_cursor_or_claude_agent(
        &repo.path().join(".cursor/agents/global-only.md"),
        "global-only",
        "cursor-project-inherited",
    );
    write_codex_agent(
        &repo.path().join(".codex/agents/shared.toml"),
        "shared",
        "codex-project",
    );
    write_cursor_or_claude_agent(
        &repo.path().join(".claude/agents/shared.md"),
        "shared",
        "claude-project",
    );
    write(
        &repo.path().join(".codex/subagents/project-legacy.md"),
        &format!("{AGENT_BODY_SENTINEL}-project-legacy\n"),
    );
    write(
        &repo.path().join(".hermes/agents/project-ghost.md"),
        &format!("{AGENT_BODY_SENTINEL}-project-hermes-must-not-scan\n"),
    );

    let outside = TempDir::new().expect("outside HOME Agent owner");
    write(
        &outside.path().join("home-linked.md"),
        AGENT_EXTERNAL_SENTINEL,
    );
    fs::create_dir_all(home.path().join(".cursor")).expect("create HOME Cursor parent");
    symlink(outside.path(), home.path().join(".cursor/agents"))
        .expect("create escaped HOME Cursor Agent root");
    write_cursor_or_claude_agent(
        &home.path().join(".claude/agents/home-only.md"),
        "home-only",
        "home-must-not-scan",
    );
    write_codex_agent(
        &home.path().join(".codex/agents/home-only.toml"),
        "home-only",
        "home-must-not-scan",
    );
    write(
        &home.path().join(".codex/subagents/home-only.md"),
        AGENT_EXTERNAL_SENTINEL,
    );
    write(
        &home.path().join(".hermes/agents/home-only.md"),
        AGENT_EXTERNAL_SENTINEL,
    );

    let home_before = tree_snapshot(home.path());
    let repo_before = tree_snapshot(repo.path());
    let outside_before = tree_snapshot(outside.path());
    let output = rules_inventory_output(home.path(), repo.path());
    assert!(
        output.status.success(),
        "project Agent inventory must succeed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 project Agent inventory");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 project Agent stderr");
    assert_agent_report_redacted(&stdout, &stderr);
    assert_eq!(tree_snapshot(home.path()), home_before, "HOME changed");
    assert_eq!(tree_snapshot(repo.path()), repo_before, "project changed");
    assert_eq!(
        tree_snapshot(outside.path()),
        outside_before,
        "outside HOME Agent owner changed"
    );
    let report: Value = serde_json::from_str(&stdout).expect("project Agent inventory JSON");
    assert_eq!(report["scope"], "project");

    let cursor = assert_generated_agent_entry(
        &report,
        "shared",
        ".cursor/agents/shared.md",
        "markdown",
        "cursor",
    );
    assert_agent_source(&cursor, "project", &project_shared, "project");
    assert_ne!(
        cursor["canonical_path"],
        global_shared.to_string_lossy().as_ref(),
        "project whole-agent definition must override global"
    );
    let inherited = assert_generated_agent_entry(
        &report,
        "global-only",
        ".cursor/agents/global-only.md",
        "markdown",
        "cursor",
    );
    assert_agent_source(&inherited, "global", &global_only, "project");
    let codex = assert_generated_agent_entry(
        &report,
        "shared",
        ".codex/agents/shared.toml",
        "toml",
        "codex",
    );
    assert_agent_source(&codex, "project", &project_shared, "project");
    let claude = assert_generated_agent_entry(
        &report,
        "shared",
        ".claude/agents/shared.md",
        "markdown",
        "claude",
    );
    assert_agent_source(&claude, "project", &project_shared, "project");

    let legacy = entry_named(
        &report,
        "project-legacy",
        ".codex/subagents/project-legacy.md",
    );
    assert_eq!(legacy["kind"], "agent");
    assert_eq!(legacy["provenance"], "platform_legacy");
    assert_eq!(legacy["reason_code"], "legacy_codex_subagent");
    assert_eq!(legacy["currently_consumed"], false);
    assert_eq!(legacy["classification"], "foreign");
    assert_eq!(legacy["ownership_state"], "foreign");
    assert_eq!(legacy["blocking"], false);
    assert_eq!(legacy["owned"], false);
    assert_eq!(legacy["selectable"], false);

    let raw_shared = report["entries"]
        .as_array()
        .expect("Agent entries")
        .iter()
        .filter(|entry| {
            entry["kind"] == "agent"
                && entry["name"] == "shared"
                && entry["provenance"] == "canonical"
        })
        .collect::<Vec<_>>();
    assert_eq!(
        raw_shared.len(),
        2,
        "raw inventory must retain global and project Agent provenance: {report:?}"
    );
    let layers = raw_shared
        .iter()
        .map(|entry| entry["source_layer"].as_str().unwrap_or_default())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        layers,
        std::collections::BTreeSet::from(["global", "project"])
    );

    assert_hermes_agent_unsupported(&report, "shared", "project", &project_shared, "project");
    assert_hermes_agent_unsupported(&report, "global-only", "global", &global_only, "project");
    let serialized = serde_json::to_string(&report).expect("serialize project Agent inventory");
    assert!(
        !serialized.contains("home-only")
            && !serialized.contains("home-linked")
            && !serialized.contains(outside.path().to_string_lossy().as_ref()),
        "project Agent inventory must not read HOME current/legacy paths or follow HOME parent links"
    );
    assert!(
        !serialized.contains(".hermes/agents") && !serialized.contains("project-ghost"),
        "Hermes global/project static Agent directories are unsupported and must not be scanned"
    );
    for issue in report["issues"].as_array().expect("inventory issues") {
        let Some(path) = issue["path"].as_str() else {
            continue;
        };
        if issue["kind"] == "agent" {
            assert!(
                Path::new(path).starts_with(repo.path()),
                "project Agent issue must never originate from HOME or outside: {issue:?}"
            );
        }
    }
}

#[test]
fn agent_inventory_reports_symlink_boundaries_and_invalid_canonical_schema_without_leaking() {
    let assert_issue = |report: &Value, path_fragment: &str, reason: &str| {
        let issue = report["issues"]
            .as_array()
            .expect("Agent issues")
            .iter()
            .find(|issue| {
                issue["kind"] == "agent"
                    && issue["path"]
                        .as_str()
                        .is_some_and(|path| path.contains(path_fragment))
            })
            .unwrap_or_else(|| panic!("missing Agent issue {reason}: {report:?}"));
        assert_eq!(issue["reason_code"], reason);
        assert_eq!(issue["blocking"], true);
    };

    let home = TempDir::new().expect("temporary HOME");
    let repo = TempDir::new().expect("temporary project");
    let global_root = home.path().join(".agents-manager");
    let project_root = repo.path().join(".agents-manager");
    let safe_source = write_canonical_agent(&global_root, "safe", "global-safe");
    fs::create_dir_all(&project_root).expect("create project canonical root");
    let outside_canonical = TempDir::new().expect("outside canonical Agent root");
    write(
        &outside_canonical.path().join("escaped.md"),
        AGENT_EXTERNAL_SENTINEL,
    );
    symlink(outside_canonical.path(), project_root.join("agents"))
        .expect("escape project canonical agents");

    let outside_cursor = TempDir::new().expect("outside Cursor parent");
    write(
        &outside_cursor.path().join("agents/escaped.md"),
        AGENT_EXTERNAL_SENTINEL,
    );
    symlink(outside_cursor.path(), repo.path().join(".cursor")).expect("escape Cursor parent");
    write_codex_agent(
        &repo.path().join(".codex/agents/safe.toml"),
        "safe",
        "valid-codex",
    );
    write(
        &repo
            .path()
            .join(".codex/agents/directory.toml/must-not-recurse.toml"),
        AGENT_EXTERNAL_SENTINEL,
    );
    let outside_file = outside_cursor.path().join("linked.md");
    write(&outside_file, AGENT_EXTERNAL_SENTINEL);
    fs::create_dir_all(repo.path().join(".codex/subagents"))
        .expect("create legacy Codex Agent root");
    symlink(
        &outside_file,
        repo.path().join(".codex/subagents/legacy-linked.md"),
    )
    .expect("link legacy Codex Agent child");
    fs::create_dir_all(repo.path().join(".claude/agents")).expect("create Claude agents");
    symlink(&outside_file, repo.path().join(".claude/agents/linked.md"))
        .expect("link final Claude Agent file");
    write(
        &repo.path().join(".claude/subagents/legacy.md"),
        AGENT_BODY_SENTINEL,
    );

    let output = rules_inventory_output(home.path(), repo.path());
    assert!(output.status.success(), "stdout={:?}", output.stdout);
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 Agent security inventory");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 Agent security stderr");
    assert_agent_report_redacted(&stdout, &stderr);
    let report: Value = serde_json::from_str(&stdout).expect("Agent security inventory JSON");
    assert_issue(
        &report,
        "/.codex/agents/directory.toml",
        "unsafe_agent_file_non_regular",
    );
    assert_issue(
        &report,
        "/.codex/subagents/legacy-linked.md",
        "unsafe_agent_file_symlink",
    );
    assert_issue(
        &report,
        "/.agents-manager/agents",
        "unsafe_canonical_agent_directory",
    );
    assert_issue(&report, "/.cursor/agents", "unsafe_agent_directory_parent");
    assert_issue(
        &report,
        "/.claude/agents/linked.md",
        "unsafe_agent_file_symlink",
    );
    let codex =
        assert_generated_agent_entry(&report, "safe", ".codex/agents/safe.toml", "toml", "codex");
    assert_agent_source(&codex, "global", &safe_source, "project");
    let legacy = entry_named(&report, "legacy", ".claude/subagents/legacy.md");
    assert_eq!(legacy["provenance"], "platform_legacy");
    assert_eq!(legacy["reason_code"], "legacy_claude_subagent");
    assert_eq!(legacy["currently_consumed"], false);
    assert_eq!(legacy["blocking"], false);
    let serialized = serde_json::to_string(&report).expect("serialize Agent security report");
    assert!(!report["entries"]
        .as_array()
        .expect("Agent security entries")
        .iter()
        .any(|entry| {
            matches!(
                entry["name"].as_str(),
                Some("escaped" | "linked" | "directory" | "legacy-linked")
            )
        }));
    assert!(!serialized.contains("must-not-recurse"));
    assert!(
        !serialized.contains(outside_canonical.path().to_string_lossy().as_ref())
            && !serialized.contains(outside_cursor.path().to_string_lossy().as_ref())
    );

    let invalid_home = TempDir::new().expect("invalid Agent HOME");
    let invalid_root = invalid_home.path().join(".agents-manager");
    write(
        &invalid_root.join("agents/wrong.md"),
        &format!("---\nname: different\ndescription: valid\n---\n{AGENT_BODY_SENTINEL}-mismatch\n"),
    );
    write(
        &invalid_root.join("agents/invalid.md"),
        &format!("---\nname: invalid\n---\n{AGENT_BODY_SENTINEL}-missing-description"),
    );
    write_codex_agent(
        &invalid_home.path().join(".codex/agents/wrong.toml"),
        "wrong",
        "valid-foreign",
    );
    let reviewer_source = write_canonical_agent(&invalid_root, "Reviewer", "valid-reviewer");
    write_codex_agent(
        &invalid_home.path().join(".codex/agents/Reviewer.toml"),
        "Reviewer",
        "current-reviewer",
    );
    write(
        &invalid_home.path().join(".codex/subagents/Reviewer.yaml"),
        &format!("name: Reviewer\ninstructions: {AGENT_BODY_SENTINEL}-legacy-yaml\n"),
    );
    write(
        &invalid_home.path().join(".codex/subagents/reviewer.json"),
        &format!(r#"{{"name":"reviewer","instructions":"{AGENT_BODY_SENTINEL}-legacy-json"}}"#),
    );
    write(
        &invalid_home.path().join(".codex/subagents/extensionless"),
        AGENT_BODY_SENTINEL,
    );
    write(
        &invalid_home
            .path()
            .join(".claude/subagents/bundle/AGENT.md"),
        AGENT_BODY_SENTINEL,
    );
    write(
        &invalid_root.join("agents/legacy.yaml"),
        &format!("name: legacy\ndescription: old\ninstructions: {AGENT_BODY_SENTINEL}\n"),
    );
    write(
        &invalid_root.join("agents/legacy-dir/AGENT.md"),
        AGENT_BODY_SENTINEL,
    );
    write_cursor_or_claude_agent(
        &invalid_home.path().join(".cursor/agents/legacy.md"),
        "legacy",
        "current-must-not-use-canonical-legacy",
    );
    for path in [
        invalid_root.join("agents/.hidden.md"),
        invalid_root.join("agents/README.md"),
    ] {
        write(
            &path,
            &format!(
                "---\nname: ignored\ndescription: ignored\n---\n{AGENT_BODY_SENTINEL}-ignored"
            ),
        );
    }
    let invalid_output = rules_inventory_output(invalid_home.path(), &invalid_root);
    assert!(invalid_output.status.success());
    let invalid_stdout =
        String::from_utf8(invalid_output.stdout).expect("UTF-8 invalid Agent inventory");
    let invalid_stderr =
        String::from_utf8(invalid_output.stderr).expect("UTF-8 invalid Agent stderr");
    assert_agent_report_redacted(&invalid_stdout, &invalid_stderr);
    let invalid_report: Value =
        serde_json::from_str(&invalid_stdout).expect("invalid Agent inventory JSON");
    assert_issue(
        &invalid_report,
        "/agents/wrong.md",
        "canonical_agent_name_mismatch",
    );
    assert_issue(
        &invalid_report,
        "/agents/invalid.md",
        "invalid_canonical_agent_schema",
    );
    let foreign = entry_named(&invalid_report, "wrong", ".codex/agents/wrong.toml");
    assert_eq!(foreign["classification"], "foreign");
    assert_eq!(foreign["source_layer"], Value::Null);
    assert_eq!(foreign["canonical_path"], Value::Null);
    assert_eq!(foreign["blocking"], false);
    assert!(!invalid_report["entries"]
        .as_array()
        .expect("invalid Agent entries")
        .iter()
        .any(|entry| {
            entry["kind"] == "agent"
                && entry["provenance"] == "canonical"
                && matches!(entry["name"].as_str(), Some("wrong" | "invalid"))
        }));

    let current_reviewer = entry_named(&invalid_report, "Reviewer", ".codex/agents/Reviewer.toml");
    assert_agent_source(current_reviewer, "global", &reviewer_source, "global");
    assert_ne!(current_reviewer["classification"], "case_collision");
    assert_ne!(
        entry_named(&invalid_report, "Reviewer", "/agents/Reviewer.md")["classification"],
        "case_collision"
    );
    let assert_legacy = |name: &str, fragment: &str, format: &str| {
        let entry = entry_named(&invalid_report, name, fragment);
        assert_eq!(entry["kind"], "agent");
        assert_eq!(entry["provenance"], "platform_legacy");
        assert_eq!(entry["classification"], "foreign");
        assert_eq!(entry["ownership_state"], "foreign");
        assert_eq!(entry["currently_consumed"], false);
        assert_eq!(entry["blocking"], false);
        assert_eq!(entry["owned"], false);
        assert_eq!(entry["selectable"], false);
        assert_eq!(entry["format"], format);
        entry
    };
    let legacy_reviewer = assert_legacy("Reviewer", ".codex/subagents/Reviewer.yaml", "yaml");
    assert_agent_source(legacy_reviewer, "global", &reviewer_source, "global");
    let lowercase = assert_legacy("reviewer", ".codex/subagents/reviewer.json", "json");
    assert_eq!(lowercase["source_layer"], Value::Null);
    assert_eq!(lowercase["canonical_path"], Value::Null);
    assert_legacy("extensionless", ".codex/subagents/extensionless", "file");
    assert_legacy("bundle", ".claude/subagents/bundle", "directory");

    for (name, fragment, format) in [
        ("legacy", "/agents/legacy.yaml", "yaml"),
        ("legacy-dir", "/agents/legacy-dir", "directory"),
    ] {
        let entry = entry_named(&invalid_report, name, fragment);
        assert_eq!(entry["provenance"], "canonical_legacy");
        assert_eq!(entry["classification"], "legacy_agent_candidate");
        assert_eq!(entry["source_layer"], "global");
        assert_eq!(entry["canonical_path"], entry["path"]);
        assert_eq!(entry["format"], format);
        assert_eq!(entry["currently_consumed"], false);
        assert_eq!(entry["owned"], false);
        assert_eq!(entry["selectable"], false);
    }
    let current_legacy = entry_named(&invalid_report, "legacy", ".cursor/agents/legacy.md");
    assert_eq!(current_legacy["source_layer"], Value::Null);
    assert_eq!(current_legacy["canonical_path"], Value::Null);
    assert_eq!(current_legacy["blocking"], false);
    assert!(!invalid_report["unsupported"]
        .as_array()
        .expect("unsupported Agents")
        .iter()
        .any(|entry| matches!(entry["name"].as_str(), Some("legacy" | "legacy-dir"))));
    let serialized = serde_json::to_string(&invalid_report).expect("serialize legacy Agents");
    assert!(!serialized.contains("/agents/.hidden.md"));
    assert!(!serialized.contains("/agents/README.md"));
}

const COMMAND_BODY_SENTINEL: &str = "t010-command-body-must-never-serialize";
const COMMAND_EXTERNAL_SENTINEL: &str = "t010-command-external-must-never-be-read";

fn write_canonical_command(asset_root: &Path, name: &str, label: &str) -> PathBuf {
    let path = asset_root.join("commands").join(format!("{name}.md"));
    write(
        &path,
        &format!(
            "---\ndescription: explicit {label}\nargument-hint: {COMMAND_BODY_SENTINEL}-args\n---\n\
             # {name}\n\n{COMMAND_BODY_SENTINEL}-{label}\n"
        ),
    );
    path
}

fn assert_command_source(entry: &Value, layer: &str, canonical: &Path, scope: &str) {
    assert_eq!(entry["kind"], "command", "entry={entry:?}");
    assert_eq!(entry["source_layer"], layer, "entry={entry:?}");
    assert_eq!(
        entry["canonical_path"],
        canonical.to_string_lossy().as_ref(),
        "entry={entry:?}"
    );
    assert_eq!(entry["scope"], scope, "entry={entry:?}");
}

fn assert_command_unsupported(
    report: &Value,
    name: &str,
    platform: &str,
    layer: &str,
    canonical: &Path,
    scope: &str,
) {
    let entry = report["unsupported"]
        .as_array()
        .expect("Command unsupported rows")
        .iter()
        .find(|entry| {
            entry["kind"] == "command"
                && entry["name"] == name
                && entry["platform"] == platform
                && entry["scope"] == scope
        })
        .unwrap_or_else(|| panic!("missing {platform} Command unsupported row: {report:?}"));
    assert_eq!(
        entry["reason_code"],
        format!("{platform}_command_unsupported")
    );
    assert_eq!(entry["source_layer"], layer);
    assert_eq!(
        entry["canonical_path"],
        canonical.to_string_lossy().as_ref()
    );
}

fn assert_command_redacted(stdout: &str, stderr: &str) {
    for sentinel in [COMMAND_BODY_SENTINEL, COMMAND_EXTERNAL_SENTINEL] {
        assert!(
            !stdout.contains(sentinel) && !stderr.contains(sentinel),
            "Command inventory leaked {sentinel}"
        );
    }
}

#[test]
fn global_command_inventory_reports_current_legacy_and_unsupported_without_bodies() {
    let home = TempDir::new().expect("temporary HOME");
    let asset_root = home.path().join(".agents-manager");
    let source = write_canonical_command(&asset_root, "review", "global");
    let case_source = write_canonical_command(&asset_root, "Reviewer", "global-case");
    fs::create_dir_all(home.path().join(".cursor/commands")).expect("create Cursor commands");
    symlink(&source, home.path().join(".cursor/commands/review.md"))
        .expect("link canonical Cursor command");
    symlink(
        &case_source,
        home.path().join(".cursor/commands/Reviewer.md"),
    )
    .expect("link case-sensitive canonical Cursor command");
    write(
        &home.path().join(".claude/commands/review.md"),
        &fs::read_to_string(&source).expect("read canonical command fixture"),
    );
    write(
        &home.path().join(".codex/prompts/review.md"),
        &format!("{COMMAND_BODY_SENTINEL}-deprecated-codex-prompt\n"),
    );
    write(
        &home.path().join(".codex/prompts/reviewer.md"),
        &format!("{COMMAND_BODY_SENTINEL}-deprecated-case-prompt\n"),
    );
    let legacy_outside = home.path().join("outside-legacy-command.md");
    write(&legacy_outside, COMMAND_EXTERNAL_SENTINEL);
    symlink(
        &legacy_outside,
        home.path().join(".codex/prompts/linked.md"),
    )
    .expect("link deprecated Codex prompt outside allowlist");
    write(
        &home.path().join(".codex/commands/must-not-scan.md"),
        COMMAND_BODY_SENTINEL,
    );
    write(
        &home.path().join(".hermes/commands/must-not-scan.md"),
        COMMAND_BODY_SENTINEL,
    );

    let before = tree_snapshot(home.path());
    let output = rules_inventory_output(home.path(), &asset_root);
    assert!(
        output.status.success(),
        "global Command inventory failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 Command inventory");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 Command stderr");
    assert_command_redacted(&stdout, &stderr);
    assert_eq!(
        tree_snapshot(home.path()),
        before,
        "Command inventory wrote"
    );
    let report: Value = serde_json::from_str(&stdout).expect("global Command inventory JSON");

    let cursor = entry_named(&report, "review", ".cursor/commands/review.md");
    assert_eq!(cursor["classification"], "managed_link");
    assert_eq!(cursor["ownership_state"], "managed_link");
    assert_eq!(cursor["provenance"], "platform_current");
    assert_eq!(cursor["reason_code"], "canonical_symlink");
    assert_eq!(cursor["currently_consumed"], true);
    assert_eq!(cursor["blocking"], false);
    assert_eq!(cursor["owned"], true);
    assert_eq!(cursor["selectable"], false);
    assert_eq!(cursor["followed"], false);
    assert_eq!(cursor["format"], "markdown");
    assert_eq!(cursor["consumers"], serde_json::json!(["cursor"]));
    assert_command_source(cursor, "global", &source, "global");

    let claude = entry_named(&report, "review", ".claude/commands/review.md");
    assert_eq!(claude["classification"], "equivalent");
    assert_eq!(claude["ownership_state"], "equivalent");
    assert_eq!(claude["provenance"], "platform_current");
    assert_eq!(claude["reason_code"], "unmarked_equal_copy");
    assert_eq!(claude["currently_consumed"], true);
    assert_eq!(claude["blocking"], false);
    assert_eq!(claude["owned"], false);
    assert_eq!(claude["selectable"], true);
    assert_eq!(claude["format"], "markdown");
    assert_eq!(claude["consumers"], serde_json::json!(["claude"]));
    assert_command_source(claude, "global", &source, "global");

    let legacy = entry_named(&report, "review", ".codex/prompts/review.md");
    assert_eq!(legacy["kind"], "command");
    assert_eq!(legacy["classification"], "foreign");
    assert_eq!(legacy["ownership_state"], "foreign");
    assert_eq!(legacy["provenance"], "platform_legacy");
    assert_eq!(legacy["reason_code"], "legacy_codex_prompt");
    assert_eq!(legacy["currently_consumed"], true);
    assert_eq!(legacy["blocking"], false);
    assert_eq!(legacy["owned"], false);
    assert_eq!(legacy["selectable"], false);
    assert_eq!(legacy["format"], "markdown");
    assert_eq!(legacy["consumers"], serde_json::json!(["codex"]));
    assert_command_source(legacy, "global", &source, "global");

    let current_case = entry_named(&report, "Reviewer", ".cursor/commands/Reviewer.md");
    assert_ne!(current_case["classification"], "case_collision");
    assert_ne!(
        entry_named(&report, "Reviewer", "/commands/Reviewer.md")["classification"],
        "case_collision"
    );
    let legacy_case = entry_named(&report, "reviewer", ".codex/prompts/reviewer.md");
    assert_eq!(legacy_case["classification"], "foreign");
    assert_eq!(legacy_case["blocking"], false);
    let legacy_link_issue = report["issues"]
        .as_array()
        .expect("Command issues")
        .iter()
        .find(|issue| {
            issue["kind"] == "command"
                && issue["path"]
                    .as_str()
                    .is_some_and(|path| path.contains(".codex/prompts/linked.md"))
        })
        .expect("legacy Codex prompt symlink issue");
    assert_eq!(
        legacy_link_issue["reason_code"],
        "unsafe_legacy_command_file_symlink"
    );
    assert_eq!(legacy_link_issue["blocking"], true);

    for platform in ["codex", "hermes"] {
        assert_command_unsupported(&report, "review", platform, "global", &source, "global");
    }
    let serialized = serde_json::to_string(&report).expect("serialize global Commands");
    assert!(!serialized.contains(".codex/commands"));
    assert!(!serialized.contains(".hermes/commands"));
    for raw in ["body", "content", "frontmatter", "argument_hint"] {
        assert!(cursor[raw].is_null() && claude[raw].is_null() && legacy[raw].is_null());
    }
}

#[test]
fn project_command_inventory_uses_overlay_and_never_scans_home_or_unsupported_paths() {
    let home = TempDir::new().expect("temporary HOME");
    let repo = TempDir::new().expect("temporary project");
    let global_root = home.path().join(".agents-manager");
    let project_root = repo.path().join(".agents-manager");
    let global_shared = write_canonical_command(&global_root, "shared", "global-shared");
    let global_only = write_canonical_command(&global_root, "global-only", "global-only");
    let project_shared = write_canonical_command(&project_root, "shared", "project-shared");
    for (path, source) in [
        (
            repo.path().join(".cursor/commands/shared.md"),
            &project_shared,
        ),
        (
            repo.path().join(".cursor/commands/global-only.md"),
            &global_only,
        ),
        (
            repo.path().join(".claude/commands/shared.md"),
            &project_shared,
        ),
    ] {
        write(
            &path,
            &fs::read_to_string(source).expect("read canonical Command"),
        );
    }
    for path in [
        home.path().join(".cursor/commands/home-only.md"),
        home.path().join(".claude/commands/home-only.md"),
        home.path().join(".codex/prompts/home-legacy.md"),
        repo.path().join(".codex/prompts/project-legacy.md"),
        repo.path().join(".codex/commands/project-forbidden.md"),
        repo.path().join(".hermes/commands/project-forbidden.md"),
    ] {
        write(&path, COMMAND_EXTERNAL_SENTINEL);
    }

    let home_before = tree_snapshot(home.path());
    let repo_before = tree_snapshot(repo.path());
    let output = rules_inventory_output(home.path(), repo.path());
    assert!(
        output.status.success(),
        "project Command inventory failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 project Command inventory");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 project Command stderr");
    assert_command_redacted(&stdout, &stderr);
    assert_eq!(tree_snapshot(home.path()), home_before, "HOME changed");
    assert_eq!(tree_snapshot(repo.path()), repo_before, "project changed");
    let report: Value = serde_json::from_str(&stdout).expect("project Command inventory JSON");
    assert_eq!(report["scope"], "project");

    let cursor = entry_named(&report, "shared", ".cursor/commands/shared.md");
    assert_eq!(cursor["classification"], "equivalent");
    assert_eq!(cursor["ownership_state"], "equivalent");
    assert_eq!(cursor["provenance"], "platform_current");
    assert_eq!(cursor["currently_consumed"], true);
    assert_eq!(cursor["format"], "markdown");
    assert_eq!(cursor["consumers"], serde_json::json!(["cursor"]));
    assert_command_source(cursor, "project", &project_shared, "project");
    assert_ne!(
        cursor["canonical_path"],
        global_shared.to_string_lossy().as_ref()
    );

    let inherited = entry_named(&report, "global-only", ".cursor/commands/global-only.md");
    assert_eq!(inherited["classification"], "equivalent");
    assert_command_source(inherited, "global", &global_only, "project");
    let claude = entry_named(&report, "shared", ".claude/commands/shared.md");
    assert_eq!(claude["classification"], "equivalent");
    assert_eq!(claude["consumers"], serde_json::json!(["claude"]));
    assert_command_source(claude, "project", &project_shared, "project");

    let raw_shared = report["entries"]
        .as_array()
        .expect("Command entries")
        .iter()
        .filter(|entry| {
            entry["kind"] == "command"
                && entry["name"] == "shared"
                && entry["provenance"] == "canonical"
        })
        .collect::<Vec<_>>();
    assert_eq!(
        raw_shared.len(),
        2,
        "raw global/project Command provenance lost"
    );
    let layers = raw_shared
        .iter()
        .map(|entry| entry["source_layer"].as_str().unwrap_or_default())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        layers,
        std::collections::BTreeSet::from(["global", "project"])
    );
    for (name, layer, source) in [
        ("shared", "project", project_shared.as_path()),
        ("global-only", "global", global_only.as_path()),
    ] {
        for platform in ["codex", "hermes"] {
            assert_command_unsupported(&report, name, platform, layer, source, "project");
        }
    }
    let serialized = serde_json::to_string(&report).expect("serialize project Commands");
    for forbidden in [
        "home-only",
        "home-legacy",
        "project-legacy",
        ".codex/commands",
        ".hermes/commands",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "project inventory scanned forbidden Command path: {forbidden}"
        );
    }
    for issue in report["issues"].as_array().expect("Command issues") {
        if issue["kind"] == "command" {
            assert!(
                issue["path"].as_str().is_some_and(|path| {
                    Path::new(path).starts_with(repo.path())
                        || Path::new(path).starts_with(&global_root)
                }),
                "project Command issue escaped approved project/canonical roots: {issue:?}"
            );
        }
    }
}

#[test]
fn command_inventory_is_fail_closed_for_links_wrong_shapes_and_kind_scoped_collisions() {
    let home = TempDir::new().expect("temporary HOME");
    let repo = TempDir::new().expect("temporary project");
    let global_root = home.path().join(".agents-manager");
    let project_root = repo.path().join(".agents-manager");
    write_canonical_command(&global_root, "safe", "global-safe");
    let case_source = write_canonical_command(&global_root, "CaseCommand", "global-case");
    write(
        &global_root.join("skills/casecommand/SKILL.md"),
        "same case-insensitive spelling in another kind\n",
    );
    fs::create_dir_all(&project_root).expect("create project canonical root");
    let outside_canonical = TempDir::new().expect("outside canonical Commands");
    write(
        &outside_canonical.path().join("escaped.md"),
        COMMAND_EXTERNAL_SENTINEL,
    );
    symlink(outside_canonical.path(), project_root.join("commands"))
        .expect("escape canonical Commands root");

    let outside_platform = TempDir::new().expect("outside platform Commands");
    write(
        &outside_platform.path().join("commands/escaped.md"),
        COMMAND_EXTERNAL_SENTINEL,
    );
    symlink(outside_platform.path(), repo.path().join(".cursor")).expect("escape Cursor parent");
    let outside_file = outside_platform.path().join("linked.md");
    write(&outside_file, COMMAND_EXTERNAL_SENTINEL);
    fs::create_dir_all(repo.path().join(".claude/commands")).expect("create Claude Commands root");
    symlink(&outside_file, repo.path().join(".claude/commands/safe.md"))
        .expect("link final Command file");
    write(
        &repo
            .path()
            .join(".claude/commands/directory.md/must-not-recurse.md"),
        COMMAND_EXTERNAL_SENTINEL,
    );
    write(
        &repo.path().join(".claude/commands/casecommand.md"),
        &format!("{COMMAND_BODY_SENTINEL}-case-platform"),
    );

    let home_before = tree_snapshot(home.path());
    let repo_before = tree_snapshot(repo.path());
    let outside_before = tree_snapshot(outside_platform.path());
    let output = rules_inventory_output(home.path(), repo.path());
    assert!(
        output.status.success(),
        "Command security inventory failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 Command security inventory");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 Command security stderr");
    assert_command_redacted(&stdout, &stderr);
    assert_eq!(tree_snapshot(home.path()), home_before, "HOME changed");
    assert_eq!(tree_snapshot(repo.path()), repo_before, "project changed");
    assert_eq!(
        tree_snapshot(outside_platform.path()),
        outside_before,
        "outside Command owner changed"
    );
    let report: Value = serde_json::from_str(&stdout).expect("Command security inventory JSON");
    let assert_issue = |fragment: &str, reason: &str| {
        let issue = report["issues"]
            .as_array()
            .expect("Command issues")
            .iter()
            .find(|issue| {
                issue["kind"] == "command"
                    && issue["path"]
                        .as_str()
                        .is_some_and(|path| path.contains(fragment))
            })
            .unwrap_or_else(|| panic!("missing Command issue {reason}: {report:?}"));
        assert_eq!(issue["reason_code"], reason);
        assert_eq!(issue["blocking"], true);
    };
    assert_issue("/.agents-manager/commands", "unsafe_canonical_command_directory");
    assert_issue("/.cursor/commands", "unsafe_command_directory_parent");
    assert_issue("/.claude/commands/safe.md", "unsafe_command_file_symlink");
    assert_issue(
        "/.claude/commands/directory.md",
        "unsafe_command_file_non_regular",
    );

    let collisions = report["entries"]
        .as_array()
        .expect("Command entries")
        .iter()
        .filter(|entry| {
            entry["kind"] == "command"
                && matches!(entry["name"].as_str(), Some("CaseCommand" | "casecommand"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        collisions.len(),
        2,
        "Command collision rows: {collisions:?}"
    );
    assert!(collisions.iter().all(|entry| {
        entry["classification"] == "case_collision"
            && entry["reason_code"] == "case_only_name_collision"
            && entry["blocking"] == true
    }));
    let cross_kind = entry_named(&report, "casecommand", "/skills/");
    assert_ne!(cross_kind["classification"], "case_collision");
    assert_eq!(
        entry_named(&report, "CaseCommand", "/commands/")["canonical_path"],
        case_source.to_string_lossy().as_ref()
    );
    let serialized = serde_json::to_string(&report).expect("serialize Command security report");
    assert!(!serialized.contains("must-not-recurse"));
    assert!(!serialized.contains(outside_canonical.path().to_string_lossy().as_ref()));
    assert!(!serialized.contains(outside_platform.path().to_string_lossy().as_ref()));
    assert!(!report["entries"]
        .as_array()
        .expect("Command entries")
        .iter()
        .any(|entry| {
            entry["kind"] == "command"
                && entry["provenance"] != "canonical"
                && matches!(
                    entry["name"].as_str(),
                    Some("safe" | "directory" | "escaped")
                )
        }));
}

const HOOK_BODY_SENTINEL: &str = "t010-hook-body-must-never-serialize";
const HOOK_SECRET_SENTINEL: &str = "t010-hook-secret-must-never-serialize";
const HOOK_EXTERNAL_SENTINEL: &str = "t010-hook-external-must-never-be-read";

fn write_hook_unit(asset_root: &Path, name: &str, label: &str) -> PathBuf {
    let path = asset_root.join("hooks").join(name);
    write(
        &path,
        &format!("#!/bin/sh\n# {HOOK_BODY_SENTINEL}-{label}\nexit 0\n"),
    );
    path
}

fn write_hook_manifest(asset_root: &Path, commands: &[&str]) {
    let bindings = commands
        .iter()
        .map(|command| {
            serde_json::json!({
                "command": format!("{command} {HOOK_SECRET_SENTINEL}"),
                "matcher": "agents-manager"
            })
        })
        .collect::<Vec<_>>();
    write(
        &asset_root.join("hooks.json"),
        &serde_json::to_string(&serde_json::json!({
            "version": 1,
            "hooks": { "afterShellExecution": bindings }
        }))
        .expect("serialize canonical Hook manifest"),
    );
}

fn hook_entry_at<'a>(report: &'a Value, name: &str, path: &Path) -> &'a Value {
    report["entries"]
        .as_array()
        .expect("Hook entries")
        .iter()
        .find(|entry| {
            entry["kind"] == "hook"
                && entry["name"] == name
                && entry["path"] == path.to_string_lossy().as_ref()
        })
        .unwrap_or_else(|| panic!("missing Hook entry name={name:?} path={path:?}: {report:?}"))
}

fn assert_hook_report_redacted(stdout: &str, stderr: &str) {
    for sentinel in [
        HOOK_BODY_SENTINEL,
        HOOK_SECRET_SENTINEL,
        HOOK_EXTERNAL_SENTINEL,
    ] {
        assert!(
            !stdout.contains(sentinel) && !stderr.contains(sentinel),
            "Hook inventory leaked {sentinel}"
        );
    }
}

#[test]
fn global_hook_inventory_reports_four_current_pairs_and_codex_legacy_without_rendering() {
    let home = TempDir::new().expect("temporary HOME");
    let asset_root = home.path().join(".agents-manager");
    let source = write_hook_unit(&asset_root, "run.sh", "global");
    write_hook_manifest(&asset_root, &["./hooks/run.sh"]);

    for path in [
        home.path().join(".cursor/hooks/run.sh"),
        home.path().join(".codex/hooks/run.sh"),
        home.path().join(".claude/hooks/run.sh"),
        home.path().join(".hermes/hooks/run.sh"),
    ] {
        fs::create_dir_all(path.parent().expect("Hook target parent"))
            .expect("create Hook target parent");
        symlink(&source, &path).expect("link canonical Hook unit");
    }
    write(
        &home.path().join(".cursor/hooks.json"),
        &format!(
            r#"{{"version":1,"hooks":{{"afterShellExecution":[{{"command":".cursor/hooks/run.sh {HOOK_SECRET_SENTINEL}","managedBy":"agents-manager","hook":"run.sh"}}]}}}}"#
        ),
    );
    write(
        &home.path().join(".codex/hooks.json"),
        &format!(
            r#"{{"version":1,"hooks":{{"PostToolUse":[{{"matcher":"Bash","hooks":[{{"type":"command","command":".codex/hooks/run.sh {HOOK_SECRET_SENTINEL}","managedBy":"agents-manager","hook":"run.sh"}}]}}]}}}}"#
        ),
    );
    write(
        &home.path().join(".claude/settings.json"),
        &format!(
            r#"{{"permissions":{{"allow":[]}},"hooks":{{"PostToolUse":[{{"matcher":"Bash","hooks":[{{"type":"command","command":".claude/hooks/run.sh {HOOK_SECRET_SENTINEL}","managedBy":"agents-manager","hook":"run.sh"}}]}}]}}}}"#
        ),
    );
    write(
        &home.path().join(".hermes/config.yaml"),
        &format!(
            "model: foreign-setting\nhooks:\n  post_tool_call:\n    - command: .hermes/hooks/run.sh {HOOK_SECRET_SENTINEL}\n      managedBy: agents-manager\n      hook: run.sh\n"
        ),
    );
    write(
        &home.path().join(".codex/config.toml"),
        &format!(
            "[hooks]\nPostToolUse = [{{ matcher = \"Bash\", hooks = [{{ type = \"command\", command = \".codex/hooks/run.sh {HOOK_SECRET_SENTINEL}\", managedBy = \"agents-manager\", hook = \"run.sh\" }}] }}]\n"
        ),
    );

    let before = tree_snapshot(home.path());
    let output = rules_inventory_output(home.path(), &asset_root);
    assert!(
        output.status.success(),
        "global Hook inventory failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 global Hook inventory");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 global Hook stderr");
    assert_hook_report_redacted(&stdout, &stderr);
    assert_eq!(
        tree_snapshot(home.path()),
        before,
        "Hook inventory must be strictly read-only"
    );
    let report: Value = serde_json::from_str(&stdout).expect("global Hook inventory JSON");

    let canonical = hook_entry_at(&report, "run.sh", &source);
    assert_eq!(canonical["classification"], "canonical_source");
    assert_eq!(canonical["ownership_state"], "canonical_source");
    assert_eq!(canonical["provenance"], "canonical");
    assert_eq!(canonical["reason_code"], "canonical_source");
    assert_eq!(canonical["entry_key"], "hooks.run.sh");
    assert_eq!(canonical["format"], "file");
    assert_eq!(canonical["source_layer"], "global");
    assert_eq!(canonical["scope"], "global");

    for (platform, container, format) in [
        ("cursor", home.path().join(".cursor/hooks.json"), "json"),
        ("codex", home.path().join(".codex/hooks.json"), "json"),
        ("claude", home.path().join(".claude/settings.json"), "json"),
        ("hermes", home.path().join(".hermes/config.yaml"), "yaml"),
    ] {
        let binding = hook_entry_at(&report, "run.sh", &container);
        assert_eq!(binding["classification"], "foreign", "binding={binding:?}");
        assert_eq!(binding["ownership_state"], "foreign", "binding={binding:?}");
        assert_eq!(binding["provenance"], "platform_current");
        assert_eq!(binding["reason_code"], "platform_hook_binding_unowned");
        assert_eq!(binding["entry_key"], "hooks.run.sh");
        assert_eq!(binding["format"], format);
        assert_eq!(binding["consumers"], serde_json::json!([platform]));
        assert_eq!(binding["currently_consumed"], true);
        assert_eq!(binding["blocking"], true);
        assert_eq!(binding["owned"], false);
        assert_eq!(binding["selectable"], false);

        let script = hook_entry_at(
            &report,
            "run.sh",
            &home.path().join(format!(".{platform}/hooks/run.sh")),
        );
        assert_eq!(script["classification"], "managed_link");
        assert_eq!(script["ownership_state"], "managed_link");
        assert_eq!(script["provenance"], "platform_current");
        assert_eq!(script["reason_code"], "canonical_symlink");
        assert_eq!(script["format"], "file");
        assert_eq!(script["consumers"], serde_json::json!([platform]));
        assert_eq!(script["currently_consumed"], true);
        assert_eq!(script["owned"], true);
        assert_eq!(script["followed"], false);
        assert_eq!(script["canonical_path"], source.to_string_lossy().as_ref());
    }

    let legacy = hook_entry_at(&report, "run.sh", &home.path().join(".codex/config.toml"));
    assert_eq!(legacy["classification"], "foreign");
    assert_eq!(legacy["ownership_state"], "foreign");
    assert_eq!(legacy["provenance"], "platform_legacy");
    assert_eq!(legacy["reason_code"], "legacy_codex_inline_hook");
    assert_eq!(legacy["entry_key"], "hooks.run.sh");
    assert_eq!(legacy["format"], "toml");
    assert_eq!(legacy["currently_consumed"], true);
    assert_eq!(legacy["blocking"], false);
    assert_eq!(legacy["owned"], false);
    assert_eq!(legacy["selectable"], false);
}

#[test]
fn project_hook_inventory_blocks_half_cross_layer_and_unsafe_paths_without_scanning_home() {
    let home = TempDir::new().expect("temporary HOME");
    let repo = TempDir::new().expect("temporary project");
    let canonical_outside = TempDir::new().expect("outside canonical Hook bundle");
    let global_root = home.path().join(".agents-manager");
    let project_root = repo.path().join(".agents-manager");

    write_hook_unit(&global_root, "cross.sh", "global-cross");
    write_hook_manifest(&global_root, &["./hooks/cross.sh"]);
    let project_source = write_hook_unit(&project_root, "shared.sh", "project-shared");
    write_hook_unit(&project_root, "unsafe-child.sh", "project-child");
    write(
        &project_root.join("hooks/orphan-bundle/hook.yaml"),
        &format!("entry: scripts/run.sh\nmarker: {HOOK_BODY_SENTINEL}\n"),
    );
    write(
        &project_root.join("hooks/orphan-bundle/scripts/run.sh"),
        &format!("#!/bin/sh\n# {HOOK_BODY_SENTINEL}\n"),
    );
    write(
        &canonical_outside.path().join("run.sh"),
        HOOK_EXTERNAL_SENTINEL,
    );
    write(
        &project_root.join("hooks/canonical-link-bundle/hook.yaml"),
        "entry: scripts/run.sh\n",
    );
    fs::create_dir_all(project_root.join("hooks/canonical-link-bundle/scripts"))
        .expect("create canonical Hook bundle scripts");
    symlink(
        canonical_outside.path().join("run.sh"),
        project_root.join("hooks/canonical-link-bundle/scripts/run.sh"),
    )
    .expect("link canonical Hook bundle descendant");
    write(
        &project_root.join("hooks/platform-bundle/hook.yaml"),
        "entry: scripts/run.sh\n",
    );
    write(
        &project_root.join("hooks/platform-bundle/scripts/run.sh"),
        &format!("#!/bin/sh\n# {HOOK_BODY_SENTINEL}-platform-bundle\n"),
    );
    write_hook_manifest(
        &project_root,
        &[
            "./hooks/shared.sh",
            "./hooks/unsafe-child.sh",
            "./hooks/cross.sh",
            "./hooks/canonical-link-bundle/scripts/run.sh",
            "./hooks/platform-bundle/scripts/run.sh",
            "/tmp/absolute-hook.sh",
            "../parent-hook.sh",
        ],
    );

    write(
        &repo.path().join(".codex/hooks.json"),
        &format!(
            r#"{{"version":1,"hooks":{{"PostToolUse":[{{"hooks":[{{"type":"command","command":".codex/hooks/shared.sh {HOOK_SECRET_SENTINEL}","managedBy":"agents-manager","hook":"shared.sh"}},{{"type":"command","command":".codex/hooks/unsafe-child.sh {HOOK_SECRET_SENTINEL}","managedBy":"agents-manager","hook":"unsafe-child.sh"}}]}}]}}}}"#
        ),
    );
    fs::create_dir_all(repo.path().join(".codex/hooks")).expect("create Codex Hook root");
    symlink(&project_source, repo.path().join(".codex/hooks/shared.sh"))
        .expect("link project Hook");

    let outside = TempDir::new().expect("outside Hook owner");
    write(&outside.path().join("child.sh"), HOOK_EXTERNAL_SENTINEL);
    write(
        &outside.path().join("bundle/hook.yaml"),
        HOOK_EXTERNAL_SENTINEL,
    );
    symlink(
        outside.path().join("child.sh"),
        repo.path().join(".codex/hooks/unsafe-child.sh"),
    )
    .expect("link unsafe Hook child");
    symlink(outside.path(), repo.path().join(".cursor")).expect("link unsafe Hook parent");
    fs::create_dir_all(repo.path().join(".claude/hooks")).expect("create Claude Hook root");
    symlink(
        outside.path().join("bundle"),
        repo.path().join(".claude/hooks/orphan-bundle"),
    )
    .expect("link unsafe Hook bundle");
    write(
        &repo.path().join(".claude/hooks/platform-bundle/hook.yaml"),
        "entry: scripts/run.sh\n",
    );
    fs::create_dir_all(repo.path().join(".claude/hooks/platform-bundle/scripts"))
        .expect("create platform Hook bundle scripts");
    symlink(
        outside.path().join("child.sh"),
        repo.path()
            .join(".claude/hooks/platform-bundle/scripts/run.sh"),
    )
    .expect("link platform Hook bundle descendant");

    for path in [
        home.path().join(".cursor/hooks/home-only.sh"),
        home.path().join(".codex/hooks/home-only.sh"),
        home.path().join(".claude/hooks/home-only.sh"),
        home.path().join(".hermes/hooks/home-only.sh"),
        repo.path().join(".hermes/hooks/project-forbidden.sh"),
    ] {
        write(&path, HOOK_EXTERNAL_SENTINEL);
    }
    write(
        &repo.path().join(".hermes/config.yaml"),
        &format!(
            "hooks:\n  post_tool_call:\n    - command: .hermes/hooks/project-forbidden.sh {HOOK_SECRET_SENTINEL}\n"
        ),
    );

    let home_before = tree_snapshot(home.path());
    let repo_before = tree_snapshot(repo.path());
    let outside_before = tree_snapshot(outside.path());
    let canonical_outside_before = tree_snapshot(canonical_outside.path());
    let output = rules_inventory_output(home.path(), repo.path());
    assert!(
        output.status.success(),
        "project Hook inventory failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 project Hook inventory");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 project Hook stderr");
    assert_hook_report_redacted(&stdout, &stderr);
    assert_eq!(tree_snapshot(home.path()), home_before, "HOME changed");
    assert_eq!(tree_snapshot(repo.path()), repo_before, "project changed");
    assert_eq!(
        tree_snapshot(outside.path()),
        outside_before,
        "outside Hook owner changed"
    );
    assert_eq!(
        tree_snapshot(canonical_outside.path()),
        canonical_outside_before,
        "outside canonical Hook owner changed"
    );
    let report: Value = serde_json::from_str(&stdout).expect("project Hook inventory JSON");
    assert_eq!(report["scope"], "project");

    let shared = hook_entry_at(&report, "shared.sh", &project_source);
    assert_eq!(shared["classification"], "canonical_source");
    assert_eq!(shared["source_layer"], "project");
    assert_eq!(shared["entry_key"], "hooks.shared.sh");

    for (name, reason) in [
        ("cross.sh", "cross_layer_hook_pair"),
        ("absolute-hook.sh", "canonical_hook_command_absolute"),
        ("parent-hook.sh", "canonical_hook_command_parent_escape"),
    ] {
        let entry = hook_entry_at(&report, name, &project_root.join("hooks.json"));
        assert_eq!(entry["classification"], "foreign", "entry={entry:?}");
        assert_eq!(entry["ownership_state"], "foreign", "entry={entry:?}");
        assert_eq!(entry["provenance"], "canonical", "entry={entry:?}");
        assert_eq!(entry["reason_code"], reason, "entry={entry:?}");
        assert_eq!(entry["blocking"], true, "entry={entry:?}");
        assert_eq!(entry["owned"], false, "entry={entry:?}");
        assert_eq!(entry["selectable"], false, "entry={entry:?}");
    }
    let orphan = hook_entry_at(
        &report,
        "orphan-bundle",
        &project_root.join("hooks/orphan-bundle"),
    );
    assert_eq!(
        orphan["reason_code"],
        "canonical_hook_script_without_binding"
    );
    assert_eq!(orphan["format"], "directory");
    assert_eq!(orphan["blocking"], true);

    let codex_binding = hook_entry_at(&report, "shared.sh", &repo.path().join(".codex/hooks.json"));
    assert_eq!(codex_binding["classification"], "foreign");
    assert_eq!(
        codex_binding["reason_code"],
        "platform_hook_binding_unowned"
    );
    assert_eq!(codex_binding["blocking"], true);
    assert_eq!(
        codex_binding["trust_requirement"],
        "trusted_project_with_independent_review"
    );
    let codex_script = hook_entry_at(
        &report,
        "shared.sh",
        &repo.path().join(".codex/hooks/shared.sh"),
    );
    assert_eq!(codex_script["classification"], "managed_link");
    assert_eq!(
        codex_script["trust_requirement"],
        "trusted_project_with_independent_review"
    );

    let assert_issue = |fragment: &str, reason: &str| {
        let issue = report["issues"]
            .as_array()
            .expect("Hook issues")
            .iter()
            .find(|issue| {
                issue["kind"] == "hook"
                    && issue["reason_code"] == reason
                    && issue["path"]
                        .as_str()
                        .is_some_and(|path| path.contains(fragment))
            })
            .unwrap_or_else(|| panic!("missing Hook issue {reason}: {report:?}"));
        assert_eq!(issue["reason_code"], reason);
        assert_eq!(issue["blocking"], true);
    };
    assert_issue("/.cursor/hooks.json", "unsafe_hook_container_parent");
    assert_issue(
        "/.codex/hooks/unsafe-child.sh",
        "unsafe_hook_script_symlink",
    );
    assert_issue("/.codex/hooks.json", "hook_half_projection");
    assert_issue("/.claude/hooks/orphan-bundle", "unsafe_hook_script_symlink");
    assert_issue(
        "/.agents-manager/hooks/canonical-link-bundle",
        "unsafe_canonical_hook_bundle_symlink",
    );
    assert_issue(
        "/.claude/hooks/platform-bundle",
        "unsafe_hook_script_bundle_symlink",
    );

    let unsupported = report["unsupported"]
        .as_array()
        .expect("Hook unsupported rows")
        .iter()
        .find(|entry| {
            entry["kind"] == "hook" && entry["platform"] == "hermes" && entry["name"] == "shared.sh"
        })
        .expect("Hermes project Hook unsupported row");
    assert_eq!(
        unsupported["reason_code"],
        "hermes_project_hook_unsupported"
    );
    assert_eq!(unsupported["source_layer"], "project");
    assert_eq!(
        unsupported["canonical_path"],
        project_source.to_string_lossy().as_ref()
    );

    let serialized = serde_json::to_string(&report).expect("serialize project Hook report");
    assert!(!serialized.contains("home-only"));
    assert!(!serialized.contains("project-forbidden"));
    assert!(!serialized.contains(outside.path().to_string_lossy().as_ref()));
    assert!(!serialized.contains(canonical_outside.path().to_string_lossy().as_ref()));
}

#[test]
fn foreign_hook_inventory_halves_and_marker_command_mismatches_are_blocking() {
    let home = TempDir::new().expect("temporary HOME");
    let asset_root = home.path().join(".agents-manager");
    fs::create_dir_all(&asset_root).expect("create empty canonical root");
    write(
        &home.path().join(".cursor/hooks.json"),
        &format!(
            r#"{{"version":1,"hooks":{{"afterShellExecution":[{{"command":".cursor/hooks/binding-only.sh {HOOK_SECRET_SENTINEL}","managedBy":"agents-manager","hook":"binding-only.sh"}},{{"command":".cursor/hooks/actual.sh {HOOK_SECRET_SENTINEL}","managedBy":"agents-manager","hook":"claimed.sh"}}]}}}}"#
        ),
    );
    write(
        &home.path().join(".cursor/hooks/script-only.sh"),
        &format!("#!/bin/sh\n# {HOOK_BODY_SENTINEL}-script-only\n"),
    );
    write(
        &home.path().join(".cursor/hooks/claimed.sh"),
        &format!("#!/bin/sh\n# {HOOK_BODY_SENTINEL}-claimed\n"),
    );

    let before = tree_snapshot(home.path());
    let output = rules_inventory_output(home.path(), &asset_root);
    assert!(
        output.status.success(),
        "foreign Hook safety inventory failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 foreign Hook inventory");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 foreign Hook stderr");
    assert_hook_report_redacted(&stdout, &stderr);
    assert_eq!(
        tree_snapshot(home.path()),
        before,
        "foreign Hook inventory wrote"
    );
    let report: Value = serde_json::from_str(&stdout).expect("foreign Hook inventory JSON");

    let binding_only = hook_entry_at(
        &report,
        "binding-only.sh",
        &home.path().join(".cursor/hooks.json"),
    );
    assert_eq!(binding_only["classification"], "foreign");
    assert_eq!(binding_only["ownership_state"], "foreign");
    assert_eq!(binding_only["provenance"], "platform_current");
    assert_eq!(binding_only["blocking"], true);
    assert_eq!(binding_only["owned"], false);
    assert_eq!(binding_only["selectable"], false);

    let script_only = hook_entry_at(
        &report,
        "script-only.sh",
        &home.path().join(".cursor/hooks/script-only.sh"),
    );
    assert_eq!(script_only["classification"], "foreign");
    assert_eq!(script_only["ownership_state"], "foreign");
    assert_eq!(script_only["provenance"], "platform_current");
    assert_eq!(script_only["blocking"], true);
    assert_eq!(script_only["owned"], false);
    assert_eq!(script_only["selectable"], false);

    let assert_blocking_issue = |path: &Path, reason: &str| {
        let issue = report["issues"]
            .as_array()
            .expect("foreign Hook issues")
            .iter()
            .find(|issue| {
                issue["kind"] == "hook"
                    && issue["path"] == path.to_string_lossy().as_ref()
                    && issue["reason_code"] == reason
            })
            .unwrap_or_else(|| panic!("missing foreign Hook issue {reason}: {report:?}"));
        assert_eq!(issue["blocking"], true);
    };
    assert_blocking_issue(
        &home.path().join(".cursor/hooks.json"),
        "hook_half_projection",
    );
    assert_blocking_issue(
        &home.path().join(".cursor/hooks/script-only.sh"),
        "hook_half_projection",
    );
    assert_blocking_issue(
        &home.path().join(".cursor/hooks.json"),
        "hook_binding_script_name_mismatch",
    );
}

#[test]
fn workspace_inventory_uses_workspace_scope_overlay_and_never_falls_back_to_home_targets() {
    let home = TempDir::new().expect("temporary isolated HOME");
    let workspace = TempDir::new().expect("temporary workspace");
    let global_root = home.path().join(".agents-manager");
    let workspace_root = workspace.path().join(".agents-manager");

    let global_shared = write_canonical_command(&global_root, "shared", "global-shared");
    let global_only = write_canonical_command(&global_root, "global-only", "global-only");
    let workspace_shared = write_canonical_command(&workspace_root, "shared", "workspace-shared");
    let workspace_only =
        write_canonical_command(&workspace_root, "workspace-only", "workspace-only");
    write(
        &workspace.path().join(".cursor/commands/shared.md"),
        &fs::read_to_string(&workspace_shared).expect("read workspace canonical command"),
    );
    write(
        &workspace.path().join(".claude/commands/global-only.md"),
        &fs::read_to_string(&global_only).expect("read global canonical command"),
    );
    write(
        &workspace.path().join(".claude/commands/workspace-only.md"),
        &fs::read_to_string(&workspace_only).expect("read workspace-only canonical command"),
    );

    // These are valid global platform paths, but they are outside the workspace deploy base and
    // must not be read, hashed, or listed by a workspace inventory.
    write(
        &home.path().join(".agents/skills/home-only-skill/SKILL.md"),
        "workspace inventory must not scan this HOME skill\n",
    );
    write(
        &home.path().join(".cursor/commands/home-only-command.md"),
        COMMAND_EXTERNAL_SENTINEL,
    );
    write(
        &home
            .path()
            .join(".claude/commands/home-only-claude-command.md"),
        COMMAND_EXTERNAL_SENTINEL,
    );

    let home_before = tree_snapshot(home.path());
    let workspace_before = tree_snapshot(workspace.path());
    let mut command = Command::cargo_bin(BIN).expect("CLI binary");
    let output = command
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env_remove("AGENTS_MANAGER_ROOT")
        .env_remove("AGENTS_MANAGER_SECRETS_DIR")
        .env_remove("HERMES_SKILLS_DIR")
        .args(["--workspace", "--root"])
        .arg(workspace.path())
        .args(["migrate", "inventory", "--json"])
        .output()
        .expect("run workspace migration inventory");
    assert!(
        output.status.success(),
        "workspace inventory must be a read-only successful command: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        tree_snapshot(home.path()),
        home_before,
        "workspace inventory wrote HOME"
    );
    assert_eq!(
        tree_snapshot(workspace.path()),
        workspace_before,
        "workspace inventory wrote the workspace"
    );

    let stdout = String::from_utf8(output.stdout).expect("UTF-8 workspace inventory JSON");
    let report: Value = serde_json::from_str(&stdout).expect("workspace inventory JSON");
    assert_eq!(
        report["scope"], "workspace",
        "--workspace must never silently downgrade migration inventory to another scope: {report:?}"
    );

    let canonical_shared = report["entries"]
        .as_array()
        .expect("workspace inventory entries")
        .iter()
        .filter(|entry| {
            entry["kind"] == "command"
                && entry["name"] == "shared"
                && entry["provenance"] == "canonical"
        })
        .collect::<Vec<_>>();
    assert_eq!(
        canonical_shared.len(),
        2,
        "global and workspace layers stay visible"
    );
    assert_eq!(
        canonical_shared
            .iter()
            .map(|entry| entry["source_layer"]
                .as_str()
                .expect("canonical source layer"))
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["global", "workspace"]),
        "workspace overlay must retain global + workspace canonical provenance"
    );

    let workspace_current = entry_named(
        &report,
        "shared",
        workspace
            .path()
            .join(".cursor/commands/shared.md")
            .to_string_lossy()
            .as_ref(),
    );
    assert_command_source(
        workspace_current,
        "workspace",
        &workspace_shared,
        "workspace",
    );
    assert_eq!(workspace_current["currently_consumed"], true);

    let inherited_global = entry_named(
        &report,
        "global-only",
        workspace
            .path()
            .join(".claude/commands/global-only.md")
            .to_string_lossy()
            .as_ref(),
    );
    assert_command_source(inherited_global, "global", &global_only, "workspace");

    for path in report["entries"]
        .as_array()
        .expect("workspace inventory entries")
        .iter()
        .filter_map(|entry| entry["path"].as_str())
    {
        assert!(
            !Path::new(path).starts_with(home.path()) || Path::new(path).starts_with(&global_root),
            "workspace inventory scanned a HOME platform target instead of its workspace deploy base: {path}"
        );
    }
    let serialized = serde_json::to_string(&report).expect("serialize workspace inventory");
    for forbidden in [
        "home-only-skill",
        "home-only-command",
        "home-only-claude-command",
        COMMAND_EXTERNAL_SENTINEL,
    ] {
        assert!(
            !serialized.contains(forbidden),
            "workspace inventory must not read or serialize HOME-only platform data: {forbidden}"
        );
    }

    assert_ne!(
        workspace_current["canonical_path"],
        global_shared.to_string_lossy().as_ref(),
        "workspace overlay must override the same-name global canonical command"
    );
}

#[test]
fn workspace_inventory_without_root_fails_closed_with_json_error_and_writes_nothing() {
    let home = TempDir::new().expect("temporary isolated HOME");
    let before = tree_snapshot(home.path());
    let mut command = Command::cargo_bin(BIN).expect("CLI binary");
    let output = command
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env_remove("AGENTS_MANAGER_ROOT")
        .env_remove("AGENTS_MANAGER_SECRETS_DIR")
        .env_remove("HERMES_SKILLS_DIR")
        .args(["--workspace", "migrate", "inventory", "--json"])
        .output()
        .expect("run rootless workspace migration inventory");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        tree_snapshot(home.path()),
        before,
        "rootless workspace inventory must fail before touching HOME"
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("JSON error envelope");
    assert_eq!(report["error"]["code"], 2);
    assert!(
        report["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("requires --root <workspace-root>")),
        "rootless --workspace inventory must fail closed instead of silently choosing another scope: {report:?}"
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("\"scope\""),
        "fail-closed error must not emit a downgraded inventory report: {report:?}"
    );
}
