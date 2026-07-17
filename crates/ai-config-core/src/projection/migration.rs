//! Read-only inventory for source-first migration.
//!
//! Callers inject every source and deployment root.  This module never resolves HOME, creates
//! directories, follows unknown links, opens a ledger, or emits asset bodies.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::CoreError;
use crate::model::{AssetKind, PlatformId};
use crate::projection::fingerprint::path_content_digest;
use crate::projection::model::SourceLayer;

pub const MIGRATION_INVENTORY_SCHEMA_VERSION: u16 = 1;

/// A caller-approved canonical source layer.  The migration core does not infer this from HOME.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalLayerRoot {
    pub layer: SourceLayer,
    pub asset_root: Utf8PathBuf,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum InventoryScope {
    Global,
    Workspace,
    Project,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryRequest {
    pub canonical_layers: Vec<CanonicalLayerRoot>,
    pub deploy_base: Utf8PathBuf,
    pub scope: InventoryScope,
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
    /// Effective canonical source selected for this target, when one exists.
    pub source_layer: Option<SourceLayer>,
    pub canonical_path: Option<Utf8PathBuf>,
    /// Platforms which can consume this exact target.  Empty means a source or external row.
    pub consumers: Vec<PlatformId>,
    pub scope: InventoryScope,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationInventory {
    pub schema_version: u16,
    pub scope: InventoryScope,
    pub plan_digest: String,
    pub entries: Vec<MigrationInventoryEntry>,
}

#[derive(Debug, Clone)]
struct CanonicalAsset {
    path: Utf8PathBuf,
    resolved_path: Utf8PathBuf,
    digest: String,
    layer: SourceLayer,
    kind: AssetKind,
}

#[derive(Debug, Clone)]
struct ExternalSkill {
    path: Utf8PathBuf,
    digest: Option<String>,
    reason_code: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryShape {
    Directory,
    RegularFile,
    Symlink,
    Other,
}

struct DirectEntry {
    name: String,
    path: Utf8PathBuf,
    shape: EntryShape,
}

pub fn inventory(request: &InventoryRequest) -> Result<MigrationInventory, CoreError> {
    let layers = sorted_layers(&request.canonical_layers)?;
    let mut raw_canonical = Vec::new();
    let mut effective = BTreeMap::new();
    for root in layers {
        for asset in canonical_assets(root)? {
            effective.insert(
                (asset_kind_order(asset.kind), asset_name(&asset)?),
                asset.clone(),
            );
            raw_canonical.push(asset);
        }
    }

    let (cc_switch, plugin_builtin) = if request.scope == InventoryScope::Global {
        (
            external_skills(
                &request.deploy_base.join(".cc-switch/skills"),
                "cc_switch_owned",
            )?,
            external_skills(
                &request.deploy_base.join(".codex/skills/.system"),
                "plugin_or_builtin",
            )?,
        )
    } else {
        (Vec::new(), Vec::new())
    };
    let external = cc_switch
        .iter()
        .chain(plugin_builtin.iter())
        .collect::<Vec<_>>();

    let mut entries = raw_canonical
        .iter()
        .map(|asset| canonical_entry(asset, request.scope))
        .collect::<Vec<_>>();

    scan_skill_root(
        &request.deploy_base.join(".agents/skills"),
        InventoryProvenance::PlatformCurrent,
        true,
        vec![PlatformId::Cursor, PlatformId::Codex],
        &effective,
        &external,
        request.scope,
        &mut entries,
    )?;
    scan_skill_root(
        &request.deploy_base.join(".claude/skills"),
        InventoryProvenance::PlatformCurrent,
        true,
        vec![PlatformId::Claude],
        &effective,
        &external,
        request.scope,
        &mut entries,
    )?;
    scan_skill_root(
        &request.deploy_base.join(".cursor/skills"),
        InventoryProvenance::PlatformLegacy,
        true,
        vec![PlatformId::Cursor],
        &effective,
        &external,
        request.scope,
        &mut entries,
    )?;
    scan_skill_root(
        &request.deploy_base.join(".codex/skills"),
        InventoryProvenance::PlatformLegacy,
        false,
        vec![PlatformId::Codex],
        &effective,
        &external,
        request.scope,
        &mut entries,
    )?;
    scan_rules(request, &effective, &mut entries)?;
    append_external_entries(
        &cc_switch,
        InventoryProvenance::CcSwitch,
        request.scope,
        &mut entries,
    );
    append_external_entries(
        &plugin_builtin,
        InventoryProvenance::PluginBuiltin,
        request.scope,
        &mut entries,
    );

    mark_case_collisions(&mut entries);
    entries.sort_by(|left, right| {
        (
            asset_kind_order(left.kind),
            &left.scope,
            left.source_layer,
            &left.provenance,
            left.name.as_str(),
            left.path.as_str(),
        )
            .cmp(&(
                asset_kind_order(right.kind),
                &right.scope,
                right.source_layer,
                &right.provenance,
                right.name.as_str(),
                right.path.as_str(),
            ))
    });
    let encoded = serde_json::to_vec(&entries).map_err(CoreError::Json)?;
    Ok(MigrationInventory {
        schema_version: MIGRATION_INVENTORY_SCHEMA_VERSION,
        scope: request.scope,
        plan_digest: hex::encode(Sha256::digest(encoded)),
        entries,
    })
}

fn sorted_layers(layers: &[CanonicalLayerRoot]) -> Result<Vec<&CanonicalLayerRoot>, CoreError> {
    let mut layers = layers.iter().collect::<Vec<_>>();
    layers.sort_by_key(|root| root.layer);
    let mut seen = BTreeSet::new();
    for root in &layers {
        if !seen.insert(root.layer) {
            return Err(CoreError::InvalidPath(format!(
                "duplicate canonical source layer: {:?}",
                root.layer
            )));
        }
    }
    Ok(layers)
}

fn canonical_assets(root: &CanonicalLayerRoot) -> Result<Vec<CanonicalAsset>, CoreError> {
    let mut assets = Vec::new();
    for entry in direct_lstat_entries(&root.asset_root.join("skills"))? {
        if entry.shape != EntryShape::Directory || !is_regular_file(&entry.path.join("SKILL.md"))? {
            continue;
        }
        assets.push(canonical_asset(root.layer, AssetKind::Skill, entry.path)?);
    }
    for entry in direct_lstat_entries(&root.asset_root.join("rules"))? {
        if entry.shape != EntryShape::RegularFile || entry.path.extension() != Some("mdc") {
            continue;
        }
        assets.push(canonical_asset(root.layer, AssetKind::Rule, entry.path)?);
    }
    Ok(assets)
}

fn canonical_asset(
    layer: SourceLayer,
    kind: AssetKind,
    path: Utf8PathBuf,
) -> Result<CanonicalAsset, CoreError> {
    // The direct child was selected from an explicitly injected canonical root.  It is the only
    // place migration inventory resolves a link-like filesystem path.
    let resolved_path = fs::canonicalize(path.as_std_path())
        .map_err(CoreError::Io)
        .and_then(utf8_path)?;
    Ok(CanonicalAsset {
        digest: path_content_digest(&path)?,
        path,
        resolved_path,
        layer,
        kind,
    })
}

fn asset_name(asset: &CanonicalAsset) -> Result<String, CoreError> {
    match asset.kind {
        AssetKind::Rule => asset
            .path
            .file_stem()
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                CoreError::InvalidPath(format!("invalid canonical rule: {}", asset.path))
            }),
        _ => asset
            .path
            .file_name()
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                CoreError::InvalidPath(format!("invalid canonical asset: {}", asset.path))
            }),
    }
}

