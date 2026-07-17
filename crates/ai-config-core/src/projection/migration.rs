//! Read-only inventory for source-first skill migration.
//!
//! Paths are injected by the caller. This module never resolves HOME, creates directories,
//! follows unknown links, opens a ledger, or emits asset bodies.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::CoreError;
use crate::model::AssetKind;
use crate::projection::fingerprint::path_content_digest;

pub const MIGRATION_INVENTORY_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryRequest {
    pub asset_root: Utf8PathBuf,
    pub deploy_base: Utf8PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum InventoryProvenance {
    Canonical,
    PlatformCurrent,
    PlatformLegacy,
    CcSwitch,
    PluginBuiltin,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InventoryClassification {
    CanonicalSource,
    ManagedLink,
    LegacyMarkerCandidate,
    Equivalent,
    Foreign,
    UnsafeLink,
    BrokenLink,
    ExternalOwned,
    CaseCollision,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InventoryOwnershipState {
    CanonicalSource,
    ManagedLink,
    Equivalent,
    Foreign,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationInventoryEntry {
    pub name: String,
    pub path: Utf8PathBuf,
    pub kind: AssetKind,
    pub classification: InventoryClassification,
    pub provenance: InventoryProvenance,
    pub reason_code: String,
    pub content_digest: Option<String>,
    pub currently_consumed: bool,
    pub blocking: bool,
    pub ownership_state: InventoryOwnershipState,
    pub owned: bool,
    pub selectable: bool,
    pub followed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationInventory {
    pub schema_version: u16,
    pub plan_digest: String,
    pub entries: Vec<MigrationInventoryEntry>,
}

struct CanonicalSkill {
    path: Utf8PathBuf,
    canonical_path: Utf8PathBuf,
    digest: String,
}

struct ExternalSkill {
    path: Utf8PathBuf,
    digest: Option<String>,
    reason_code: &'static str,
}

pub fn inventory(request: &InventoryRequest) -> Result<MigrationInventory, CoreError> {
    let canonical = canonical_skills(&request.asset_root.join("skills"))?;
    let cc_switch = external_skills(
        &request.deploy_base.join(".cc-switch/skills"),
        "cc_switch_owned",
    )?;
    let plugin_builtin = external_skills(
        &request.deploy_base.join(".codex/skills/.system"),
        "plugin_or_builtin",
    )?;
    let external = cc_switch
        .iter()
        .chain(plugin_builtin.iter())
        .collect::<Vec<_>>();
    let mut entries = canonical
        .iter()
        .map(|(name, skill)| MigrationInventoryEntry {
            name: name.clone(),
            path: skill.path.clone(),
            kind: AssetKind::Skill,
            classification: InventoryClassification::CanonicalSource,
            provenance: InventoryProvenance::Canonical,
            reason_code: "canonical_source".to_owned(),
            content_digest: Some(skill.digest.clone()),
            currently_consumed: false,
            blocking: false,
            ownership_state: InventoryOwnershipState::CanonicalSource,
            owned: false,
            selectable: false,
            followed: false,
        })
        .collect::<Vec<_>>();

    scan_target_root(
        &request.deploy_base.join(".agents/skills"),
        InventoryProvenance::PlatformCurrent,
        true,
        &canonical,
        &external,
        &mut entries,
    )?;
    scan_target_root(
        &request.deploy_base.join(".claude/skills"),
        InventoryProvenance::PlatformCurrent,
        true,
        &canonical,
        &external,
        &mut entries,
    )?;
    scan_target_root(
        &request.deploy_base.join(".cursor/skills"),
        InventoryProvenance::PlatformLegacy,
        true,
        &canonical,
        &external,
        &mut entries,
    )?;
    scan_target_root(
        &request.deploy_base.join(".codex/skills"),
        InventoryProvenance::PlatformLegacy,
        false,
        &canonical,
        &external,
        &mut entries,
    )?;
    append_external_entries(&cc_switch, InventoryProvenance::CcSwitch, &mut entries);
    append_external_entries(
        &plugin_builtin,
        InventoryProvenance::PluginBuiltin,
        &mut entries,
    );

    mark_case_collisions(&mut entries);
    entries.sort_by(|left, right| {
        (left.name.as_str(), &left.provenance, left.path.as_str()).cmp(&(
            right.name.as_str(),
            &right.provenance,
            right.path.as_str(),
        ))
    });
    let encoded = serde_json::to_vec(&entries).map_err(CoreError::Json)?;
    Ok(MigrationInventory {
        schema_version: MIGRATION_INVENTORY_SCHEMA_VERSION,
        plan_digest: hex::encode(Sha256::digest(encoded)),
        entries,
    })
}

fn canonical_skills(root: &Utf8Path) -> Result<BTreeMap<String, CanonicalSkill>, CoreError> {
    let mut skills = BTreeMap::new();
    for (name, path, metadata) in direct_entries(root)? {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        let skill_file = path.join("SKILL.md");
        if !is_regular_file(&skill_file)? {
            continue;
        }
        let canonical_path = fs::canonicalize(path.as_std_path())
            .map_err(CoreError::Io)
            .and_then(utf8_path)?;
        skills.insert(
            name,
            CanonicalSkill {
                digest: path_content_digest(&path)?,
                path,
                canonical_path,
            },
        );
    }
    Ok(skills)
}

fn scan_target_root(
    root: &Utf8Path,
    provenance: InventoryProvenance,
    currently_consumed: bool,
    canonical: &BTreeMap<String, CanonicalSkill>,
    external: &[&ExternalSkill],
    entries: &mut Vec<MigrationInventoryEntry>,
) -> Result<(), CoreError> {
    for (name, path, metadata) in direct_entries(root)? {
        if provenance == InventoryProvenance::PlatformLegacy
            && root.ends_with(".codex/skills")
            && name == ".system"
        {
            continue;
        }
        let canonical_skill = canonical.get(&name);
        let (classification, reason_code, content_digest, blocking, followed) =
            if metadata.file_type().is_symlink() {
                classify_link(&path, canonical_skill, external)?
            } else if metadata.is_dir() {
                classify_regular_skill(&path, canonical_skill)?
            } else {
                (
                    InventoryClassification::Foreign,
                    "unsupported_skill_entry".to_owned(),
                    None,
                    true,
                    false,
                )
            };
        let (ownership_state, owned, selectable) = ownership_fields(&classification);
        entries.push(MigrationInventoryEntry {
            name,
            path,
            kind: AssetKind::Skill,
            classification,
            provenance: provenance.clone(),
            reason_code,
            content_digest,
            currently_consumed,
            blocking,
            ownership_state,
            owned,
            selectable,
            followed,
        });
    }
    Ok(())
}

fn external_skills(
    root: &Utf8Path,
    reason_code: &'static str,
) -> Result<Vec<ExternalSkill>, CoreError> {
    direct_entries(root)?
        .into_iter()
        .map(|(_, path, metadata)| {
            let digest = (!metadata.file_type().is_symlink() && metadata.is_dir())
                .then(|| path_content_digest(&path))
                .transpose()?;
            Ok(ExternalSkill {
                path,
                digest,
                reason_code,
            })
        })
        .collect()
}

fn append_external_entries(
    external: &[ExternalSkill],
    provenance: InventoryProvenance,
    entries: &mut Vec<MigrationInventoryEntry>,
) {
    for skill in external {
        entries.push(MigrationInventoryEntry {
            name: skill.path.file_name().unwrap_or_default().to_owned(),
            path: skill.path.clone(),
            kind: AssetKind::Skill,
            classification: InventoryClassification::ExternalOwned,
            provenance: provenance.clone(),
            reason_code: skill.reason_code.to_owned(),
            content_digest: skill.digest.clone(),
            currently_consumed: false,
            blocking: false,
            ownership_state: InventoryOwnershipState::Foreign,
            owned: false,
            selectable: false,
            followed: false,
        });
    }
}

fn classify_link(
    path: &Utf8Path,
    canonical: Option<&CanonicalSkill>,
    external: &[&ExternalSkill],
) -> Result<(InventoryClassification, String, Option<String>, bool, bool), CoreError> {
    let target = fs::read_link(path.as_std_path()).map_err(CoreError::Io)?;
    let target = if target.is_absolute() {
        target
    } else {
        path.parent()
            .map(|parent| parent.as_std_path().join(&target))
            .unwrap_or(target)
    };
    let target = utf8_path(target)?;
    let lexical_target = lexical_normalize(&target);
    if canonical.is_some_and(|skill| {
        lexical_target == lexical_normalize(&skill.path)
            || lexical_target == lexical_normalize(&skill.canonical_path)
    }) {
        return Ok((
            InventoryClassification::ManagedLink,
            "canonical_symlink".to_owned(),
            canonical.map(|skill| skill.digest.clone()),
            false,
            false,
        ));
    }
    if let Some(external) = external
        .iter()
        .find(|skill| lexical_target == lexical_normalize(&skill.path))
    {
        return Ok((
            InventoryClassification::ExternalOwned,
            external.reason_code.to_owned(),
            None,
            false,
            false,
        ));
    }
    match fs::symlink_metadata(target.as_std_path()) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((
            InventoryClassification::BrokenLink,
            "broken_symlink".to_owned(),
            None,
            true,
            false,
        )),
        Ok(_) | Err(_) => Ok((
            InventoryClassification::UnsafeLink,
            "unknown_root_link".to_owned(),
            None,
            true,
            false,
        )),
    }
}

fn classify_regular_skill(
    path: &Utf8Path,
    canonical: Option<&CanonicalSkill>,
) -> Result<(InventoryClassification, String, Option<String>, bool, bool), CoreError> {
    let digest = path_content_digest(path)?;
    if is_regular_file(&path.join(".ai-config-deploy.json"))? {
        return Ok((
            InventoryClassification::LegacyMarkerCandidate,
            "legacy_marker_present".to_owned(),
            Some(digest),
            false,
            false,
        ));
    }
    match canonical {
        Some(skill) if skill.digest == digest => Ok((
            InventoryClassification::Equivalent,
            "unmarked_equal_copy".to_owned(),
            Some(digest),
            false,
            false,
        )),
        Some(_) => Ok((
            InventoryClassification::Foreign,
            "different_content".to_owned(),
            Some(digest),
            true,
            false,
        )),
        None => Ok((
            InventoryClassification::Foreign,
            "no_canonical_skill".to_owned(),
            Some(digest),
            true,
            false,
        )),
    }
}

fn direct_entries(root: &Utf8Path) -> Result<Vec<(String, Utf8PathBuf, fs::Metadata)>, CoreError> {
    let metadata = match fs::symlink_metadata(root.as_std_path()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(CoreError::Io(error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(root.as_std_path()).map_err(CoreError::Io)? {
        let entry = entry.map_err(CoreError::Io)?;
        let path = utf8_path(entry.path())?;
        let name = match path.file_name() {
            Some(name) if !name.is_empty() => name.to_owned(),
            _ => continue,
        };
        let metadata = fs::symlink_metadata(path.as_std_path()).map_err(CoreError::Io)?;
        entries.push((name, path, metadata));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(entries)
}

fn is_regular_file(path: &Utf8Path) -> Result<bool, CoreError> {
    match fs::symlink_metadata(path.as_std_path()) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(CoreError::Io(error)),
    }
}

fn utf8_path(path: std::path::PathBuf) -> Result<Utf8PathBuf, CoreError> {
    Utf8PathBuf::from_path_buf(path)
        .map_err(|path| CoreError::InvalidPath(path.to_string_lossy().into_owned()))
}

fn mark_case_collisions(entries: &mut [MigrationInventoryEntry]) {
    let mut variants = BTreeMap::<String, BTreeSet<String>>::new();
    for entry in entries.iter() {
        variants
            .entry(entry.name.to_lowercase())
            .or_default()
            .insert(entry.name.clone());
    }
    for entry in entries {
        if variants
            .get(&entry.name.to_lowercase())
            .is_some_and(|names| names.len() > 1)
        {
            entry.classification = InventoryClassification::CaseCollision;
            entry.reason_code = "case_only_name_collision".to_owned();
            entry.blocking = true;
            entry.ownership_state = InventoryOwnershipState::Foreign;
            entry.owned = false;
            entry.selectable = false;
            entry.followed = false;
        }
    }
}

fn ownership_fields(
    classification: &InventoryClassification,
) -> (InventoryOwnershipState, bool, bool) {
    match classification {
        InventoryClassification::ManagedLink => (InventoryOwnershipState::ManagedLink, true, false),
        InventoryClassification::Equivalent => (InventoryOwnershipState::Equivalent, false, true),
        InventoryClassification::CanonicalSource => {
            (InventoryOwnershipState::CanonicalSource, false, false)
        }
        InventoryClassification::LegacyMarkerCandidate
        | InventoryClassification::Foreign
        | InventoryClassification::UnsafeLink
        | InventoryClassification::BrokenLink
        | InventoryClassification::ExternalOwned
        | InventoryClassification::CaseCollision => {
            (InventoryOwnershipState::Foreign, false, false)
        }
    }
}

fn lexical_normalize(path: &Utf8Path) -> Utf8PathBuf {
    let mut normalized = Utf8PathBuf::new();
    for component in path.components() {
        match component.as_str() {
            "." => {}
            ".." => {
                normalized.pop();
            }
            value => normalized.push(value),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::Utf8Path;
    use tempfile::TempDir;

    use super::*;

    fn write(path: &Utf8Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap().as_std_path()).unwrap();
        fs::write(path.as_std_path(), contents).unwrap();
    }

    fn request(temp: &TempDir) -> InventoryRequest {
        let root = Utf8Path::from_path(temp.path()).unwrap();
        InventoryRequest {
            asset_root: root.join(".ai-config"),
            deploy_base: root.join("repo"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn unknown_skill_link_is_unsafe_and_never_reads_its_sentinel_target() {
        let temp = TempDir::new().unwrap();
        let request = request(&temp);
        write(
            &request.asset_root.join("skills/demo/SKILL.md"),
            "canonical skill\n",
        );
        let outside = Utf8Path::from_path(temp.path()).unwrap().join("outside");
        write(
            &outside.join("SKILL.md"),
            "DO-NOT-READ-INVENTORY-SENTINEL\n",
        );
        fs::create_dir_all(request.deploy_base.join(".agents/skills").as_std_path()).unwrap();
        std::os::unix::fs::symlink(
            outside.as_std_path(),
            request
                .deploy_base
                .join(".agents/skills/demo")
                .as_std_path(),
        )
        .unwrap();

        let result = inventory(&request).unwrap();
        let entry = result
            .entries
            .iter()
            .find(|entry| entry.path == request.deploy_base.join(".agents/skills/demo"))
            .expect("current target inventory entry");
        assert_eq!(entry.classification, InventoryClassification::UnsafeLink);
        assert_eq!(entry.content_digest, None);
        assert_eq!(entry.ownership_state, InventoryOwnershipState::Foreign);
        assert!(!entry.owned && !entry.selectable && !entry.followed);
    }

    #[test]
    fn legacy_marker_is_only_a_candidate_not_ownership() {
        let temp = TempDir::new().unwrap();
        let request = request(&temp);
        write(
            &request.asset_root.join("skills/demo/SKILL.md"),
            "canonical skill\n",
        );
        write(
            &request.deploy_base.join(".cursor/skills/demo/SKILL.md"),
            "canonical skill\n",
        );
        write(
            &request
                .deploy_base
                .join(".cursor/skills/demo/.ai-config-deploy.json"),
            r#"{"version":1,"source":"/must-not-prove-ownership"}"#,
        );

        let result = inventory(&request).unwrap();
        let entry = result
            .entries
            .iter()
            .find(|entry| entry.path == request.deploy_base.join(".cursor/skills/demo"))
            .expect("legacy target inventory entry");
        assert_eq!(
            entry.classification,
            InventoryClassification::LegacyMarkerCandidate
        );
        assert_eq!(entry.reason_code, "legacy_marker_present");
        assert!(!entry.blocking);
        assert_eq!(entry.ownership_state, InventoryOwnershipState::Foreign);
        assert!(!entry.owned && !entry.selectable && !entry.followed);
    }

    #[test]
    fn case_only_skill_names_are_blocking_and_plan_digest_is_deterministic() {
        let temp = TempDir::new().unwrap();
        let request = request(&temp);
        write(
            &request.asset_root.join("skills/Demo/SKILL.md"),
            "canonical upper\n",
        );
        write(
            &request.deploy_base.join(".agents/skills/demo/SKILL.md"),
            "platform lower\n",
        );

        let first = inventory(&request).unwrap();
        let second = inventory(&request).unwrap();
        assert_eq!(first.plan_digest, second.plan_digest);
        assert_eq!(first.entries.len(), 2);
        assert!(first.entries.iter().all(|entry| {
            entry.classification == InventoryClassification::CaseCollision && entry.blocking
        }));
    }
}
