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

fn rules_inventory_output(home: &Path, root: &Path) -> std::process::Output {
    let mut command = Command::cargo_bin(BIN).expect("CLI binary");
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("AI_CONFIG_ROOT")
        .env_remove("AI_CONFIG_SECRETS_DIR")
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
    let asset_root = home.path().join(".ai-config");
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
    let global_source = home.path().join(".ai-config/rules/shared.mdc");
    let global_only_source = home.path().join(".ai-config/rules/global-only.mdc");
    let project_source = repo.path().join(".ai-config/rules/shared.mdc");
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
    let asset_root = home.path().join(".ai-config");
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

    let legacy_source = entry_named(&report, "source-legacy-only", "/.ai-config/mcp.json");
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
    let global_root = home.path().join(".ai-config");
    let project_root = repo.path().join(".ai-config");
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
    let asset_root = home.path().join(".ai-config");
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
    let asset_root = home.path().join(".ai-config");
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
        "/.ai-config/mcp/servers",
        "unsafe_canonical_mcp_parent_symlink",
    );
    assert_mcp_issue(
        &report,
        "/.ai-config/mcp.json",
        "invalid_legacy_canonical_mcp",
    );
    assert!(
        !serde_json::to_string(&report)
            .expect("serialize canonical issues")
            .contains("escaped"),
        "canonical MCP parent symlink contents must not be inspected"
    );

    let mismatch_home = TempDir::new().expect("mismatch HOME");
    let mismatch_root = mismatch_home.path().join(".ai-config");
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
        "/.ai-config/mcp.json",
        "unsafe_legacy_canonical_mcp_symlink",
    );
    assert!(
        !serde_json::to_string(&mismatch_report)
            .expect("serialize mismatch issues")
            .contains("different-name"),
        "a mismatched canonical server must not become effective"
    );
}