fn canonical_entry(asset: &CanonicalAsset, scope: InventoryScope) -> MigrationInventoryEntry {
    MigrationInventoryEntry {
        name: asset_name(asset).expect("canonical asset was validated before inventory entry"),
        path: asset.path.clone(),
        kind: asset.kind,
        classification: InventoryClassification::CanonicalSource,
        provenance: InventoryProvenance::Canonical,
        reason_code: "canonical_source".to_owned(),
        content_digest: Some(asset.digest.clone()),
        currently_consumed: false,
        blocking: false,
        ownership_state: InventoryOwnershipState::CanonicalSource,
        owned: false,
        selectable: false,
        followed: false,
        source_layer: Some(asset.layer),
        canonical_path: Some(asset.path.clone()),
        consumers: Vec::new(),
        scope,
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_skill_root(
    root: &Utf8Path,
    provenance: InventoryProvenance,
    currently_consumed: bool,
    consumers: Vec<PlatformId>,
    canonical: &BTreeMap<(u8, String), CanonicalAsset>,
    external: &[&ExternalSkill],
    scope: InventoryScope,
    entries: &mut Vec<MigrationInventoryEntry>,
) -> Result<(), CoreError> {
    for entry in direct_lstat_entries(root)? {
        if provenance == InventoryProvenance::PlatformLegacy
            && root.ends_with(".codex/skills")
            && entry.name == ".system"
        {
            continue;
        }
        let source = canonical.get(&(asset_kind_order(AssetKind::Skill), entry.name.clone()));
        let (classification, reason_code, content_digest, blocking, followed) = match entry.shape {
            EntryShape::Symlink => classify_link(&entry.path, source, external)?,
            EntryShape::Directory => {
                classify_regular_asset(&entry.path, source, true, "no_canonical_skill")?
            }
            EntryShape::RegularFile | EntryShape::Other => (
                InventoryClassification::Foreign,
                "unsupported_skill_entry".to_owned(),
                None,
                true,
                false,
            ),
        };
        entries.push(target_entry(
            entry.name,
            entry.path,
            AssetKind::Skill,
            classification,
            provenance.clone(),
            reason_code,
            content_digest,
            currently_consumed,
            blocking,
            followed,
            source,
            consumers.clone(),
            scope,
        ));
    }
    Ok(())
}

fn scan_rules(
    request: &InventoryRequest,
    canonical: &BTreeMap<(u8, String), CanonicalAsset>,
    entries: &mut Vec<MigrationInventoryEntry>,
) -> Result<(), CoreError> {
    if request.scope != InventoryScope::Global {
        scan_rule_root(
            &request.deploy_base.join(".cursor/rules"),
            "mdc",
            InventoryProvenance::PlatformCurrent,
            vec![PlatformId::Cursor, PlatformId::Hermes],
            canonical,
            request.scope,
            entries,
        )?;
    }
    scan_rule_root(
        &request.deploy_base.join(".claude/rules"),
        "md",
        InventoryProvenance::PlatformCurrent,
        vec![PlatformId::Claude],
        canonical,
        request.scope,
        entries,
    )?;
    scan_codex_execution_policies(
        &request.deploy_base.join(".codex/rules"),
        canonical,
        request.scope,
        entries,
    )
}

fn scan_rule_root(
    root: &Utf8Path,
    extension: &str,
    provenance: InventoryProvenance,
    consumers: Vec<PlatformId>,
    canonical: &BTreeMap<(u8, String), CanonicalAsset>,
    scope: InventoryScope,
    entries: &mut Vec<MigrationInventoryEntry>,
) -> Result<(), CoreError> {
    for entry in direct_lstat_entries(root)? {
        if entry.path.extension() != Some(extension) {
            continue;
        }
        let name = entry
            .path
            .file_stem()
            .filter(|name| !name.is_empty())
            .unwrap_or(&entry.name)
            .to_owned();
        let source = canonical.get(&(asset_kind_order(AssetKind::Rule), name.clone()));
        let (classification, reason_code, content_digest, blocking, followed) = match entry.shape {
            EntryShape::Symlink => classify_link(&entry.path, source, &[])?,
            EntryShape::RegularFile => {
                classify_regular_asset(&entry.path, source, false, "no_canonical_rule")?
            }
            EntryShape::Directory | EntryShape::Other => (
                InventoryClassification::Foreign,
                "unsupported_rule_entry".to_owned(),
                None,
                true,
                false,
            ),
        };
        entries.push(target_entry(
            name,
            entry.path,
            AssetKind::Rule,
            classification,
            provenance.clone(),
            reason_code,
            content_digest,
            true,
            blocking,
            followed,
            source,
            consumers.clone(),
            scope,
        ));
    }
    Ok(())
}

fn scan_codex_execution_policies(
    root: &Utf8Path,
    canonical: &BTreeMap<(u8, String), CanonicalAsset>,
    scope: InventoryScope,
    entries: &mut Vec<MigrationInventoryEntry>,
) -> Result<(), CoreError> {
    for entry in direct_lstat_entries(root)? {
        if entry.path.extension() != Some("rules") {
            continue;
        }
        let name = entry
            .path
            .file_stem()
            .filter(|name| !name.is_empty())
            .unwrap_or(&entry.name)
            .to_owned();
        let digest = (entry.shape == EntryShape::RegularFile)
            .then(|| path_content_digest(&entry.path))
            .transpose()?;
        let source = canonical.get(&(asset_kind_order(AssetKind::Rule), name.clone()));
        entries.push(target_entry(
            name,
            entry.path,
            AssetKind::Rule,
            InventoryClassification::ExternalOwned,
            InventoryProvenance::PlatformLegacy,
            "codex_execution_policy".to_owned(),
            digest,
            true,
            false,
            false,
            source,
            vec![PlatformId::Codex],
            scope,
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn target_entry(
    name: String,
    path: Utf8PathBuf,
    kind: AssetKind,
    classification: InventoryClassification,
    provenance: InventoryProvenance,
    reason_code: String,
    content_digest: Option<String>,
    currently_consumed: bool,
    blocking: bool,
    followed: bool,
    source: Option<&CanonicalAsset>,
    consumers: Vec<PlatformId>,
    scope: InventoryScope,
) -> MigrationInventoryEntry {
    let (ownership_state, owned, selectable) = ownership_fields(&classification);
    MigrationInventoryEntry {
        name,
        path,
        kind,
        classification,
        provenance,
        reason_code,
        content_digest,
        currently_consumed,
        blocking,
        ownership_state,
        owned,
        selectable,
        followed,
        source_layer: source.map(|asset| asset.layer),
        canonical_path: source.map(|asset| asset.path.clone()),
        consumers,
        scope,
    }
}

fn external_skills(
    root: &Utf8Path,
    reason_code: &'static str,
) -> Result<Vec<ExternalSkill>, CoreError> {
    direct_lstat_entries(root)?
        .into_iter()
        .map(|entry| {
            let digest = (entry.shape == EntryShape::Directory)
                .then(|| path_content_digest(&entry.path))
                .transpose()?;
            Ok(ExternalSkill {
                path: entry.path,
                digest,
                reason_code,
            })
        })
        .collect()
}

fn append_external_entries(
    external: &[ExternalSkill],
    provenance: InventoryProvenance,
    scope: InventoryScope,
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
            source_layer: None,
            canonical_path: None,
            consumers: Vec::new(),
            scope,
        });
    }
}

fn classify_link(
    path: &Utf8Path,
    canonical: Option<&CanonicalAsset>,
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
    if canonical.is_some_and(|asset| {
        lexical_target == lexical_normalize(&asset.path)
            || lexical_target == lexical_normalize(&asset.resolved_path)
    }) {
        return Ok((
            InventoryClassification::ManagedLink,
            "canonical_symlink".to_owned(),
            canonical.map(|asset| asset.digest.clone()),
            false,
            false,
        ));
    }
    if let Some(external) = external
        .iter()
        .find(|asset| lexical_target == lexical_normalize(&asset.path))
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

fn classify_regular_asset(
    path: &Utf8Path,
    canonical: Option<&CanonicalAsset>,
    accepts_legacy_marker: bool,
    missing_source_reason: &str,
) -> Result<(InventoryClassification, String, Option<String>, bool, bool), CoreError> {
    let digest = path_content_digest(path)?;
    if accepts_legacy_marker && is_regular_file(&path.join(".ai-config-deploy.json"))? {
        return Ok((
            InventoryClassification::LegacyMarkerCandidate,
            "legacy_marker_present".to_owned(),
            Some(digest),
            false,
            false,
        ));
    }
    match canonical {
        Some(asset) if asset.digest == digest => Ok((
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
            missing_source_reason.to_owned(),
            Some(digest),
            true,
            false,
        )),
    }
}

fn direct_lstat_entries(root: &Utf8Path) -> Result<Vec<DirectEntry>, CoreError> {
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
        let Some(name) = path.file_name().filter(|name| !name.is_empty()) else {
            continue;
        };
        let metadata = fs::symlink_metadata(path.as_std_path()).map_err(CoreError::Io)?;
        let file_type = metadata.file_type();
        let shape = if file_type.is_symlink() {
            EntryShape::Symlink
        } else if file_type.is_dir() {
            EntryShape::Directory
        } else if file_type.is_file() {
            EntryShape::RegularFile
        } else {
            EntryShape::Other
        };
        entries.push(DirectEntry {
            name: name.to_owned(),
            path,
            shape,
        });
    }
    entries.sort_by(|left, right| left.name.cmp(&right.name));
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
    let mut variants = BTreeMap::<(u8, InventoryScope, String), BTreeSet<String>>::new();
    for entry in entries.iter() {
        variants
            .entry((
                asset_kind_order(entry.kind),
                entry.scope,
                entry.name.to_lowercase(),
            ))
            .or_default()
            .insert(entry.name.clone());
    }
    for entry in entries {
        if variants
            .get(&(
                asset_kind_order(entry.kind),
                entry.scope,
                entry.name.to_lowercase(),
            ))
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

fn asset_kind_order(kind: AssetKind) -> u8 {
    match kind {
        AssetKind::Skill => 0,
        AssetKind::Rule => 1,
        AssetKind::Mcp => 2,
        AssetKind::Agent => 3,
        AssetKind::Command => 4,
        AssetKind::Prompt => 5,
        AssetKind::Hook => 6,
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
            canonical_layers: vec![CanonicalLayerRoot {
                layer: SourceLayer::Global,
                asset_root: root.join(".ai-config"),
            }],
            deploy_base: root.join("repo"),
            scope: InventoryScope::Project,
        }
    }

    fn canonical_root(request: &InventoryRequest) -> &Utf8Path {
        &request.canonical_layers[0].asset_root
    }

    #[cfg(unix)]
    #[test]
    fn unknown_skill_link_is_unsafe_and_never_reads_its_sentinel_target() {
        let temp = TempDir::new().unwrap();
        let request = request(&temp);
        write(
            &canonical_root(&request).join("skills/demo/SKILL.md"),
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
            &canonical_root(&request).join("skills/demo/SKILL.md"),
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
            &canonical_root(&request).join("skills/Demo/SKILL.md"),
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

    #[test]
    fn project_rule_uses_effective_project_layer_and_keeps_global_raw_source() {
        let temp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let global = root.join("global/.ai-config");
        let project = root.join("project/.ai-config");
        let request = InventoryRequest {
            canonical_layers: vec![
                CanonicalLayerRoot {
                    layer: SourceLayer::Global,
                    asset_root: global.clone(),
                },
                CanonicalLayerRoot {
                    layer: SourceLayer::Project,
                    asset_root: project.clone(),
                },
            ],
            deploy_base: root.join("project"),
            scope: InventoryScope::Project,
        };
        write(&global.join("rules/shared.mdc"), "global rule\n");
        write(&project.join("rules/shared.mdc"), "project rule\n");
        write(
            &request.deploy_base.join(".cursor/rules/shared.mdc"),
            "project rule\n",
        );

        let result = inventory(&request).unwrap();
        let raw = result
            .entries
            .iter()
            .filter(|entry| entry.kind == AssetKind::Rule && entry.name == "shared")
            .collect::<Vec<_>>();
        assert_eq!(
            raw.len(),
            3,
            "both raw canonical layers plus target are retained"
        );
        let target = raw
            .iter()
            .find(|entry| entry.path.ends_with(".cursor/rules/shared.mdc"))
            .unwrap();
        assert_eq!(target.classification, InventoryClassification::Equivalent);
        assert_eq!(target.source_layer, Some(SourceLayer::Project));
        assert_eq!(
            target.canonical_path,
            Some(project.join("rules/shared.mdc"))
        );
        assert_eq!(
            target.consumers,
            vec![PlatformId::Cursor, PlatformId::Hermes]
        );
    }

    #[test]
    fn skill_and_rule_case_variants_do_not_cross_kind_collide() {
        let temp = TempDir::new().unwrap();
        let request = request(&temp);
        write(
            &canonical_root(&request).join("skills/Demo/SKILL.md"),
            "skill\n",
        );
        write(&canonical_root(&request).join("rules/demo.mdc"), "rule\n");

        let result = inventory(&request).unwrap();
        assert!(result
            .entries
            .iter()
            .all(|entry| { entry.classification == InventoryClassification::CanonicalSource }));
    }
}
