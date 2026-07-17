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
use crate::projection::mcp::entry_fingerprint::{
    inspect_claude_mcp_entries, inspect_codex_mcp_entries, inspect_cursor_mcp_entries,
    inspect_hermes_mcp_entries, McpEntryFingerprint,
};
use crate::projection::mcp::source::load_mcp_definition_at;
use crate::projection::model::SourceLayer;
use crate::projection::platform_adapter::TrustRequirement;

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
    CanonicalLegacy,
    PlatformCurrent,
    PlatformLegacy,
    CcSwitch,
    PluginBuiltin,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InventoryClassification {
    CanonicalSource,
    LegacyMcpCandidate,
    LegacyAgentCandidate,
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
    /// Named entry within a generated container; absent for direct assets and sources.
    pub entry_key: Option<String>,
    /// Source or target syntax/shape.  It never contains rendered configuration bytes or bodies.
    pub format: Option<String>,
    /// Canonical MCP references only.  Literal platform values are deliberately never exposed.
    pub secret_keys: Vec<String>,
    /// Platform-specific trust prerequisite for applying a generated MCP entry.
    pub trust_requirement: TrustRequirement,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationInventoryUnsupported {
    pub kind: AssetKind,
    pub platform: PlatformId,
    pub name: String,
    pub source_layer: Option<SourceLayer>,
    pub canonical_path: Option<Utf8PathBuf>,
    pub scope: InventoryScope,
    pub reason_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationInventoryIssue {
    pub kind: AssetKind,
    pub path: Utf8PathBuf,
    pub scope: InventoryScope,
    pub reason_code: String,
    pub blocking: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationInventory {
    pub schema_version: u16,
    pub scope: InventoryScope,
    pub plan_digest: String,
    pub entries: Vec<MigrationInventoryEntry>,
    pub unsupported: Vec<MigrationInventoryUnsupported>,
    pub issues: Vec<MigrationInventoryIssue>,
}

#[derive(Debug, Clone)]
struct CanonicalAsset {
    name: String,
    path: Utf8PathBuf,
    resolved_path: Utf8PathBuf,
    digest: String,
    layer: SourceLayer,
    kind: AssetKind,
    secret_keys: Vec<String>,
    targets: Option<Vec<PlatformId>>,
    hook_binding_digest: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CanonicalAgentSchemaError {
    Invalid,
    NameMismatch,
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
    let mut issues = Vec::new();
    let mut legacy_canonical_agents = Vec::new();
    let mut canonical_hook_findings = Vec::new();
    for root in layers {
        for asset in canonical_assets(
            root,
            request.scope,
            &mut issues,
            &mut legacy_canonical_agents,
            &mut canonical_hook_findings,
        )? {
            effective.insert(
                (asset_kind_order(asset.kind), asset.name.clone()),
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
    entries.extend(legacy_canonical_agents);
    let split_cross_layer_hooks = canonical_hook_findings
        .iter()
        .filter(|finding| {
            canonical_hook_findings.iter().any(|other| {
                finding.name == other.name
                    && finding.source_layer != other.source_layer
                    && matches!(
                        (finding.reason_code.as_str(), other.reason_code.as_str(),),
                        (
                            "canonical_hook_binding_without_script",
                            "canonical_hook_script_without_binding",
                        ) | (
                            "canonical_hook_script_without_binding",
                            "canonical_hook_binding_without_script",
                        )
                    )
            })
        })
        .map(|finding| finding.name.clone())
        .collect::<BTreeSet<_>>();
    for finding in &mut canonical_hook_findings {
        if matches!(
            finding.reason_code.as_str(),
            "canonical_hook_binding_without_script" | "canonical_hook_script_without_binding"
        ) {
            let key = (asset_kind_order(AssetKind::Hook), finding.name.clone());
            if split_cross_layer_hooks.contains(&finding.name)
                || effective.get(&key).is_some_and(|asset| {
                    finding
                        .source_layer
                        .is_some_and(|layer| layer > asset.layer)
                })
            {
                finding.reason_code = "cross_layer_hook_pair".to_owned();
                effective.remove(&key);
            }
        }
    }
    entries.extend(canonical_hook_findings);
    let mut unsupported = Vec::new();

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
    scan_mcp_targets(
        request,
        &effective,
        &mut entries,
        &mut unsupported,
        &mut issues,
    )?;
    scan_agent_targets(
        request,
        &effective,
        &mut entries,
        &mut unsupported,
        &mut issues,
    )?;
    scan_command_targets(
        request,
        &effective,
        &mut entries,
        &mut unsupported,
        &mut issues,
    )?;
    scan_hook_targets(
        request,
        &effective,
        &mut entries,
        &mut unsupported,
        &mut issues,
    )?;
    scan_legacy_canonical_mcp(
        &request.canonical_layers,
        request.scope,
        &mut entries,
        &mut issues,
    )?;
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
    unsupported.sort_by(|left, right| {
        (
            asset_kind_order(left.kind),
            platform_order(left.platform),
            left.name.as_str(),
            left.canonical_path.as_ref().map(|path| path.as_str()),
        )
            .cmp(&(
                asset_kind_order(right.kind),
                platform_order(right.platform),
                right.name.as_str(),
                right.canonical_path.as_ref().map(|path| path.as_str()),
            ))
    });
    issues.sort_by(|left, right| {
        (
            asset_kind_order(left.kind),
            left.path.as_str(),
            &left.scope,
            left.reason_code.as_str(),
        )
            .cmp(&(
                asset_kind_order(right.kind),
                right.path.as_str(),
                &right.scope,
                right.reason_code.as_str(),
            ))
    });
    let encoded =
        serde_json::to_vec(&(&entries, &unsupported, &issues)).map_err(CoreError::Json)?;
    Ok(MigrationInventory {
        schema_version: MIGRATION_INVENTORY_SCHEMA_VERSION,
        scope: request.scope,
        plan_digest: hex::encode(Sha256::digest(encoded)),
        entries,
        unsupported,
        issues,
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

fn canonical_assets(
    root: &CanonicalLayerRoot,
    scope: InventoryScope,
    issues: &mut Vec<MigrationInventoryIssue>,
    legacy_agents: &mut Vec<MigrationInventoryEntry>,
    canonical_hook_findings: &mut Vec<MigrationInventoryEntry>,
) -> Result<Vec<CanonicalAsset>, CoreError> {
    let mut assets = Vec::new();
    for entry in direct_lstat_entries(&root.asset_root.join("skills"))? {
        if entry.shape != EntryShape::Directory || !is_regular_file(&entry.path.join("SKILL.md"))? {
            continue;
        }
        assets.push(canonical_asset(
            root.layer,
            AssetKind::Skill,
            entry.name,
            entry.path.clone(),
        )?);
    }
    for entry in direct_lstat_entries(&root.asset_root.join("rules"))? {
        if entry.shape != EntryShape::RegularFile || entry.path.extension() != Some("mdc") {
            continue;
        }
        let name = entry
            .path
            .file_stem()
            .filter(|name| !name.is_empty())
            .unwrap_or(&entry.name)
            .to_owned();
        assets.push(canonical_asset(
            root.layer,
            AssetKind::Rule,
            name,
            entry.path,
        )?);
    }
    scan_canonical_agents(root, scope, issues, legacy_agents, &mut assets)?;
    scan_canonical_commands(root, scope, issues, &mut assets)?;
    scan_canonical_hooks(root, scope, issues, canonical_hook_findings, &mut assets)?;
    let mcp_servers = root.asset_root.join("mcp/servers");
    if let Some(parent_issue) = mcp_parent_issue(&root.asset_root, &mcp_servers) {
        let reason_code = match parent_issue {
            "unsafe_mcp_container_parent_symlink" => "unsafe_canonical_mcp_parent_symlink",
            "unsafe_mcp_container_parent_non_directory" => {
                "unsafe_canonical_mcp_parent_non_directory"
            }
            _ => "unreadable_canonical_mcp_parent",
        };
        push_mcp_container_issue(issues, &mcp_servers, scope, reason_code);
        return Ok(assets);
    }
    let mcp_entries = match direct_lstat_entries(&mcp_servers) {
        Ok(entries) => entries,
        Err(_) => {
            push_mcp_container_issue(issues, &mcp_servers, scope, "unreadable_canonical_mcp");
            return Ok(assets);
        }
    };
    for entry in mcp_entries {
        if entry.shape == EntryShape::Symlink {
            push_mcp_container_issue(issues, &entry.path, scope, "unsafe_canonical_mcp_symlink");
            continue;
        }
        if entry.shape != EntryShape::RegularFile || entry.path.extension() != Some("json") {
            continue;
        }
        let definition = match load_mcp_definition_at(&entry.path) {
            Ok(definition) => definition,
            Err(_) => {
                push_mcp_container_issue(issues, &entry.path, scope, "invalid_canonical_mcp");
                continue;
            }
        };
        if entry.path.file_stem() != Some(definition.server.name.as_str()) {
            push_mcp_container_issue(
                issues,
                &entry.path,
                scope,
                "canonical_mcp_filename_name_mismatch",
            );
            continue;
        }
        let mut asset = match canonical_asset(
            root.layer,
            AssetKind::Mcp,
            definition.server.name,
            entry.path.clone(),
        ) {
            Ok(asset) => asset,
            Err(_) => {
                push_mcp_container_issue(issues, &entry.path, scope, "unreadable_canonical_mcp");
                continue;
            }
        };
        asset.secret_keys = definition.server.secret_keys;
        asset.targets = Some(definition.targets);
        assets.push(asset);
    }
    Ok(assets)
}

fn scan_canonical_commands(
    root: &CanonicalLayerRoot,
    scope: InventoryScope,
    issues: &mut Vec<MigrationInventoryIssue>,
    assets: &mut Vec<CanonicalAsset>,
) -> Result<(), CoreError> {
    let root_metadata = match fs::symlink_metadata(root.asset_root.as_std_path()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {
            push_inventory_issue(
                issues,
                AssetKind::Command,
                &root.asset_root,
                scope,
                "unreadable_canonical_command_root",
            );
            return Ok(());
        }
    };
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        push_inventory_issue(
            issues,
            AssetKind::Command,
            &root.asset_root,
            scope,
            "unsafe_canonical_command_root",
        );
        return Ok(());
    }

    let commands = root.asset_root.join("commands");
    let metadata = match fs::symlink_metadata(commands.as_std_path()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {
            push_inventory_issue(
                issues,
                AssetKind::Command,
                &commands,
                scope,
                "unreadable_canonical_command_directory",
            );
            return Ok(());
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        push_inventory_issue(
            issues,
            AssetKind::Command,
            &commands,
            scope,
            "unsafe_canonical_command_directory",
        );
        return Ok(());
    }

    let entries = match direct_lstat_entries(&commands) {
        Ok(entries) => entries,
        Err(_) => {
            push_inventory_issue(
                issues,
                AssetKind::Command,
                &commands,
                scope,
                "unreadable_canonical_command_directory",
            );
            return Ok(());
        }
    };
    for entry in entries {
        if is_excluded_agent_entry(&entry.name) || entry.path.extension() != Some("md") {
            continue;
        }
        if entry.shape == EntryShape::Symlink {
            push_inventory_issue(
                issues,
                AssetKind::Command,
                &entry.path,
                scope,
                "unsafe_canonical_command_file_symlink",
            );
            continue;
        }
        if entry.shape != EntryShape::RegularFile {
            push_inventory_issue(
                issues,
                AssetKind::Command,
                &entry.path,
                scope,
                "unsafe_canonical_command_file_non_regular",
            );
            continue;
        }
        let Some(name) = entry
            .path
            .file_stem()
            .filter(|name| is_safe_asset_name(name))
        else {
            push_inventory_issue(
                issues,
                AssetKind::Command,
                &entry.path,
                scope,
                "invalid_canonical_command_name",
            );
            continue;
        };
        if fs::read_to_string(entry.path.as_std_path()).is_err() {
            push_inventory_issue(
                issues,
                AssetKind::Command,
                &entry.path,
                scope,
                "unreadable_canonical_command",
            );
            continue;
        }
        match canonical_asset(
            root.layer,
            AssetKind::Command,
            name.to_owned(),
            entry.path.clone(),
        ) {
            Ok(asset) => assets.push(asset),
            Err(_) => push_inventory_issue(
                issues,
                AssetKind::Command,
                &entry.path,
                scope,
                "unreadable_canonical_command",
            ),
        }
    }
    Ok(())
}

fn scan_canonical_hooks(
    root: &CanonicalLayerRoot,
    scope: InventoryScope,
    issues: &mut Vec<MigrationInventoryIssue>,
    findings: &mut Vec<MigrationInventoryEntry>,
    assets: &mut Vec<CanonicalAsset>,
) -> Result<(), CoreError> {
    let root_metadata = match fs::symlink_metadata(root.asset_root.as_std_path()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {
            push_inventory_issue(
                issues,
                AssetKind::Hook,
                &root.asset_root,
                scope,
                "unreadable_canonical_hook_root",
            );
            return Ok(());
        }
    };
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        push_inventory_issue(
            issues,
            AssetKind::Hook,
            &root.asset_root,
            scope,
            "unsafe_canonical_hook_root",
        );
        return Ok(());
    }

    let manifest = root.asset_root.join("hooks.json");
    let manifest_bindings = canonical_hook_binding_names(root, &manifest, scope, issues, findings)?;
    let hooks = root.asset_root.join("hooks");
    let hook_units = canonical_hook_units(&hooks, scope, issues)?;
    let names = manifest_bindings
        .keys()
        .chain(hook_units.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    for name in names {
        match (manifest_bindings.get(&name), hook_units.get(&name)) {
            (Some(binding_digest), Some(path)) => {
                match canonical_asset(root.layer, AssetKind::Hook, name, path.clone()) {
                    Ok(mut asset) => {
                        asset.hook_binding_digest = Some(binding_digest.clone());
                        assets.push(asset);
                    }
                    Err(_) => push_inventory_issue(
                        issues,
                        AssetKind::Hook,
                        path,
                        scope,
                        "unreadable_canonical_hook_unit",
                    ),
                }
            }
            (Some(_), None) => findings.push(canonical_hook_finding(
                root,
                scope,
                name,
                manifest.clone(),
                "canonical_hook_binding_without_script",
                "json",
            )),
            (None, Some(path)) => findings.push(canonical_hook_finding(
                root,
                scope,
                name,
                path.clone(),
                "canonical_hook_script_without_binding",
                hook_unit_format(path),
            )),
            (None, None) => unreachable!("name came from binding or unit"),
        }
    }
    Ok(())
}

fn canonical_hook_binding_names(
    root: &CanonicalLayerRoot,
    manifest: &Utf8Path,
    scope: InventoryScope,
    issues: &mut Vec<MigrationInventoryIssue>,
    findings: &mut Vec<MigrationInventoryEntry>,
) -> Result<BTreeMap<String, String>, CoreError> {
    let metadata = match fs::symlink_metadata(manifest.as_std_path()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(_) => {
            push_inventory_issue(
                issues,
                AssetKind::Hook,
                manifest,
                scope,
                "unreadable_canonical_hook_manifest",
            );
            return Ok(BTreeMap::new());
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        push_inventory_issue(
            issues,
            AssetKind::Hook,
            manifest,
            scope,
            "unsafe_canonical_hook_manifest",
        );
        return Ok(BTreeMap::new());
    }
    let raw = match fs::read_to_string(manifest.as_std_path()) {
        Ok(raw) => raw,
        Err(_) => {
            push_inventory_issue(
                issues,
                AssetKind::Hook,
                manifest,
                scope,
                "unreadable_canonical_hook_manifest",
            );
            return Ok(BTreeMap::new());
        }
    };
    let document = match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(document) => document,
        Err(_) => {
            push_inventory_issue(
                issues,
                AssetKind::Hook,
                manifest,
                scope,
                "invalid_canonical_hook_manifest",
            );
            return Ok(BTreeMap::new());
        }
    };
    let Some(hooks) = document.get("hooks").and_then(serde_json::Value::as_object) else {
        push_inventory_issue(
            issues,
            AssetKind::Hook,
            manifest,
            scope,
            "invalid_canonical_hook_manifest",
        );
        return Ok(BTreeMap::new());
    };
    let mut bindings = BTreeMap::<String, Vec<Vec<u8>>>::new();
    for (lifecycle, entries) in hooks {
        let Some(entries) = entries.as_array() else {
            push_inventory_issue(
                issues,
                AssetKind::Hook,
                manifest,
                scope,
                "invalid_canonical_hook_binding",
            );
            continue;
        };
        for entry in entries {
            let command = entry.get("command").and_then(serde_json::Value::as_str);
            let Some(command) = command else {
                push_inventory_issue(
                    issues,
                    AssetKind::Hook,
                    manifest,
                    scope,
                    if entry.get("prompt").is_some()
                        || entry.get("type").is_some_and(|value| value == "prompt")
                    {
                        "unsupported_canonical_hook_prompt"
                    } else {
                        "invalid_canonical_hook_binding"
                    },
                );
                continue;
            };
            let name =
                hook_name_from_any_command(command).unwrap_or_else(|| "invalid-hook".to_owned());
            if Utf8Path::new(command.split_whitespace().next().unwrap_or_default()).is_absolute() {
                findings.push(canonical_hook_finding(
                    root,
                    scope,
                    name,
                    manifest.to_path_buf(),
                    "canonical_hook_command_absolute",
                    "json",
                ));
                continue;
            }
            if command.split_whitespace().next().is_some_and(|path| {
                Utf8Path::new(path)
                    .components()
                    .any(|component| component.as_str() == "..")
            }) {
                findings.push(canonical_hook_finding(
                    root,
                    scope,
                    name,
                    manifest.to_path_buf(),
                    "canonical_hook_command_parent_escape",
                    "json",
                ));
                continue;
            }
            let Some(name) = canonical_hook_name_from_command(command) else {
                push_inventory_issue(
                    issues,
                    AssetKind::Hook,
                    manifest,
                    scope,
                    "invalid_canonical_hook_binding",
                );
                continue;
            };
            if !is_supported_canonical_hook_lifecycle(lifecycle) {
                push_inventory_issue(
                    issues,
                    AssetKind::Hook,
                    manifest,
                    scope,
                    "unsupported_canonical_hook_lifecycle",
                );
                continue;
            }
            let executable = command.split_whitespace().next().unwrap_or_default();
            let relative = executable.strip_prefix("./").unwrap_or(executable);
            let target = root.asset_root.join(relative);
            let target_is_safe_file = matches!(
                fs::symlink_metadata(target.as_std_path()),
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink()
            );
            if !target_is_safe_file {
                let unit = root.asset_root.join("hooks").join(&name);
                match fs::symlink_metadata(unit.as_std_path()) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        // Preserve a binding-only component so overlay resolution can
                        // distinguish a forbidden cross-layer pair from an ordinary
                        // missing command leaf.
                    }
                    Ok(metadata) => {
                        let reason = if metadata.is_file() && target != unit {
                            "canonical_hook_command_unit_shape_mismatch"
                        } else {
                            "canonical_hook_command_missing_entry"
                        };
                        push_inventory_issue(issues, AssetKind::Hook, manifest, scope, reason);
                        continue;
                    }
                    Err(_) => {
                        push_inventory_issue(
                            issues,
                            AssetKind::Hook,
                            manifest,
                            scope,
                            "canonical_hook_command_missing_entry",
                        );
                        continue;
                    }
                }
            }
            let encoded = serde_json::to_vec(&serde_json::json!({
                "lifecycle": lifecycle,
                "binding": entry,
            }))
            .map_err(CoreError::Json)?;
            bindings.entry(name).or_default().push(encoded);
        }
    }
    Ok(bindings
        .into_iter()
        .map(|(name, mut encoded)| {
            encoded.sort();
            let mut hasher = Sha256::new();
            for value in encoded {
                hasher.update(value);
                hasher.update([0]);
            }
            (name, hex::encode(hasher.finalize()))
        })
        .collect())
}

fn is_supported_canonical_hook_lifecycle(lifecycle: &str) -> bool {
    crate::hook_lifecycle::CURSOR_LIFECYCLES
        .iter()
        .any(|definition| definition.id == lifecycle)
}

fn canonical_hook_finding(
    root: &CanonicalLayerRoot,
    scope: InventoryScope,
    name: String,
    path: Utf8PathBuf,
    reason_code: &str,
    format: &str,
) -> MigrationInventoryEntry {
    let mut entry = target_entry(
        name.clone(),
        path.clone(),
        AssetKind::Hook,
        InventoryClassification::Foreign,
        InventoryProvenance::Canonical,
        reason_code.to_owned(),
        None,
        false,
        true,
        false,
        None,
        Vec::new(),
        scope,
    );
    entry.source_layer = Some(root.layer);
    entry.canonical_path = Some(path);
    entry.entry_key = Some(format!("hooks.{name}"));
    entry.format = Some(format.to_owned());
    entry
}

fn canonical_hook_name_from_command(command: &str) -> Option<String> {
    let executable = command.split_whitespace().next()?;
    let relative = executable.strip_prefix("./hooks/")?;
    let path = Utf8Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component.as_str(), "" | "." | ".."))
    {
        return None;
    }
    let name = path.components().next()?.as_str();
    is_safe_asset_name(name).then(|| name.to_owned())
}

fn hook_name_from_any_command(command: &str) -> Option<String> {
    let executable = command.split_whitespace().next()?;
    let path = Utf8Path::new(executable);
    let components = path
        .components()
        .map(|component| component.as_str())
        .collect::<Vec<_>>();
    if let Some(index) = components
        .iter()
        .position(|component| *component == "hooks")
    {
        if let Some(name) = components
            .get(index + 1)
            .filter(|name| is_safe_asset_name(name))
        {
            return Some((*name).to_owned());
        }
    }
    path.file_name()
        .filter(|name| is_safe_asset_name(name))
        .map(str::to_owned)
}

fn hook_unit_format(path: &Utf8Path) -> &'static str {
    match fs::symlink_metadata(path.as_std_path()) {
        Ok(metadata) if metadata.is_dir() => "directory",
        Ok(metadata) if metadata.is_file() => "file",
        _ => "unknown",
    }
}

#[cfg(test)]
mod hook_inventory_p1_red_tests {
    use super::*;
    use tempfile::TempDir;

    fn write(path: &Utf8Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap().as_std_path()).unwrap();
        fs::write(path.as_std_path(), contents).unwrap();
    }

    fn global_request(home: &Utf8Path, asset_root: &Utf8Path) -> InventoryRequest {
        InventoryRequest {
            canonical_layers: vec![CanonicalLayerRoot {
                layer: SourceLayer::Global,
                asset_root: asset_root.to_path_buf(),
            }],
            deploy_base: home.to_path_buf(),
            scope: InventoryScope::Global,
        }
    }

    #[test]
    fn cursor_catalog_lifecycles_are_valid_canonical_hook_bindings() {
        let temp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(temp.path()).unwrap();
        let asset_root = home.join(".ai-config");
        let request = global_request(home, &asset_root);
        let lifecycle_names = [
            "beforeReadFile",
            "afterAgentResponse",
            "afterAgentThought",
            "beforeTabFileRead",
            "workspaceOpen",
        ];
        let catalog_names = crate::hook_lifecycle::CURSOR_LIFECYCLES
            .iter()
            .map(|lifecycle| lifecycle.id)
            .collect::<BTreeSet<_>>();
        assert!(
            lifecycle_names
                .iter()
                .all(|lifecycle| catalog_names.contains(lifecycle)),
            "test contract must stay aligned with the canonical Cursor lifecycle catalog"
        );

        let mut hooks = serde_json::Map::new();
        for lifecycle in lifecycle_names {
            let name = format!("{lifecycle}.sh");
            write(&asset_root.join("hooks").join(&name), "#!/bin/sh\n");
            hooks.insert(
                lifecycle.to_owned(),
                serde_json::json!([{
                    "command": format!("./hooks/{name}"),
                }]),
            );
        }
        write(
            &asset_root.join("hooks.json"),
            &serde_json::to_string(&serde_json::json!({
                "version": 1,
                "hooks": hooks,
            }))
            .unwrap(),
        );

        let report = inventory(&request).unwrap();
        let accepted = report
            .entries
            .iter()
            .filter(|entry| {
                entry.kind == AssetKind::Hook
                    && entry.classification == InventoryClassification::CanonicalSource
            })
            .map(|entry| entry.name.as_str())
            .collect::<BTreeSet<_>>();
        let expected = lifecycle_names
            .iter()
            .map(|lifecycle| format!("{lifecycle}.sh"))
            .collect::<BTreeSet<_>>();

        assert_eq!(
            accepted,
            expected.iter().map(String::as_str).collect(),
            "every lifecycle in the canonical Cursor catalog must remain inventoryable"
        );
        assert!(
            report.issues.iter().all(|issue| {
                issue.kind != AssetKind::Hook
                    || issue.reason_code != "unsupported_canonical_hook_lifecycle"
            }),
            "catalog lifecycle was incorrectly rejected: {:?}",
            report.issues
        );
    }

    #[test]
    fn malformed_native_hook_container_shapes_are_blocking() {
        let temp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(temp.path()).unwrap();
        let asset_root = home.join(".ai-config");
        let request = global_request(home, &asset_root);
        let scalar = home.join(".cursor/hooks.json");
        let array = home.join(".codex/hooks.json");
        write(&scalar, r#"{"hooks":"not-an-event-map"}"#);
        write(&array, r#"{"hooks":[{"command":".codex/hooks/run.sh"}]}"#);
        write(&home.join(".codex/hooks/run.sh"), "#!/bin/sh\n");

        let report = inventory(&request).unwrap();
        let missing = [&scalar, &array]
            .into_iter()
            .filter(|path| {
                !report.issues.iter().any(|issue| {
                    issue.kind == AssetKind::Hook && issue.path == **path && issue.blocking
                })
            })
            .collect::<Vec<_>>();
        assert!(
            missing.is_empty(),
            "malformed native Hook containers must fail closed: missing={missing:?}; issues={:?}",
            report.issues
        );
    }

    #[test]
    fn malformed_native_hook_document_roots_are_blocking() {
        let temp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(temp.path()).unwrap();
        let asset_root = home.join(".ai-config");
        let request = global_request(home, &asset_root);
        let cursor = home.join(".cursor/hooks.json");
        let hermes = home.join(".hermes/config.yaml");
        write(&cursor, "[]");
        write(&hermes, "scalar-root\n");

        let report = inventory(&request).unwrap();
        let missing = [&cursor, &hermes]
            .into_iter()
            .filter(|path| {
                !report.issues.iter().any(|issue| {
                    issue.kind == AssetKind::Hook
                        && issue.path == **path
                        && issue.reason_code == "invalid_hook_container"
                        && issue.blocking
                })
            })
            .collect::<Vec<_>>();
        assert!(
            missing.is_empty(),
            "non-object native Hook document roots must fail closed: \
             missing={missing:?}; issues={:?}",
            report.issues
        );
    }

    #[test]
    fn platform_hook_binding_cannot_pair_with_a_different_script_root() {
        let temp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(temp.path()).unwrap();
        let asset_root = home.join(".ai-config");
        let request = global_request(home, &asset_root);
        let container = home.join(".cursor/hooks.json");
        write(
            &container,
            r#"{"hooks":{"afterShellExecution":[{"command":"/tmp/hooks/run.sh","hook":"run.sh"}]}}"#,
        );
        write(&home.join(".cursor/hooks/run.sh"), "#!/bin/sh\n");

        let report = inventory(&request).unwrap();
        let unsafe_binding_blocked = report.issues.iter().any(|issue| {
            issue.kind == AssetKind::Hook
                && issue.path == container
                && issue.blocking
                && (issue.reason_code.contains("mismatch") || issue.reason_code.contains("invalid"))
        });
        assert!(
            unsafe_binding_blocked,
            "a binding outside the platform Hook root must not pair with the local script: \
             entries={:?}; issues={:?}",
            report.entries, report.issues
        );
    }

    #[test]
    fn inventory_recognizes_current_adapter_hook_command_paths() {
        let global_temp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(global_temp.path()).unwrap();
        let global_assets = home.join(".ai-config");
        let global_request = global_request(home, &global_assets);
        let cursor_container = home.join(".cursor/hooks.json");
        let codex_container = home.join(".codex/hooks.json");
        let claude_container = home.join(".claude/settings.json");
        write(&home.join(".cursor/hooks/cursor.sh"), "#!/bin/sh\n");
        write(&home.join(".codex/hooks/codex.sh"), "#!/bin/sh\n");
        write(&home.join(".claude/hooks/claude.sh"), "#!/bin/sh\n");
        write(
            &cursor_container,
            r#"{"hooks":{"afterShellExecution":[{"command":"./hooks/cursor.sh","hook":"cursor.sh"}]}}"#,
        );
        write(
            &codex_container,
            &serde_json::to_string(&serde_json::json!({
                "hooks": {
                    "PostToolUse": [{
                        "hooks": [{
                            "command": home.join(".codex/hooks/codex.sh").as_str(),
                            "hook": "codex.sh",
                        }],
                    }],
                },
            }))
            .unwrap(),
        );
        write(
            &claude_container,
            &serde_json::to_string(&serde_json::json!({
                "hooks": {
                    "PostToolUse": [{
                        "hooks": [{
                            "command": home.join(".claude/hooks/claude.sh").as_str(),
                            "hook": "claude.sh",
                        }],
                    }],
                },
            }))
            .unwrap(),
        );

        let global_report = inventory(&global_request).unwrap();
        let missing_global = [
            (PlatformId::Cursor, "cursor.sh", &cursor_container),
            (PlatformId::Codex, "codex.sh", &codex_container),
            (PlatformId::Claude, "claude.sh", &claude_container),
        ]
        .into_iter()
        .filter(|(platform, name, container)| {
            !global_report.entries.iter().any(|entry| {
                entry.kind == AssetKind::Hook
                    && entry.name == *name
                    && entry.path == **container
                    && entry.consumers == vec![*platform]
            })
        })
        .map(|(platform, name, _)| (platform, name))
        .collect::<Vec<_>>();
        let global_path_issue = global_report.issues.iter().any(|issue| {
            issue.kind == AssetKind::Hook
                && matches!(
                    issue.reason_code.as_str(),
                    "hook_binding_script_name_mismatch" | "hook_half_projection"
                )
        });

        let project_temp = TempDir::new().unwrap();
        let project = Utf8Path::from_path(project_temp.path()).unwrap();
        let project_assets = project.join(".ai-config");
        let project_request = InventoryRequest {
            canonical_layers: vec![CanonicalLayerRoot {
                layer: SourceLayer::Project,
                asset_root: project_assets,
            }],
            deploy_base: project.to_path_buf(),
            scope: InventoryScope::Project,
        };
        let project_cursor_container = project.join(".cursor/hooks.json");
        let project_claude_container = project.join(".claude/settings.json");
        write(&project.join(".cursor/hooks/shared.sh"), "#!/bin/sh\n");
        write(
            &project_cursor_container,
            r#"{"hooks":{"afterShellExecution":[{"command":".cursor/hooks/shared.sh","hook":"shared.sh"}]}}"#,
        );
        write(
            &project_claude_container,
            r#"{"hooks":{"PostToolUse":[{"hooks":[{"command":".cursor/hooks/shared.sh","hook":"shared.sh"}]}]}}"#,
        );

        let project_report = inventory(&project_request).unwrap();
        let project_binding_recognized = project_report.entries.iter().any(|entry| {
            entry.kind == AssetKind::Hook
                && entry.name == "shared.sh"
                && entry.path == project_claude_container
                && entry.consumers == vec![PlatformId::Claude]
        });
        let project_path_issue = project_report.issues.iter().any(|issue| {
            issue.kind == AssetKind::Hook
                && matches!(
                    issue.reason_code.as_str(),
                    "hook_binding_script_name_mismatch" | "hook_half_projection"
                )
        });

        assert!(
            missing_global.is_empty()
                && !global_path_issue
                && project_binding_recognized
                && !project_path_issue,
            "current adapter Hook paths must remain recognizable and complete: \
             missing_global={missing_global:?} global_issues={:?} \
             project_binding_recognized={project_binding_recognized} project_issues={:?} \
             project_entries={:?}",
            global_report.issues,
            project_report.issues,
            project_report.entries
        );
    }

    #[test]
    fn claude_only_project_binding_can_use_the_shared_cursor_hook_script_root() {
        let temp = TempDir::new().unwrap();
        let project = Utf8Path::from_path(temp.path()).unwrap();
        let project_assets = project.join(".ai-config");
        let request = InventoryRequest {
            canonical_layers: vec![CanonicalLayerRoot {
                layer: SourceLayer::Project,
                asset_root: project_assets,
            }],
            deploy_base: project.to_path_buf(),
            scope: InventoryScope::Project,
        };
        let shared_script = project.join(".cursor/hooks/shared.sh");
        write(&shared_script, "#!/bin/sh\n");
        write(
            &project.join(".claude/settings.json"),
            r#"{"hooks":{"PostToolUse":[{"hooks":[{"command":".cursor/hooks/shared.sh","hook":"shared.sh"}]}]}}"#,
        );

        let report = inventory(&request).unwrap();
        let script = report
            .entries
            .iter()
            .find(|entry| entry.kind == AssetKind::Hook && entry.path == shared_script)
            .expect("shared Cursor-path Hook script inventory row");
        assert!(
            script.consumers.contains(&PlatformId::Claude),
            "shared script must disclose Claude consumption: {script:?}"
        );
        assert!(
            report.issues.iter().all(|issue| {
                issue.kind != AssetKind::Hook
                    || issue.reason_code != "hook_half_projection"
                    || issue.path != shared_script
            }),
            "a complete Claude-only shared Hook must not retain Cursor's early half issue: {:?}",
            report.issues
        );
    }

    #[test]
    fn unknown_nested_command_does_not_forge_a_platform_hook_binding() {
        let temp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(temp.path()).unwrap();
        let asset_root = home.join(".ai-config");
        let request = global_request(home, &asset_root);
        let container = home.join(".cursor/hooks.json");
        write(
            &container,
            r#"{"hooks":{"foreignExtension":{"nested":{"command":".cursor/hooks/ghost.sh"}}}}"#,
        );

        let report = inventory(&request).unwrap();
        assert!(
            report.entries.iter().all(|entry| {
                entry.kind != AssetKind::Hook || entry.path != container || entry.name != "ghost.sh"
            }),
            "unknown nested commands must not be interpreted as native Hook bindings: {:?}",
            report.entries
        );
        assert!(
            report.issues.iter().all(|issue| {
                issue.kind != AssetKind::Hook
                    || issue.path != container
                    || issue.reason_code != "hook_half_projection"
            }),
            "a forged binding must not create a half-projection conflict: {:?}",
            report.issues
        );
    }

    #[test]
    fn platform_hook_binding_digest_is_isolated_from_siblings_and_unknown_fields() {
        let temp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(temp.path()).unwrap();
        let asset_root = home.join(".ai-config");
        let request = global_request(home, &asset_root);
        let container = home.join(".cursor/hooks.json");
        for name in ["alpha.sh", "beta.sh"] {
            write(&home.join(".cursor/hooks").join(name), "#!/bin/sh\n");
        }
        let write_container = |beta_matcher: &str, foreign_revision: u64| {
            write(
                &container,
                &serde_json::to_string(&serde_json::json!({
                    "hooks": {
                        "afterShellExecution": [
                            {
                                "command": ".cursor/hooks/alpha.sh",
                                "matcher": "alpha",
                            },
                            {
                                "command": ".cursor/hooks/beta.sh",
                                "matcher": beta_matcher,
                            },
                        ],
                        "foreignExtension": {
                            "revision": foreign_revision,
                        },
                    },
                }))
                .unwrap(),
            );
        };
        let alpha_digest = |report: &MigrationInventory| {
            report
                .entries
                .iter()
                .find(|entry| {
                    entry.kind == AssetKind::Hook
                        && entry.path == container
                        && entry.name == "alpha.sh"
                })
                .and_then(|entry| entry.content_digest.clone())
                .expect("alpha binding digest")
        };

        write_container("beta-v1", 1);
        let baseline = inventory(&request).unwrap();
        write_container("beta-v2", 1);
        let sibling_changed = inventory(&request).unwrap();
        write_container("beta-v2", 2);
        let unknown_changed = inventory(&request).unwrap();

        assert_eq!(
            alpha_digest(&baseline),
            alpha_digest(&sibling_changed),
            "changing beta must not drift alpha's named binding digest"
        );
        assert_eq!(
            alpha_digest(&sibling_changed),
            alpha_digest(&unknown_changed),
            "changing an unknown foreign field must not drift alpha's named binding digest"
        );
    }

    #[test]
    fn hook_inventory_fingerprints_canonical_bindings_and_nested_platform_matchers() {
        let temp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(temp.path()).unwrap();
        let asset_root = home.join(".ai-config");
        let source = asset_root.join("hooks/run.sh");
        let request = InventoryRequest {
            canonical_layers: vec![CanonicalLayerRoot {
                layer: SourceLayer::Global,
                asset_root: asset_root.clone(),
            }],
            deploy_base: home.to_path_buf(),
            scope: InventoryScope::Global,
        };
        write(&source, "#!/bin/sh\nexit 0\n");

        let write_manifest = |lifecycle: &str, matcher: &str, argument: &str| {
            write(
                &asset_root.join("hooks.json"),
                &serde_json::to_string(&serde_json::json!({
                    "version": 1,
                    "hooks": {
                        lifecycle: [{
                            "command": format!("./hooks/run.sh {argument}"),
                            "matcher": matcher,
                        }],
                    },
                }))
                .unwrap(),
            );
        };
        let write_nested_platform_binding = |path: &Utf8Path, script_root: &str, matcher: &str| {
            write(
                path,
                &serde_json::to_string(&serde_json::json!({
                    "hooks": {
                        "PostToolUse": [{
                            "matcher": matcher,
                            "timeout": 3,
                            "hooks": [{
                                "type": "command",
                                "command": format!("{script_root}/run.sh"),
                                "hook": "run.sh",
                            }],
                        }],
                    },
                }))
                .unwrap(),
            );
        };

        write_manifest("afterShellExecution", "Bash", "first");
        write_nested_platform_binding(&home.join(".codex/hooks.json"), ".codex/hooks", "Bash");
        write_nested_platform_binding(&home.join(".claude/settings.json"), ".claude/hooks", "Bash");
        let before = inventory(&request).unwrap();

        write_manifest("beforeShellExecution", "Edit|Write", "second");
        write_nested_platform_binding(
            &home.join(".codex/hooks.json"),
            ".codex/hooks",
            "Edit|Write",
        );
        write_nested_platform_binding(
            &home.join(".claude/settings.json"),
            ".claude/hooks",
            "Edit|Write",
        );
        let after = inventory(&request).unwrap();

        let digest_at = |report: &MigrationInventory, path: &Utf8Path| {
            report
                .entries
                .iter()
                .find(|entry| {
                    entry.kind == AssetKind::Hook && entry.name == "run.sh" && entry.path == path
                })
                .and_then(|entry| entry.content_digest.clone())
                .unwrap_or_else(|| panic!("missing Hook digest at {path}"))
        };
        let canonical_changed = digest_at(&before, &source) != digest_at(&after, &source);
        let codex_changed = digest_at(&before, &home.join(".codex/hooks.json"))
            != digest_at(&after, &home.join(".codex/hooks.json"));
        let claude_changed = digest_at(&before, &home.join(".claude/settings.json"))
            != digest_at(&after, &home.join(".claude/settings.json"));
        let plan_changed = before.plan_digest != after.plan_digest;

        assert!(
            canonical_changed && codex_changed && claude_changed && plan_changed,
            "Hook semantic fingerprints must include canonical lifecycle/matcher/args and \
             nested platform matcher: canonical_changed={canonical_changed} \
             codex_changed={codex_changed} claude_changed={claude_changed} \
             plan_changed={plan_changed}"
        );
    }

    #[test]
    fn invalid_hook_lifecycle_and_unresolvable_command_leaf_are_never_canonical_sources() {
        let temp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(temp.path()).unwrap();
        let asset_root = home.join(".ai-config");
        let request = InventoryRequest {
            canonical_layers: vec![CanonicalLayerRoot {
                layer: SourceLayer::Global,
                asset_root: asset_root.clone(),
            }],
            deploy_base: home.to_path_buf(),
            scope: InventoryScope::Global,
        };
        write(&asset_root.join("hooks/unknown.sh"), "#!/bin/sh\n");
        write(
            &asset_root.join("hooks/bundle/hook.yaml"),
            "entry: scripts/missing.sh\n",
        );
        write(&asset_root.join("hooks/file.sh"), "#!/bin/sh\n");
        write(
            &asset_root.join("hooks.json"),
            &serde_json::to_string(&serde_json::json!({
                "version": 1,
                "hooks": {
                    "notARealLifecycle": [{
                        "command": "./hooks/unknown.sh",
                    }],
                    "sessionStart": [
                        {
                            "command": "./hooks/bundle/scripts/missing.sh",
                        },
                        {
                            "command": "./hooks/file.sh/missing",
                        },
                    ],
                },
            }))
            .unwrap(),
        );

        let report = inventory(&request).unwrap();
        let accepted = report
            .entries
            .iter()
            .filter(|entry| {
                entry.kind == AssetKind::Hook
                    && entry.classification == InventoryClassification::CanonicalSource
            })
            .map(|entry| entry.name.clone())
            .collect::<BTreeSet<_>>();
        let blocking_reasons = report
            .issues
            .iter()
            .filter(|issue| issue.kind == AssetKind::Hook && issue.blocking)
            .map(|issue| issue.reason_code.as_str())
            .collect::<BTreeSet<_>>();
        let unknown_is_unsupported = report.unsupported.iter().any(|entry| {
            entry.kind == AssetKind::Hook
                && entry.name == "unknown.sh"
                && entry.reason_code.contains("lifecycle")
        });

        assert!(
            accepted.is_empty()
                && (unknown_is_unsupported
                    || blocking_reasons.contains("unsupported_canonical_hook_lifecycle"))
                && blocking_reasons.contains("canonical_hook_command_missing_entry")
                && blocking_reasons.contains("canonical_hook_command_unit_shape_mismatch"),
            "invalid Hook bindings must fail closed: accepted={accepted:?} \
             blocking_reasons={blocking_reasons:?} unsupported={:?}",
            report.unsupported
        );
    }

    #[cfg(unix)]
    #[test]
    fn hook_bundles_with_non_regular_nodes_fail_closed_in_source_and_platform_targets() {
        use std::os::unix::net::UnixListener;

        let temp = TempDir::new().unwrap();
        let home = Utf8Path::from_path(temp.path()).unwrap();
        let asset_root = home.join(".ai-config");
        let request = InventoryRequest {
            canonical_layers: vec![CanonicalLayerRoot {
                layer: SourceLayer::Global,
                asset_root: asset_root.clone(),
            }],
            deploy_base: home.to_path_buf(),
            scope: InventoryScope::Global,
        };
        write(
            &asset_root.join("hooks/bad-bundle/scripts/run.sh"),
            "#!/bin/sh\n",
        );
        write(
            &asset_root.join("hooks/good-bundle/scripts/run.sh"),
            "#!/bin/sh\n",
        );
        write(
            &asset_root.join("hooks.json"),
            r#"{"version":1,"hooks":{"sessionStart":[{"command":"./hooks/bad-bundle/scripts/run.sh"},{"command":"./hooks/good-bundle/scripts/run.sh"}]}}"#,
        );
        let _canonical_socket =
            UnixListener::bind(asset_root.join("hooks/bad-bundle/state.sock").as_std_path())
                .unwrap();

        write(
            &home.join(".cursor/hooks/good-bundle/scripts/run.sh"),
            "#!/bin/sh\n",
        );
        let platform_bundle = home.join(".cursor/hooks/good-bundle");
        let _platform_socket =
            UnixListener::bind(platform_bundle.join("state.sock").as_std_path()).unwrap();

        let report = inventory(&request).unwrap();
        let bad_canonical_accepted = report.entries.iter().any(|entry| {
            entry.kind == AssetKind::Hook
                && entry.name == "bad-bundle"
                && entry.classification == InventoryClassification::CanonicalSource
        });
        let bad_platform_accepted = report.entries.iter().any(|entry| {
            entry.kind == AssetKind::Hook
                && entry.path == platform_bundle
                && entry.provenance == InventoryProvenance::PlatformCurrent
        });
        let issue_reasons = report
            .issues
            .iter()
            .filter(|issue| issue.kind == AssetKind::Hook && issue.blocking)
            .map(|issue| issue.reason_code.as_str())
            .collect::<BTreeSet<_>>();
        let canonical_bundle_blocked = report.issues.iter().any(|issue| {
            issue.kind == AssetKind::Hook
                && issue.blocking
                && issue.path == asset_root.join("hooks/bad-bundle")
        });
        let platform_bundle_blocked = report.issues.iter().any(|issue| {
            issue.kind == AssetKind::Hook && issue.blocking && issue.path == platform_bundle
        });

        assert!(
            !bad_canonical_accepted
                && !bad_platform_accepted
                && canonical_bundle_blocked
                && platform_bundle_blocked,
            "Hook bundles must reject sockets/FIFOs/Other nodes: \
             bad_canonical_accepted={bad_canonical_accepted} \
             bad_platform_accepted={bad_platform_accepted} \
             issue_reasons={issue_reasons:?}"
        );
    }
}

fn canonical_hook_units(
    hooks: &Utf8Path,
    scope: InventoryScope,
    issues: &mut Vec<MigrationInventoryIssue>,
) -> Result<BTreeMap<String, Utf8PathBuf>, CoreError> {
    let metadata = match fs::symlink_metadata(hooks.as_std_path()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(_) => {
            push_inventory_issue(
                issues,
                AssetKind::Hook,
                hooks,
                scope,
                "unreadable_canonical_hook_directory",
            );
            return Ok(BTreeMap::new());
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        push_inventory_issue(
            issues,
            AssetKind::Hook,
            hooks,
            scope,
            "unsafe_canonical_hook_directory",
        );
        return Ok(BTreeMap::new());
    }
    let mut units = BTreeMap::new();
    for entry in direct_lstat_entries(hooks)? {
        if is_excluded_agent_entry(&entry.name) || !is_safe_asset_name(&entry.name) {
            continue;
        }
        match entry.shape {
            EntryShape::RegularFile => {
                if fs::read_to_string(entry.path.as_std_path()).is_err() {
                    push_inventory_issue(
                        issues,
                        AssetKind::Hook,
                        &entry.path,
                        scope,
                        "unreadable_canonical_hook_unit",
                    );
                    continue;
                }
                units.insert(entry.name, entry.path);
            }
            EntryShape::Directory => {
                if hook_tree_contains_link(&entry.path)? {
                    push_inventory_issue(
                        issues,
                        AssetKind::Hook,
                        &entry.path,
                        scope,
                        "unsafe_canonical_hook_bundle_symlink",
                    );
                    continue;
                }
                units.insert(entry.name, entry.path);
            }
            EntryShape::Symlink => push_inventory_issue(
                issues,
                AssetKind::Hook,
                &entry.path,
                scope,
                "unsafe_canonical_hook_unit_symlink",
            ),
            EntryShape::Other => push_inventory_issue(
                issues,
                AssetKind::Hook,
                &entry.path,
                scope,
                "unsafe_canonical_hook_unit_non_regular",
            ),
        }
    }
    Ok(units)
}

fn hook_tree_contains_link(root: &Utf8Path) -> Result<bool, CoreError> {
    for entry in direct_lstat_entries(root)? {
        match entry.shape {
            EntryShape::Symlink | EntryShape::Other => return Ok(true),
            EntryShape::Directory if hook_tree_contains_link(&entry.path)? => return Ok(true),
            EntryShape::Directory | EntryShape::RegularFile => {}
        }
    }
    Ok(false)
}

fn scan_canonical_agents(
    root: &CanonicalLayerRoot,
    scope: InventoryScope,
    issues: &mut Vec<MigrationInventoryIssue>,
    legacy_agents: &mut Vec<MigrationInventoryEntry>,
    assets: &mut Vec<CanonicalAsset>,
) -> Result<(), CoreError> {
    let asset_root_metadata = match fs::symlink_metadata(root.asset_root.as_std_path()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {
            push_inventory_issue(
                issues,
                AssetKind::Agent,
                &root.asset_root,
                scope,
                "unreadable_canonical_agent_root",
            );
            return Ok(());
        }
    };
    if asset_root_metadata.file_type().is_symlink() || !asset_root_metadata.is_dir() {
        push_inventory_issue(
            issues,
            AssetKind::Agent,
            &root.asset_root,
            scope,
            "unsafe_canonical_agent_root",
        );
        return Ok(());
    }
    let agents = root.asset_root.join("agents");
    let metadata = match fs::symlink_metadata(agents.as_std_path()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {
            push_inventory_issue(
                issues,
                AssetKind::Agent,
                &agents,
                scope,
                "unreadable_canonical_agent_directory",
            );
            return Ok(());
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        push_inventory_issue(
            issues,
            AssetKind::Agent,
            &agents,
            scope,
            "unsafe_canonical_agent_directory",
        );
        return Ok(());
    }
    let entries = match direct_lstat_entries(&agents) {
        Ok(entries) => entries,
        Err(_) => {
            push_inventory_issue(
                issues,
                AssetKind::Agent,
                &agents,
                scope,
                "unreadable_canonical_agent_directory",
            );
            return Ok(());
        }
    };
    for entry in entries {
        if is_excluded_agent_entry(&entry.name) {
            continue;
        }
        if entry.shape == EntryShape::Symlink {
            push_inventory_issue(
                issues,
                AssetKind::Agent,
                &entry.path,
                scope,
                "unsafe_canonical_agent_symlink",
            );
            continue;
        }
        if entry.shape == EntryShape::Directory
            || (entry.shape == EntryShape::RegularFile
                && matches!(entry.path.extension(), Some("yaml" | "yml" | "json")))
        {
            append_legacy_canonical_agent(root, scope, legacy_agents, &entry);
            continue;
        }
        if entry.shape != EntryShape::RegularFile || entry.path.extension() != Some("md") {
            continue;
        }
        let Some(name) = entry
            .path
            .file_stem()
            .filter(|name| is_safe_asset_name(name))
        else {
            push_inventory_issue(
                issues,
                AssetKind::Agent,
                &entry.path,
                scope,
                "invalid_canonical_agent_name",
            );
            continue;
        };
        let document = match fs::read_to_string(entry.path.as_std_path()) {
            Ok(document) => document,
            Err(_) => {
                push_inventory_issue(
                    issues,
                    AssetKind::Agent,
                    &entry.path,
                    scope,
                    "unreadable_canonical_agent",
                );
                continue;
            }
        };
        match validate_canonical_agent_markdown(&document, name) {
            Ok(()) => {}
            Err(CanonicalAgentSchemaError::NameMismatch) => {
                push_inventory_issue(
                    issues,
                    AssetKind::Agent,
                    &entry.path,
                    scope,
                    "canonical_agent_name_mismatch",
                );
                continue;
            }
            Err(CanonicalAgentSchemaError::Invalid) => {
                push_inventory_issue(
                    issues,
                    AssetKind::Agent,
                    &entry.path,
                    scope,
                    "invalid_canonical_agent_schema",
                );
                continue;
            }
        }
        match canonical_asset(
            root.layer,
            AssetKind::Agent,
            name.to_owned(),
            entry.path.clone(),
        ) {
            Ok(asset) => assets.push(asset),
            Err(_) => push_inventory_issue(
                issues,
                AssetKind::Agent,
                &entry.path,
                scope,
                "unreadable_canonical_agent",
            ),
        }
    }
    Ok(())
}

fn validate_canonical_agent_markdown(
    document: &str,
    expected_name: &str,
) -> Result<(), CanonicalAgentSchemaError> {
    let mut lines = document.lines();
    if lines.next() != Some("---") {
        return Err(CanonicalAgentSchemaError::Invalid);
    }
    let mut frontmatter = Vec::new();
    let mut closed = false;
    for line in lines.by_ref() {
        if line == "---" {
            closed = true;
            break;
        }
        frontmatter.push(line);
    }
    if !closed {
        return Err(CanonicalAgentSchemaError::Invalid);
    }
    let body = lines.collect::<Vec<_>>().join("\n");
    if body.trim().is_empty() {
        return Err(CanonicalAgentSchemaError::Invalid);
    }
    let frontmatter = frontmatter.join("\n");
    let value = serde_yaml::from_str::<serde_yaml::Value>(&frontmatter)
        .map_err(|_| CanonicalAgentSchemaError::Invalid)?;
    let mapping = value
        .as_mapping()
        .ok_or(CanonicalAgentSchemaError::Invalid)?;
    let field = |key: &str| {
        mapping
            .get(serde_yaml::Value::String(key.to_owned()))
            .and_then(serde_yaml::Value::as_str)
            .filter(|value| !value.trim().is_empty())
    };
    let name = field("name").ok_or(CanonicalAgentSchemaError::Invalid)?;
    let _description = field("description").ok_or(CanonicalAgentSchemaError::Invalid)?;
    if name != expected_name {
        return Err(CanonicalAgentSchemaError::NameMismatch);
    }
    Ok(())
}

fn append_legacy_canonical_agent(
    root: &CanonicalLayerRoot,
    scope: InventoryScope,
    entries: &mut Vec<MigrationInventoryEntry>,
    entry: &DirectEntry,
) {
    let name = if entry.shape == EntryShape::Directory {
        entry.name.clone()
    } else {
        entry.path.file_stem().unwrap_or(&entry.name).to_owned()
    };
    if !is_safe_asset_name(&name) {
        return;
    }
    let content_digest = if entry.shape == EntryShape::RegularFile {
        path_content_digest(&entry.path).ok()
    } else {
        None
    };
    entries.push(MigrationInventoryEntry {
        name,
        path: entry.path.clone(),
        kind: AssetKind::Agent,
        classification: InventoryClassification::LegacyAgentCandidate,
        provenance: InventoryProvenance::CanonicalLegacy,
        reason_code: "legacy_canonical_agent_shape".to_owned(),
        content_digest,
        currently_consumed: false,
        blocking: false,
        ownership_state: InventoryOwnershipState::Foreign,
        owned: false,
        selectable: false,
        followed: false,
        source_layer: Some(root.layer),
        canonical_path: Some(entry.path.clone()),
        consumers: Vec::new(),
        scope,
        entry_key: None,
        format: Some(agent_entry_format(entry)),
        secret_keys: Vec::new(),
        trust_requirement: TrustRequirement::None,
    });
}

fn canonical_asset(
    layer: SourceLayer,
    kind: AssetKind,
    name: String,
    path: Utf8PathBuf,
) -> Result<CanonicalAsset, CoreError> {
    // The direct child was selected from an explicitly injected canonical root.  It is the only
    // place migration inventory resolves a link-like filesystem path.
    let resolved_path = fs::canonicalize(path.as_std_path())
        .map_err(CoreError::Io)
        .and_then(utf8_path)?;
    Ok(CanonicalAsset {
        name,
        digest: path_content_digest(&path)?,
        path,
        resolved_path,
        layer,
        kind,
        secret_keys: Vec::new(),
        targets: None,
        hook_binding_digest: None,
    })
}

fn canonical_entry(asset: &CanonicalAsset, scope: InventoryScope) -> MigrationInventoryEntry {
    let content_digest = if asset.kind == AssetKind::Hook {
        asset
            .hook_binding_digest
            .as_ref()
            .map(|binding| hook_compound_digest(&asset.digest, binding))
            .unwrap_or_else(|| asset.digest.clone())
    } else {
        asset.digest.clone()
    };
    let mut entry = MigrationInventoryEntry {
        name: asset.name.clone(),
        path: asset.path.clone(),
        kind: asset.kind,
        classification: InventoryClassification::CanonicalSource,
        provenance: InventoryProvenance::Canonical,
        reason_code: "canonical_source".to_owned(),
        content_digest: Some(content_digest),
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
        entry_key: None,
        format: None,
        secret_keys: asset.secret_keys.clone(),
        trust_requirement: TrustRequirement::None,
    };
    if asset.kind == AssetKind::Hook {
        entry.entry_key = Some(format!("hooks.{}", asset.name));
        entry.format = Some(hook_unit_format(&asset.path).to_owned());
    }
    entry
}

fn hook_compound_digest(script_digest: &str, binding_digest: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"hook\0");
    hasher.update(script_digest.as_bytes());
    hasher.update([0]);
    hasher.update(binding_digest.as_bytes());
    hex::encode(hasher.finalize())
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

fn scan_mcp_targets(
    request: &InventoryRequest,
    canonical: &BTreeMap<(u8, String), CanonicalAsset>,
    entries: &mut Vec<MigrationInventoryEntry>,
    unsupported: &mut Vec<MigrationInventoryUnsupported>,
    issues: &mut Vec<MigrationInventoryIssue>,
) -> Result<(), CoreError> {
    scan_mcp_container(
        &request.deploy_base,
        &request.deploy_base.join(".cursor/mcp.json"),
        "json",
        "mcpServers",
        PlatformId::Cursor,
        InventoryProvenance::PlatformCurrent,
        true,
        "platform_mcp_entry_unowned",
        inspect_cursor_mcp_entries,
        canonical,
        request.scope,
        entries,
        issues,
    )?;
    scan_mcp_container(
        &request.deploy_base,
        &request.deploy_base.join(".codex/config.toml"),
        "toml",
        "mcp_servers",
        PlatformId::Codex,
        InventoryProvenance::PlatformCurrent,
        true,
        "platform_mcp_entry_unowned",
        inspect_codex_mcp_entries,
        canonical,
        request.scope,
        entries,
        issues,
    )?;
    if request.scope == InventoryScope::Global {
        scan_mcp_container(
            &request.deploy_base,
            &request.deploy_base.join(".claude.json"),
            "json",
            "mcpServers",
            PlatformId::Claude,
            InventoryProvenance::PlatformCurrent,
            true,
            "platform_mcp_entry_unowned",
            inspect_claude_mcp_entries,
            canonical,
            request.scope,
            entries,
            issues,
        )?;
        scan_mcp_container(
            &request.deploy_base,
            &request.deploy_base.join(".hermes/config.yaml"),
            "yaml",
            "mcp_servers",
            PlatformId::Hermes,
            InventoryProvenance::PlatformCurrent,
            true,
            "platform_mcp_entry_unowned",
            inspect_hermes_mcp_entries,
            canonical,
            request.scope,
            entries,
            issues,
        )?;
        scan_mcp_container(
            &request.deploy_base,
            &request.deploy_base.join(".hermes/mcp.json"),
            "json",
            "mcpServers",
            PlatformId::Hermes,
            InventoryProvenance::PlatformLegacy,
            false,
            "legacy_hermes_mcp_json",
            inspect_cursor_mcp_entries,
            canonical,
            request.scope,
            entries,
            issues,
        )?;
    } else {
        scan_mcp_container(
            &request.deploy_base,
            &request.deploy_base.join(".mcp.json"),
            "json",
            "mcpServers",
            PlatformId::Claude,
            InventoryProvenance::PlatformCurrent,
            true,
            "platform_mcp_entry_unowned",
            inspect_claude_mcp_entries,
            canonical,
            request.scope,
            entries,
            issues,
        )?;
        for asset in canonical
            .values()
            .filter(|asset| asset.kind == AssetKind::Mcp)
        {
            if asset
                .targets
                .as_ref()
                .map_or(true, |targets| targets.contains(&PlatformId::Hermes))
            {
                unsupported.push(MigrationInventoryUnsupported {
                    kind: AssetKind::Mcp,
                    platform: PlatformId::Hermes,
                    name: asset.name.clone(),
                    source_layer: Some(asset.layer),
                    canonical_path: Some(asset.path.clone()),
                    scope: request.scope,
                    reason_code: "hermes_project_mcp_unsupported".to_owned(),
                });
            }
        }
    }
    scan_mcp_container(
        &request.deploy_base,
        &request.deploy_base.join(".codex/mcp.json"),
        "json",
        "mcpServers",
        PlatformId::Codex,
        InventoryProvenance::PlatformLegacy,
        false,
        "legacy_codex_mcp_json",
        inspect_cursor_mcp_entries,
        canonical,
        request.scope,
        entries,
        issues,
    )?;
    scan_mcp_container(
        &request.deploy_base,
        &request.deploy_base.join(".claude/mcp.json"),
        "json",
        "mcpServers",
        PlatformId::Claude,
        InventoryProvenance::PlatformLegacy,
        false,
        "legacy_claude_mcp_json",
        inspect_claude_mcp_entries,
        canonical,
        request.scope,
        entries,
        issues,
    )
}

fn scan_agent_targets(
    request: &InventoryRequest,
    canonical: &BTreeMap<(u8, String), CanonicalAsset>,
    entries: &mut Vec<MigrationInventoryEntry>,
    unsupported: &mut Vec<MigrationInventoryUnsupported>,
    issues: &mut Vec<MigrationInventoryIssue>,
) -> Result<(), CoreError> {
    scan_agent_directory(
        &request.deploy_base,
        &request.deploy_base.join(".cursor/agents"),
        false,
        "md",
        "markdown",
        PlatformId::Cursor,
        InventoryProvenance::PlatformCurrent,
        true,
        "platform_agent_file_unowned",
        canonical,
        request.scope,
        entries,
        issues,
    )?;
    scan_agent_directory(
        &request.deploy_base,
        &request.deploy_base.join(".codex/agents"),
        false,
        "toml",
        "toml",
        PlatformId::Codex,
        InventoryProvenance::PlatformCurrent,
        true,
        "platform_agent_file_unowned",
        canonical,
        request.scope,
        entries,
        issues,
    )?;
    scan_agent_directory(
        &request.deploy_base,
        &request.deploy_base.join(".claude/agents"),
        false,
        "md",
        "markdown",
        PlatformId::Claude,
        InventoryProvenance::PlatformCurrent,
        true,
        "platform_agent_file_unowned",
        canonical,
        request.scope,
        entries,
        issues,
    )?;
    scan_agent_directory(
        &request.deploy_base,
        &request.deploy_base.join(".codex/subagents"),
        true,
        "",
        "",
        PlatformId::Codex,
        InventoryProvenance::PlatformLegacy,
        false,
        "legacy_codex_subagent",
        canonical,
        request.scope,
        entries,
        issues,
    )?;
    scan_agent_directory(
        &request.deploy_base,
        &request.deploy_base.join(".claude/subagents"),
        true,
        "",
        "",
        PlatformId::Claude,
        InventoryProvenance::PlatformLegacy,
        false,
        "legacy_claude_subagent",
        canonical,
        request.scope,
        entries,
        issues,
    )?;
    for asset in canonical
        .values()
        .filter(|asset| asset.kind == AssetKind::Agent)
    {
        unsupported.push(MigrationInventoryUnsupported {
            kind: AssetKind::Agent,
            platform: PlatformId::Hermes,
            name: asset.name.clone(),
            source_layer: Some(asset.layer),
            canonical_path: Some(asset.path.clone()),
            scope: request.scope,
            reason_code: "hermes_static_agent_unsupported".to_owned(),
        });
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn scan_agent_directory(
    approved_root: &Utf8Path,
    directory: &Utf8Path,
    legacy_all_shapes: bool,
    extension: &str,
    format: &str,
    platform: PlatformId,
    provenance: InventoryProvenance,
    currently_consumed: bool,
    reason_code: &str,
    canonical: &BTreeMap<(u8, String), CanonicalAsset>,
    scope: InventoryScope,
    entries: &mut Vec<MigrationInventoryEntry>,
    issues: &mut Vec<MigrationInventoryIssue>,
) -> Result<(), CoreError> {
    if mcp_parent_issue(approved_root, &directory.join(".inventory-probe")).is_some() {
        push_inventory_issue(
            issues,
            AssetKind::Agent,
            directory,
            scope,
            "unsafe_agent_directory_parent",
        );
        return Ok(());
    }
    let metadata = match fs::symlink_metadata(directory.as_std_path()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {
            push_inventory_issue(
                issues,
                AssetKind::Agent,
                directory,
                scope,
                "unreadable_agent_directory",
            );
            return Ok(());
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        push_inventory_issue(
            issues,
            AssetKind::Agent,
            directory,
            scope,
            "unsafe_agent_directory",
        );
        return Ok(());
    }
    let paths = match direct_lstat_entries(directory) {
        Ok(paths) => paths,
        Err(_) => {
            push_inventory_issue(
                issues,
                AssetKind::Agent,
                directory,
                scope,
                "unreadable_agent_directory",
            );
            return Ok(());
        }
    };
    for entry in paths {
        if is_excluded_agent_entry(&entry.name) {
            continue;
        }
        if entry.shape == EntryShape::Symlink {
            push_inventory_issue(
                issues,
                AssetKind::Agent,
                &entry.path,
                scope,
                "unsafe_agent_file_symlink",
            );
            continue;
        }
        if !legacy_all_shapes
            && entry.path.extension() == Some(extension)
            && entry.shape != EntryShape::RegularFile
        {
            push_inventory_issue(
                issues,
                AssetKind::Agent,
                &entry.path,
                scope,
                "unsafe_agent_file_non_regular",
            );
            continue;
        }
        if legacy_all_shapes {
            if !matches!(entry.shape, EntryShape::RegularFile | EntryShape::Directory) {
                continue;
            }
        } else if entry.shape != EntryShape::RegularFile
            || entry.path.extension() != Some(extension)
        {
            continue;
        }
        let name = if entry.shape == EntryShape::Directory {
            entry.name.clone()
        } else {
            entry.path.file_stem().unwrap_or(&entry.name).to_owned()
        };
        if !is_safe_asset_name(&name) {
            push_inventory_issue(
                issues,
                AssetKind::Agent,
                &entry.path,
                scope,
                "invalid_agent_file_name",
            );
            continue;
        }
        let digest = match entry.shape {
            EntryShape::Directory => None,
            EntryShape::RegularFile => match path_content_digest(&entry.path) {
                Ok(digest) => Some(digest),
                Err(_) => {
                    push_inventory_issue(
                        issues,
                        AssetKind::Agent,
                        &entry.path,
                        scope,
                        "unreadable_agent_file",
                    );
                    continue;
                }
            },
            EntryShape::Symlink | EntryShape::Other => unreachable!("shape was filtered above"),
        };
        let source = canonical.get(&(asset_kind_order(AssetKind::Agent), name.clone()));
        let detected_format = legacy_all_shapes.then(|| agent_entry_format(&entry));
        let mut entry = target_entry(
            name,
            entry.path.clone(),
            AssetKind::Agent,
            InventoryClassification::Foreign,
            provenance.clone(),
            reason_code.to_owned(),
            digest,
            currently_consumed,
            currently_consumed && source.is_some(),
            false,
            source,
            vec![platform],
            scope,
        );
        entry.format = Some(detected_format.unwrap_or_else(|| format.to_owned()));
        entries.push(entry);
    }
    Ok(())
}

fn scan_command_targets(
    request: &InventoryRequest,
    canonical: &BTreeMap<(u8, String), CanonicalAsset>,
    entries: &mut Vec<MigrationInventoryEntry>,
    unsupported: &mut Vec<MigrationInventoryUnsupported>,
    issues: &mut Vec<MigrationInventoryIssue>,
) -> Result<(), CoreError> {
    scan_current_command_root(
        &request.deploy_base,
        &request.deploy_base.join(".cursor/commands"),
        PlatformId::Cursor,
        canonical,
        request.scope,
        entries,
        issues,
    )?;
    scan_current_command_root(
        &request.deploy_base,
        &request.deploy_base.join(".claude/commands"),
        PlatformId::Claude,
        canonical,
        request.scope,
        entries,
        issues,
    )?;
    if request.scope == InventoryScope::Global {
        scan_legacy_codex_prompts(
            &request.deploy_base,
            &request.deploy_base.join(".codex/prompts"),
            canonical,
            request.scope,
            entries,
            issues,
        )?;
    }
    for asset in canonical
        .values()
        .filter(|asset| asset.kind == AssetKind::Command)
    {
        for (platform, reason_code) in [
            (PlatformId::Codex, "codex_command_unsupported"),
            (PlatformId::Hermes, "hermes_command_unsupported"),
        ] {
            unsupported.push(MigrationInventoryUnsupported {
                kind: AssetKind::Command,
                platform,
                name: asset.name.clone(),
                source_layer: Some(asset.layer),
                canonical_path: Some(asset.path.clone()),
                scope: request.scope,
                reason_code: reason_code.to_owned(),
            });
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn scan_current_command_root(
    approved_root: &Utf8Path,
    directory: &Utf8Path,
    platform: PlatformId,
    canonical: &BTreeMap<(u8, String), CanonicalAsset>,
    scope: InventoryScope,
    entries: &mut Vec<MigrationInventoryEntry>,
    issues: &mut Vec<MigrationInventoryIssue>,
) -> Result<(), CoreError> {
    if mcp_parent_issue(approved_root, &directory.join(".inventory-probe")).is_some() {
        push_inventory_issue(
            issues,
            AssetKind::Command,
            directory,
            scope,
            "unsafe_command_directory_parent",
        );
        return Ok(());
    }
    let metadata = match fs::symlink_metadata(directory.as_std_path()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {
            push_inventory_issue(
                issues,
                AssetKind::Command,
                directory,
                scope,
                "unreadable_command_directory",
            );
            return Ok(());
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        push_inventory_issue(
            issues,
            AssetKind::Command,
            directory,
            scope,
            "unsafe_command_directory",
        );
        return Ok(());
    }

    let paths = match direct_lstat_entries(directory) {
        Ok(paths) => paths,
        Err(_) => {
            push_inventory_issue(
                issues,
                AssetKind::Command,
                directory,
                scope,
                "unreadable_command_directory",
            );
            return Ok(());
        }
    };
    for direct in paths {
        if is_excluded_agent_entry(&direct.name) || direct.path.extension() != Some("md") {
            continue;
        }
        let Some(name) = direct
            .path
            .file_stem()
            .filter(|name| is_safe_asset_name(name))
            .map(str::to_owned)
        else {
            push_inventory_issue(
                issues,
                AssetKind::Command,
                &direct.path,
                scope,
                "invalid_command_file_name",
            );
            continue;
        };
        let source = canonical.get(&(asset_kind_order(AssetKind::Command), name.clone()));
        let (classification, reason_code, content_digest, blocking, followed) = match direct.shape {
            EntryShape::Symlink => match classify_link(&direct.path, source, &[]) {
                Ok(result) if result.0 == InventoryClassification::ManagedLink => result,
                Ok(_) | Err(_) => {
                    push_inventory_issue(
                        issues,
                        AssetKind::Command,
                        &direct.path,
                        scope,
                        "unsafe_command_file_symlink",
                    );
                    continue;
                }
            },
            EntryShape::RegularFile => {
                if fs::read_to_string(direct.path.as_std_path()).is_err() {
                    push_inventory_issue(
                        issues,
                        AssetKind::Command,
                        &direct.path,
                        scope,
                        "unreadable_command_file",
                    );
                    continue;
                }
                classify_regular_asset(&direct.path, source, false, "no_canonical_command")?
            }
            EntryShape::Directory | EntryShape::Other => {
                push_inventory_issue(
                    issues,
                    AssetKind::Command,
                    &direct.path,
                    scope,
                    "unsafe_command_file_non_regular",
                );
                continue;
            }
        };
        let mut entry = target_entry(
            name,
            direct.path,
            AssetKind::Command,
            classification,
            InventoryProvenance::PlatformCurrent,
            reason_code,
            content_digest,
            true,
            blocking,
            followed,
            source,
            vec![platform],
            scope,
        );
        entry.format = Some("markdown".to_owned());
        entries.push(entry);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn scan_legacy_codex_prompts(
    approved_root: &Utf8Path,
    directory: &Utf8Path,
    canonical: &BTreeMap<(u8, String), CanonicalAsset>,
    scope: InventoryScope,
    entries: &mut Vec<MigrationInventoryEntry>,
    issues: &mut Vec<MigrationInventoryIssue>,
) -> Result<(), CoreError> {
    if mcp_parent_issue(approved_root, &directory.join(".inventory-probe")).is_some() {
        push_inventory_issue(
            issues,
            AssetKind::Command,
            directory,
            scope,
            "unsafe_legacy_command_directory_parent",
        );
        return Ok(());
    }
    let paths = match direct_lstat_entries(directory) {
        Ok(paths) => paths,
        Err(_) => {
            push_inventory_issue(
                issues,
                AssetKind::Command,
                directory,
                scope,
                "unreadable_legacy_command_directory",
            );
            return Ok(());
        }
    };
    for direct in paths {
        if is_excluded_agent_entry(&direct.name) || direct.path.extension() != Some("md") {
            continue;
        }
        if direct.shape == EntryShape::Symlink {
            push_inventory_issue(
                issues,
                AssetKind::Command,
                &direct.path,
                scope,
                "unsafe_legacy_command_file_symlink",
            );
            continue;
        }
        if direct.shape != EntryShape::RegularFile {
            push_inventory_issue(
                issues,
                AssetKind::Command,
                &direct.path,
                scope,
                "unsafe_legacy_command_file_non_regular",
            );
            continue;
        }
        let Some(name) = direct
            .path
            .file_stem()
            .filter(|name| is_safe_asset_name(name))
            .map(str::to_owned)
        else {
            continue;
        };
        if fs::read_to_string(direct.path.as_std_path()).is_err() {
            push_inventory_issue(
                issues,
                AssetKind::Command,
                &direct.path,
                scope,
                "unreadable_legacy_command_file",
            );
            continue;
        }
        let source = canonical.get(&(asset_kind_order(AssetKind::Command), name.clone()));
        let mut entry = target_entry(
            name,
            direct.path.clone(),
            AssetKind::Command,
            InventoryClassification::Foreign,
            InventoryProvenance::PlatformLegacy,
            "legacy_codex_prompt".to_owned(),
            Some(path_content_digest(&direct.path)?),
            true,
            false,
            false,
            source,
            vec![PlatformId::Codex],
            scope,
        );
        entry.format = Some("markdown".to_owned());
        entries.push(entry);
    }
    Ok(())
}

fn scan_hook_targets(
    request: &InventoryRequest,
    canonical: &BTreeMap<(u8, String), CanonicalAsset>,
    entries: &mut Vec<MigrationInventoryEntry>,
    unsupported: &mut Vec<MigrationInventoryUnsupported>,
    issues: &mut Vec<MigrationInventoryIssue>,
) -> Result<(), CoreError> {
    scan_hook_platform(
        request,
        PlatformId::Cursor,
        &request.deploy_base.join(".cursor/hooks.json"),
        &request.deploy_base.join(".cursor/hooks"),
        "json",
        inspect_cursor_hook_bindings,
        canonical,
        entries,
        issues,
    )?;
    scan_hook_platform(
        request,
        PlatformId::Codex,
        &request.deploy_base.join(".codex/hooks.json"),
        &request.deploy_base.join(".codex/hooks"),
        "json",
        inspect_codex_hook_bindings,
        canonical,
        entries,
        issues,
    )?;
    scan_hook_platform(
        request,
        PlatformId::Claude,
        &request.deploy_base.join(".claude/settings.json"),
        &request.deploy_base.join(".claude/hooks"),
        "json",
        inspect_claude_hook_bindings,
        canonical,
        entries,
        issues,
    )?;
    if request.scope == InventoryScope::Global {
        scan_hook_platform(
            request,
            PlatformId::Hermes,
            &request.deploy_base.join(".hermes/config.yaml"),
            &request.deploy_base.join(".hermes/hooks"),
            "yaml",
            inspect_hermes_hook_bindings,
            canonical,
            entries,
            issues,
        )?;
    } else {
        let reason_code = match request.scope {
            InventoryScope::Workspace => "hermes_workspace_hook_unsupported",
            InventoryScope::Project => "hermes_project_hook_unsupported",
            InventoryScope::Global => unreachable!("global Hermes Hooks were scanned above"),
        };
        for asset in canonical
            .values()
            .filter(|asset| asset.kind == AssetKind::Hook)
        {
            unsupported.push(MigrationInventoryUnsupported {
                kind: AssetKind::Hook,
                platform: PlatformId::Hermes,
                name: asset.name.clone(),
                source_layer: Some(asset.layer),
                canonical_path: Some(asset.path.clone()),
                scope: request.scope,
                reason_code: reason_code.to_owned(),
            });
        }
    }
    scan_legacy_hook_container(
        request,
        &request.deploy_base.join(".codex/config.toml"),
        canonical,
        entries,
        issues,
    )?;
    Ok(())
}

struct HookBindingInspection {
    bindings: BTreeMap<String, String>,
    name_mismatch: bool,
    shared_cursor_names: BTreeSet<String>,
}

struct HookBindingScan {
    names: BTreeSet<String>,
    shared_cursor_names: BTreeSet<String>,
}

type HookBindingInspector = fn(&str, &Utf8Path) -> Result<HookBindingInspection, ()>;

#[allow(clippy::too_many_arguments)]
fn scan_hook_platform(
    request: &InventoryRequest,
    platform: PlatformId,
    container: &Utf8Path,
    scripts: &Utf8Path,
    format: &str,
    inspect: HookBindingInspector,
    canonical: &BTreeMap<(u8, String), CanonicalAsset>,
    entries: &mut Vec<MigrationInventoryEntry>,
    issues: &mut Vec<MigrationInventoryIssue>,
) -> Result<(), CoreError> {
    let trust = hook_trust_requirement(platform, request.scope);
    let binding_scan = scan_hook_binding_container(
        &request.deploy_base,
        container,
        format,
        inspect,
        platform,
        InventoryProvenance::PlatformCurrent,
        "platform_hook_binding_unowned",
        true,
        true,
        trust,
        canonical,
        request.scope,
        entries,
        issues,
    )?;
    let script_names = scan_hook_script_root(
        &request.deploy_base,
        scripts,
        platform,
        trust,
        canonical,
        request.scope,
        entries,
        issues,
    )?;
    let mut shared_script_names = BTreeSet::new();
    if platform == PlatformId::Claude {
        for name in &binding_scan.shared_cursor_names {
            let shared_path = request.deploy_base.join(".cursor/hooks").join(name);
            let shared_entry = entries.iter_mut().find(|entry| {
                entry.kind == AssetKind::Hook
                    && entry.provenance == InventoryProvenance::PlatformCurrent
                    && entry.path == shared_path
                    && entry.consumers.contains(&PlatformId::Cursor)
            });
            if let Some(entry) = shared_entry {
                if !entry.consumers.contains(&PlatformId::Claude) {
                    entry.consumers.push(PlatformId::Claude);
                }
                entry.blocking = matches!(
                    entry.classification,
                    InventoryClassification::Foreign
                        | InventoryClassification::UnsafeLink
                        | InventoryClassification::BrokenLink
                        | InventoryClassification::CaseCollision
                );
                shared_script_names.insert(name.clone());
                issues.retain(|issue| {
                    !(issue.kind == AssetKind::Hook
                        && issue.path == shared_path
                        && issue.scope == request.scope
                        && issue.reason_code == "hook_half_projection")
                });
            }
        }
    }
    let component_names = binding_scan
        .names
        .iter()
        .chain(&script_names)
        .chain(&shared_script_names)
        .cloned()
        .collect::<BTreeSet<_>>();
    for name in component_names {
        let has_binding = binding_scan.names.contains(&name);
        let has_script = script_names.contains(&name) || shared_script_names.contains(&name);
        if has_binding ^ has_script {
            let missing_component = if binding_scan.shared_cursor_names.contains(&name) {
                request.deploy_base.join(".cursor/hooks").join(&name)
            } else {
                scripts.join(&name)
            };
            for entry in entries.iter_mut().filter(|entry| {
                entry.kind == AssetKind::Hook
                    && entry.name == name
                    && entry.provenance == InventoryProvenance::PlatformCurrent
                    && entry.consumers == vec![platform]
                    && (entry.path == container || entry.path == missing_component)
            }) {
                entry.blocking = true;
            }
            push_inventory_issue(
                issues,
                AssetKind::Hook,
                if has_binding {
                    container
                } else {
                    &missing_component
                },
                request.scope,
                "hook_half_projection",
            );
        }
    }
    Ok(())
}

fn hook_trust_requirement(platform: PlatformId, scope: InventoryScope) -> TrustRequirement {
    if platform == PlatformId::Codex && scope != InventoryScope::Global {
        TrustRequirement::TrustedProjectWithIndependentReview
    } else {
        TrustRequirement::None
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_hook_binding_container(
    approved_root: &Utf8Path,
    container: &Utf8Path,
    format: &str,
    inspect: HookBindingInspector,
    platform: PlatformId,
    provenance: InventoryProvenance,
    reason_code: &str,
    currently_consumed: bool,
    block_with_source: bool,
    trust: TrustRequirement,
    canonical: &BTreeMap<(u8, String), CanonicalAsset>,
    scope: InventoryScope,
    entries: &mut Vec<MigrationInventoryEntry>,
    issues: &mut Vec<MigrationInventoryIssue>,
) -> Result<HookBindingScan, CoreError> {
    if mcp_parent_issue(approved_root, container).is_some() {
        push_inventory_issue(
            issues,
            AssetKind::Hook,
            container,
            scope,
            "unsafe_hook_container_parent",
        );
        return Ok(HookBindingScan {
            names: BTreeSet::new(),
            shared_cursor_names: BTreeSet::new(),
        });
    }
    let metadata = match fs::symlink_metadata(container.as_std_path()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(HookBindingScan {
                names: BTreeSet::new(),
                shared_cursor_names: BTreeSet::new(),
            })
        }
        Err(_) => {
            push_inventory_issue(
                issues,
                AssetKind::Hook,
                container,
                scope,
                "unreadable_hook_container",
            );
            return Ok(HookBindingScan {
                names: BTreeSet::new(),
                shared_cursor_names: BTreeSet::new(),
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        push_inventory_issue(
            issues,
            AssetKind::Hook,
            container,
            scope,
            "unsafe_hook_container",
        );
        return Ok(HookBindingScan {
            names: BTreeSet::new(),
            shared_cursor_names: BTreeSet::new(),
        });
    }
    let raw = match fs::read_to_string(container.as_std_path()) {
        Ok(raw) => raw,
        Err(_) => {
            push_inventory_issue(
                issues,
                AssetKind::Hook,
                container,
                scope,
                "unreadable_hook_container",
            );
            return Ok(HookBindingScan {
                names: BTreeSet::new(),
                shared_cursor_names: BTreeSet::new(),
            });
        }
    };
    let inspection = match inspect(&raw, approved_root) {
        Ok(inspection) => inspection,
        Err(()) => {
            push_inventory_issue(
                issues,
                AssetKind::Hook,
                container,
                scope,
                "invalid_hook_container",
            );
            return Ok(HookBindingScan {
                names: BTreeSet::new(),
                shared_cursor_names: BTreeSet::new(),
            });
        }
    };
    if inspection.name_mismatch {
        push_inventory_issue(
            issues,
            AssetKind::Hook,
            container,
            scope,
            "hook_binding_script_name_mismatch",
        );
    }
    let shared_cursor_names = inspection.shared_cursor_names;
    let bindings = inspection.bindings;
    let names = bindings.keys().cloned().collect::<BTreeSet<_>>();
    for (name, digest) in bindings {
        let source = canonical.get(&(asset_kind_order(AssetKind::Hook), name.clone()));
        let mut entry = target_entry(
            name.clone(),
            container.to_path_buf(),
            AssetKind::Hook,
            InventoryClassification::Foreign,
            provenance.clone(),
            reason_code.to_owned(),
            Some(digest),
            currently_consumed,
            block_with_source && source.is_some(),
            false,
            source,
            vec![platform],
            scope,
        );
        entry.entry_key = Some(format!("hooks.{name}"));
        entry.format = Some(format.to_owned());
        entry.trust_requirement = trust;
        entries.push(entry);
    }
    Ok(HookBindingScan {
        names,
        shared_cursor_names,
    })
}

#[allow(clippy::too_many_arguments)]
fn scan_hook_script_root(
    approved_root: &Utf8Path,
    scripts: &Utf8Path,
    platform: PlatformId,
    trust: TrustRequirement,
    canonical: &BTreeMap<(u8, String), CanonicalAsset>,
    scope: InventoryScope,
    entries: &mut Vec<MigrationInventoryEntry>,
    issues: &mut Vec<MigrationInventoryIssue>,
) -> Result<BTreeSet<String>, CoreError> {
    if mcp_parent_issue(approved_root, &scripts.join(".inventory-probe")).is_some() {
        push_inventory_issue(
            issues,
            AssetKind::Hook,
            scripts,
            scope,
            "unsafe_hook_script_parent",
        );
        return Ok(BTreeSet::new());
    }
    let metadata = match fs::symlink_metadata(scripts.as_std_path()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(_) => {
            push_inventory_issue(
                issues,
                AssetKind::Hook,
                scripts,
                scope,
                "unreadable_hook_script_directory",
            );
            return Ok(BTreeSet::new());
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        push_inventory_issue(
            issues,
            AssetKind::Hook,
            scripts,
            scope,
            "unsafe_hook_script_directory",
        );
        return Ok(BTreeSet::new());
    }
    let mut names = BTreeSet::new();
    for direct in direct_lstat_entries(scripts)? {
        if is_excluded_agent_entry(&direct.name) || !is_safe_asset_name(&direct.name) {
            continue;
        }
        let source = canonical.get(&(asset_kind_order(AssetKind::Hook), direct.name.clone()));
        let (classification, reason_code, content_digest, blocking, followed, unit_format) =
            match direct.shape {
                EntryShape::Symlink => match classify_link(&direct.path, source, &[]) {
                    Ok(result) if result.0 == InventoryClassification::ManagedLink => (
                        result.0,
                        result.1,
                        result.2,
                        result.3,
                        result.4,
                        source.map_or("unknown", |asset| hook_unit_format(&asset.path)),
                    ),
                    Ok(_) | Err(_) => {
                        push_inventory_issue(
                            issues,
                            AssetKind::Hook,
                            &direct.path,
                            scope,
                            "unsafe_hook_script_symlink",
                        );
                        continue;
                    }
                },
                EntryShape::RegularFile => {
                    if fs::read_to_string(direct.path.as_std_path()).is_err() {
                        push_inventory_issue(
                            issues,
                            AssetKind::Hook,
                            &direct.path,
                            scope,
                            "unreadable_hook_script",
                        );
                        continue;
                    }
                    let result =
                        classify_regular_asset(&direct.path, source, false, "no_canonical_hook")?;
                    (result.0, result.1, result.2, result.3, result.4, "file")
                }
                EntryShape::Directory => {
                    if hook_tree_contains_link(&direct.path)? {
                        push_inventory_issue(
                            issues,
                            AssetKind::Hook,
                            &direct.path,
                            scope,
                            "unsafe_hook_script_bundle_symlink",
                        );
                        continue;
                    }
                    let result =
                        classify_regular_asset(&direct.path, source, false, "no_canonical_hook")?;
                    (
                        result.0,
                        result.1,
                        result.2,
                        result.3,
                        result.4,
                        "directory",
                    )
                }
                EntryShape::Other => {
                    push_inventory_issue(
                        issues,
                        AssetKind::Hook,
                        &direct.path,
                        scope,
                        "unsafe_hook_script_non_regular",
                    );
                    continue;
                }
            };
        names.insert(direct.name.clone());
        let mut entry = target_entry(
            direct.name,
            direct.path,
            AssetKind::Hook,
            classification,
            InventoryProvenance::PlatformCurrent,
            reason_code,
            content_digest,
            true,
            blocking,
            followed,
            source,
            vec![platform],
            scope,
        );
        entry.format = Some(unit_format.to_owned());
        entry.trust_requirement = trust;
        entries.push(entry);
    }
    Ok(names)
}

fn scan_legacy_hook_container(
    request: &InventoryRequest,
    container: &Utf8Path,
    canonical: &BTreeMap<(u8, String), CanonicalAsset>,
    entries: &mut Vec<MigrationInventoryEntry>,
    issues: &mut Vec<MigrationInventoryIssue>,
) -> Result<(), CoreError> {
    scan_hook_binding_container(
        &request.deploy_base,
        container,
        "toml",
        inspect_grouped_toml_hook_bindings,
        PlatformId::Codex,
        InventoryProvenance::PlatformLegacy,
        "legacy_codex_inline_hook",
        true,
        false,
        hook_trust_requirement(PlatformId::Codex, request.scope),
        canonical,
        request.scope,
        entries,
        issues,
    )?;
    Ok(())
}

fn inspect_cursor_hook_bindings(
    raw: &str,
    deploy_base: &Utf8Path,
) -> Result<HookBindingInspection, ()> {
    let document = serde_json::from_str::<serde_json::Value>(raw).map_err(|_| ())?;
    inspect_direct_hook_document(&document, PlatformId::Cursor, deploy_base)
}

fn inspect_codex_hook_bindings(
    raw: &str,
    deploy_base: &Utf8Path,
) -> Result<HookBindingInspection, ()> {
    let document = serde_json::from_str::<serde_json::Value>(raw).map_err(|_| ())?;
    inspect_grouped_hook_document(&document, PlatformId::Codex, deploy_base)
}

fn inspect_claude_hook_bindings(
    raw: &str,
    deploy_base: &Utf8Path,
) -> Result<HookBindingInspection, ()> {
    let document = serde_json::from_str::<serde_json::Value>(raw).map_err(|_| ())?;
    inspect_grouped_hook_document(&document, PlatformId::Claude, deploy_base)
}

fn inspect_hermes_hook_bindings(
    raw: &str,
    deploy_base: &Utf8Path,
) -> Result<HookBindingInspection, ()> {
    let document = serde_yaml::from_str::<serde_yaml::Value>(raw).map_err(|_| ())?;
    let document = serde_json::to_value(document).map_err(|_| ())?;
    inspect_direct_hook_document(&document, PlatformId::Hermes, deploy_base)
}

fn inspect_grouped_toml_hook_bindings(
    raw: &str,
    deploy_base: &Utf8Path,
) -> Result<HookBindingInspection, ()> {
    let document = toml::from_str::<toml::Value>(raw).map_err(|_| ())?;
    let document = serde_json::to_value(document).map_err(|_| ())?;
    inspect_grouped_hook_document(&document, PlatformId::Codex, deploy_base)
}

fn empty_hook_binding_inspection() -> HookBindingInspection {
    HookBindingInspection {
        bindings: BTreeMap::new(),
        name_mismatch: false,
        shared_cursor_names: BTreeSet::new(),
    }
}

fn hook_event_map(
    document: &serde_json::Value,
) -> Result<Option<&serde_json::Map<String, serde_json::Value>>, ()> {
    let root = document.as_object().ok_or(())?;
    match root.get("hooks") {
        None => Ok(None),
        Some(serde_json::Value::Object(hooks)) => Ok(Some(hooks)),
        Some(_) => Err(()),
    }
}

fn inspect_direct_hook_document(
    document: &serde_json::Value,
    platform: PlatformId,
    deploy_base: &Utf8Path,
) -> Result<HookBindingInspection, ()> {
    let Some(hooks) = hook_event_map(document)? else {
        return Ok(empty_hook_binding_inspection());
    };
    let mut values = BTreeMap::<String, Vec<Vec<u8>>>::new();
    let mut name_mismatch = false;
    let mut shared_cursor_names = BTreeSet::new();
    for (lifecycle, entries) in hooks {
        let Some(entries) = entries.as_array() else {
            // Unknown extension keys are preserved by generated renderers and are
            // outside the native lifecycle grammar. They cannot create bindings.
            continue;
        };
        for entry in entries {
            let Some(binding) = entry.as_object() else {
                continue;
            };
            let semantic_value = serde_json::json!({
                "lifecycle": lifecycle,
                "binding": entry,
            });
            record_native_hook_binding(
                platform,
                deploy_base,
                binding,
                &semantic_value,
                &mut values,
                &mut name_mismatch,
                &mut shared_cursor_names,
            );
        }
    }
    Ok(finish_hook_binding_inspection(
        values,
        name_mismatch,
        shared_cursor_names,
    ))
}

fn inspect_grouped_hook_document(
    document: &serde_json::Value,
    platform: PlatformId,
    deploy_base: &Utf8Path,
) -> Result<HookBindingInspection, ()> {
    let Some(hooks) = hook_event_map(document)? else {
        return Ok(empty_hook_binding_inspection());
    };
    let mut values = BTreeMap::<String, Vec<Vec<u8>>>::new();
    let mut name_mismatch = false;
    let mut shared_cursor_names = BTreeSet::new();
    for (lifecycle, groups) in hooks {
        let Some(groups) = groups.as_array() else {
            continue;
        };
        for group in groups {
            let Some(group_object) = group.as_object() else {
                continue;
            };
            let Some(bindings) = group_object
                .get("hooks")
                .and_then(serde_json::Value::as_array)
            else {
                continue;
            };
            for binding in bindings {
                let Some(binding_object) = binding.as_object() else {
                    continue;
                };
                let mut isolated_group = group_object.clone();
                isolated_group.insert(
                    "hooks".to_owned(),
                    serde_json::Value::Array(vec![binding.clone()]),
                );
                let semantic_value = serde_json::json!({
                    "lifecycle": lifecycle,
                    "group": isolated_group,
                });
                record_native_hook_binding(
                    platform,
                    deploy_base,
                    binding_object,
                    &semantic_value,
                    &mut values,
                    &mut name_mismatch,
                    &mut shared_cursor_names,
                );
            }
        }
    }
    Ok(finish_hook_binding_inspection(
        values,
        name_mismatch,
        shared_cursor_names,
    ))
}

fn record_native_hook_binding(
    platform: PlatformId,
    deploy_base: &Utf8Path,
    binding: &serde_json::Map<String, serde_json::Value>,
    semantic_value: &serde_json::Value,
    values: &mut BTreeMap<String, Vec<Vec<u8>>>,
    name_mismatch: &mut bool,
    shared_cursor_names: &mut BTreeSet<String>,
) {
    let Some(command) = binding.get("command").and_then(serde_json::Value::as_str) else {
        return;
    };
    let marker_present = binding.contains_key("hook");
    let marker = binding
        .get("hook")
        .and_then(serde_json::Value::as_str)
        .filter(|name| is_safe_asset_name(name))
        .map(str::to_owned);
    let command_name = hook_name_from_platform_command(command, platform, deploy_base);
    let name = match (marker_present, marker, command_name) {
        (true, Some(marker), Some(command_name)) if marker == command_name => Some(marker),
        (true, _, _) => {
            *name_mismatch = true;
            None
        }
        (false, _, command_name) => command_name,
    };
    if let Some(name) = name {
        if platform == PlatformId::Claude && claude_command_uses_shared_cursor_root(command, &name)
        {
            shared_cursor_names.insert(name.clone());
        }
        if let Ok(encoded) = serde_json::to_vec(semantic_value) {
            values.entry(name).or_default().push(encoded);
        }
    }
}

fn finish_hook_binding_inspection(
    values: BTreeMap<String, Vec<Vec<u8>>>,
    name_mismatch: bool,
    shared_cursor_names: BTreeSet<String>,
) -> HookBindingInspection {
    let bindings = values
        .into_iter()
        .map(|(name, mut encoded_values)| {
            encoded_values.sort();
            let mut hasher = Sha256::new();
            hasher.update(b"hook-container-entry\0");
            hasher.update(name.as_bytes());
            hasher.update([0]);
            for encoded in encoded_values {
                hasher.update(encoded);
                hasher.update([0]);
            }
            (name, hex::encode(hasher.finalize()))
        })
        .collect();
    HookBindingInspection {
        bindings,
        name_mismatch,
        shared_cursor_names,
    }
}

fn claude_command_uses_shared_cursor_root(command: &str, name: &str) -> bool {
    let Some(executable) = command.split_whitespace().next() else {
        return false;
    };
    Utf8Path::new(executable)
        .components()
        .map(|component| component.as_str())
        .eq([".cursor", "hooks", name])
}

fn hook_name_from_platform_command(
    command: &str,
    platform: PlatformId,
    deploy_base: &Utf8Path,
) -> Option<String> {
    let executable = command.split_whitespace().next()?;
    let path = Utf8Path::new(executable);
    if path.is_absolute() {
        let relative_root = match platform {
            PlatformId::Cursor => ".cursor/hooks",
            PlatformId::Codex => ".codex/hooks",
            PlatformId::Claude => ".claude/hooks",
            PlatformId::Hermes => ".hermes/hooks",
            PlatformId::AiConfig => return None,
        };
        let name = path.file_name().filter(|name| is_safe_asset_name(name))?;
        return (path == deploy_base.join(relative_root).join(name)).then(|| name.to_owned());
    }
    let components = path
        .components()
        .map(|component| component.as_str())
        .collect::<Vec<_>>();
    let name = match platform {
        PlatformId::Cursor => match components.as_slice() {
            [".", "hooks", name] | ["hooks", name] | [".cursor", "hooks", name] => Some(*name),
            _ => None,
        },
        PlatformId::Codex => match components.as_slice() {
            [".codex", "hooks", name] => Some(*name),
            _ => None,
        },
        PlatformId::Claude => match components.as_slice() {
            [".claude", "hooks", name]
            | [".cursor", "hooks", name]
            | ["${CLAUDE_PROJECT_DIR}", ".claude", "hooks", name] => Some(*name),
            _ => None,
        },
        PlatformId::Hermes => match components.as_slice() {
            [".hermes", "hooks", name] => Some(*name),
            _ => None,
        },
        PlatformId::AiConfig => None,
    }?;
    is_safe_asset_name(name).then(|| name.to_owned())
}

#[allow(clippy::too_many_arguments)]
fn scan_mcp_container(
    approved_root: &Utf8Path,
    path: &Utf8Path,
    format: &str,
    entry_prefix: &str,
    platform: PlatformId,
    provenance: InventoryProvenance,
    currently_consumed: bool,
    reason_code: &str,
    inspect: fn(&str) -> Result<Vec<McpEntryFingerprint>, CoreError>,
    canonical: &BTreeMap<(u8, String), CanonicalAsset>,
    scope: InventoryScope,
    entries: &mut Vec<MigrationInventoryEntry>,
    issues: &mut Vec<MigrationInventoryIssue>,
) -> Result<(), CoreError> {
    if let Some(reason_code) = mcp_parent_issue(approved_root, path) {
        push_mcp_container_issue(issues, path, scope, reason_code);
        return Ok(());
    }
    let metadata = match fs::symlink_metadata(path.as_std_path()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {
            push_mcp_container_issue(issues, path, scope, "unreadable_mcp_container");
            return Ok(());
        }
    };
    // A generated container is never a canonical source.  Unknown links are deliberately not
    // opened: no server name is worth escaping the migration read boundary for.
    if metadata.file_type().is_symlink() {
        push_mcp_container_issue(issues, path, scope, "unsafe_mcp_container_symlink");
        return Ok(());
    }
    if !metadata.is_file() {
        push_mcp_container_issue(issues, path, scope, "unsafe_mcp_container_non_regular");
        return Ok(());
    }
    let existing = match fs::read_to_string(path.as_std_path()) {
        Ok(existing) => existing,
        Err(_) => {
            push_mcp_container_issue(issues, path, scope, "unreadable_mcp_container");
            return Ok(());
        }
    };
    let fingerprints = match inspect(&existing) {
        Ok(fingerprints) => fingerprints,
        Err(_) => {
            push_mcp_container_issue(issues, path, scope, "invalid_mcp_container");
            return Ok(());
        }
    };
    for fingerprint in fingerprints {
        let source = canonical
            .get(&(asset_kind_order(AssetKind::Mcp), fingerprint.name.clone()))
            .filter(|asset| {
                asset
                    .targets
                    .as_ref()
                    .map_or(true, |targets| targets.contains(&platform))
            });
        let blocking = currently_consumed && source.is_some();
        let mut entry = target_entry(
            fingerprint.name.clone(),
            path.to_path_buf(),
            AssetKind::Mcp,
            InventoryClassification::Foreign,
            provenance.clone(),
            reason_code.to_owned(),
            Some(fingerprint.digest),
            currently_consumed,
            blocking,
            false,
            source,
            vec![platform],
            scope,
        );
        entry.entry_key = Some(format!("{entry_prefix}.{}", fingerprint.name));
        entry.format = Some(format.to_owned());
        entry.trust_requirement =
            if platform == PlatformId::Codex && scope != InventoryScope::Global {
                TrustRequirement::TrustedProject
            } else {
                TrustRequirement::None
            };
        entries.push(entry);
    }
    Ok(())
}

fn push_mcp_container_issue(
    issues: &mut Vec<MigrationInventoryIssue>,
    path: &Utf8Path,
    scope: InventoryScope,
    reason_code: &str,
) {
    push_inventory_issue(issues, AssetKind::Mcp, path, scope, reason_code);
}

fn push_inventory_issue(
    issues: &mut Vec<MigrationInventoryIssue>,
    kind: AssetKind,
    path: &Utf8Path,
    scope: InventoryScope,
    reason_code: &str,
) {
    issues.push(MigrationInventoryIssue {
        kind,
        path: path.to_path_buf(),
        scope,
        reason_code: reason_code.to_owned(),
        blocking: true,
    });
}

fn is_safe_asset_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

fn is_excluded_agent_entry(name: &str) -> bool {
    let lowercase = name.to_ascii_lowercase();
    name.starts_with('.')
        || lowercase == "readme"
        || lowercase.starts_with("readme.")
        || matches!(
            lowercase.as_str(),
            "thumbs.db" | "desktop.ini" | "node_modules" | "target" | "dist" | "build"
        )
        || lowercase.ends_with(".bak")
        || lowercase.ends_with(".orig")
        || lowercase.ends_with(".swp")
        || lowercase.ends_with('~')
        || lowercase.ends_with(".tmp")
        || lowercase.ends_with(".ai-config-deploy.json")
}

fn agent_entry_format(entry: &DirectEntry) -> String {
    match entry.shape {
        EntryShape::Directory => "directory".to_owned(),
        EntryShape::RegularFile => match entry.path.extension() {
            Some("md") => "markdown".to_owned(),
            Some("yaml" | "yml") => "yaml".to_owned(),
            Some("json") => "json".to_owned(),
            Some(extension) => extension.to_owned(),
            None => "file".to_owned(),
        },
        EntryShape::Symlink | EntryShape::Other => "unknown".to_owned(),
    }
}

fn mcp_parent_issue(approved_root: &Utf8Path, path: &Utf8Path) -> Option<&'static str> {
    let parent = path.parent()?;
    let relative = parent.strip_prefix(approved_root).ok()?;
    let mut current = approved_root.to_path_buf();
    for component in relative.iter() {
        current.push(component);
        let metadata = match fs::symlink_metadata(current.as_std_path()) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(_) => return Some("unreadable_mcp_container_parent"),
        };
        if metadata.file_type().is_symlink() {
            return Some("unsafe_mcp_container_parent_symlink");
        }
        if !metadata.is_dir() {
            return Some("unsafe_mcp_container_parent_non_directory");
        }
    }
    None
}

fn scan_legacy_canonical_mcp(
    layers: &[CanonicalLayerRoot],
    scope: InventoryScope,
    entries: &mut Vec<MigrationInventoryEntry>,
    issues: &mut Vec<MigrationInventoryIssue>,
) -> Result<(), CoreError> {
    for root in sorted_layers(layers)? {
        let path = root.asset_root.join("mcp.json");
        let metadata = match fs::symlink_metadata(path.as_std_path()) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                push_mcp_container_issue(issues, &path, scope, "unreadable_legacy_canonical_mcp");
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            push_mcp_container_issue(issues, &path, scope, "unsafe_legacy_canonical_mcp_symlink");
            continue;
        }
        if !metadata.is_file() {
            push_mcp_container_issue(
                issues,
                &path,
                scope,
                "unsafe_legacy_canonical_mcp_non_regular",
            );
            continue;
        }
        let existing = match fs::read_to_string(path.as_std_path()) {
            Ok(existing) => existing,
            Err(_) => {
                push_mcp_container_issue(issues, &path, scope, "unreadable_legacy_canonical_mcp");
                continue;
            }
        };
        let fingerprints = match inspect_cursor_mcp_entries(&existing) {
            Ok(fingerprints) => fingerprints,
            Err(_) => {
                push_mcp_container_issue(issues, &path, scope, "invalid_legacy_canonical_mcp");
                continue;
            }
        };
        for fingerprint in fingerprints {
            entries.push(MigrationInventoryEntry {
                name: fingerprint.name.clone(),
                path: path.clone(),
                kind: AssetKind::Mcp,
                classification: InventoryClassification::LegacyMcpCandidate,
                provenance: InventoryProvenance::CanonicalLegacy,
                reason_code: "legacy_monolithic_mcp".to_owned(),
                content_digest: Some(fingerprint.digest),
                currently_consumed: false,
                blocking: false,
                ownership_state: InventoryOwnershipState::Foreign,
                owned: false,
                selectable: false,
                followed: false,
                source_layer: Some(root.layer),
                canonical_path: Some(path.clone()),
                consumers: Vec::new(),
                scope,
                entry_key: Some(format!("mcpServers.{}", fingerprint.name)),
                format: Some("json".to_owned()),
                secret_keys: Vec::new(),
                trust_requirement: TrustRequirement::None,
            });
        }
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
        entry_key: None,
        format: None,
        secret_keys: source
            .map(|asset| asset.secret_keys.clone())
            .unwrap_or_default(),
        trust_requirement: TrustRequirement::None,
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
            entry_key: None,
            format: None,
            secret_keys: Vec::new(),
            trust_requirement: TrustRequirement::None,
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
        if is_legacy_only(entry) {
            continue;
        }
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
        if is_legacy_only(entry) {
            continue;
        }
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

fn is_legacy_only(entry: &MigrationInventoryEntry) -> bool {
    entry.provenance == InventoryProvenance::CanonicalLegacy
        || (entry.provenance == InventoryProvenance::PlatformLegacy
            && (!entry.currently_consumed
                || matches!(entry.kind, AssetKind::Command | AssetKind::Hook)))
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

fn platform_order(platform: PlatformId) -> u8 {
    match platform {
        PlatformId::AiConfig => 0,
        PlatformId::Cursor => 1,
        PlatformId::Codex => 2,
        PlatformId::Claude => 3,
        PlatformId::Hermes => 4,
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
        | InventoryClassification::LegacyMcpCandidate
        | InventoryClassification::LegacyAgentCandidate
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

    #[test]
    fn workspace_command_inventory_uses_workspace_overlay_and_never_scans_home_platforms() {
        const COMMAND_BODY_SENTINEL: &str = "workspace-command-body-must-not-serialize";
        const HOME_PLATFORM_SENTINEL: &str = "workspace-command-home-platform-must-not-be-read";

        let temp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let home = root.join("home");
        let workspace = root.join("workspace");
        let global = home.join(".ai-config");
        let workspace_assets = workspace.join(".ai-config");
        let request = InventoryRequest {
            canonical_layers: vec![
                CanonicalLayerRoot {
                    layer: SourceLayer::Global,
                    asset_root: global.clone(),
                },
                CanonicalLayerRoot {
                    layer: SourceLayer::Workspace,
                    asset_root: workspace_assets.clone(),
                },
            ],
            deploy_base: workspace.clone(),
            scope: InventoryScope::Workspace,
        };

        write(
            &global.join("commands/shared.md"),
            &format!("global shared {COMMAND_BODY_SENTINEL}\n"),
        );
        write(
            &global.join("commands/global-only.md"),
            &format!("global only {COMMAND_BODY_SENTINEL}\n"),
        );
        write(
            &workspace_assets.join("commands/shared.md"),
            &format!("workspace shared {COMMAND_BODY_SENTINEL}\n"),
        );
        write(
            &workspace_assets.join("commands/workspace-only.md"),
            &format!("workspace only {COMMAND_BODY_SENTINEL}\n"),
        );

        write(
            &workspace.join(".cursor/commands/shared.md"),
            &format!("workspace shared {COMMAND_BODY_SENTINEL}\n"),
        );
        write(
            &workspace.join(".claude/commands/global-only.md"),
            &format!("global only {COMMAND_BODY_SENTINEL}\n"),
        );
        write(
            &workspace.join(".claude/commands/workspace-only.md"),
            &format!("workspace only {COMMAND_BODY_SENTINEL}\n"),
        );

        for path in [
            home.join(".cursor/commands/home-only.md"),
            home.join(".claude/commands/home-only.md"),
            home.join(".codex/prompts/home-legacy.md"),
            home.join(".codex/commands/forbidden.md"),
            home.join(".hermes/commands/forbidden.md"),
            workspace.join(".codex/prompts/workspace-legacy.md"),
            workspace.join(".codex/commands/forbidden.md"),
            workspace.join(".hermes/commands/forbidden.md"),
        ] {
            write(&path, HOME_PLATFORM_SENTINEL);
        }

        let home_before = path_content_digest(&home).unwrap();
        let workspace_before = path_content_digest(&workspace).unwrap();
        let first = inventory(&request).unwrap();
        let second = inventory(&request).unwrap();
        assert_eq!(first.plan_digest, second.plan_digest);
        assert_eq!(path_content_digest(&home).unwrap(), home_before);
        assert_eq!(path_content_digest(&workspace).unwrap(), workspace_before);

        let command_entry = |path: &Utf8Path| {
            first
                .entries
                .iter()
                .find(|entry| entry.kind == AssetKind::Command && entry.path == path)
                .unwrap_or_else(|| panic!("missing workspace Command entry: {path}"))
        };
        let cursor = command_entry(&workspace.join(".cursor/commands/shared.md"));
        assert_eq!(cursor.classification, InventoryClassification::Equivalent);
        assert_eq!(cursor.provenance, InventoryProvenance::PlatformCurrent);
        assert_eq!(cursor.source_layer, Some(SourceLayer::Workspace));
        assert_eq!(
            cursor.canonical_path,
            Some(workspace_assets.join("commands/shared.md"))
        );
        assert_eq!(cursor.consumers, vec![PlatformId::Cursor]);
        assert_eq!(cursor.scope, InventoryScope::Workspace);
        assert_eq!(cursor.format.as_deref(), Some("markdown"));
        assert_eq!(cursor.trust_requirement, TrustRequirement::None);
        assert!(cursor.currently_consumed);
        assert!(!cursor.owned && cursor.selectable && !cursor.followed);

        let inherited = command_entry(&workspace.join(".claude/commands/global-only.md"));
        assert_eq!(
            inherited.classification,
            InventoryClassification::Equivalent
        );
        assert_eq!(inherited.source_layer, Some(SourceLayer::Global));
        assert_eq!(
            inherited.canonical_path,
            Some(global.join("commands/global-only.md"))
        );
        assert_eq!(inherited.consumers, vec![PlatformId::Claude]);
        assert_eq!(inherited.scope, InventoryScope::Workspace);
        assert_eq!(inherited.format.as_deref(), Some("markdown"));

        let workspace_only = command_entry(&workspace.join(".claude/commands/workspace-only.md"));
        assert_eq!(workspace_only.source_layer, Some(SourceLayer::Workspace));
        assert_eq!(
            workspace_only.canonical_path,
            Some(workspace_assets.join("commands/workspace-only.md"))
        );

        let raw_shared = first
            .entries
            .iter()
            .filter(|entry| {
                entry.kind == AssetKind::Command
                    && entry.name == "shared"
                    && entry.provenance == InventoryProvenance::Canonical
            })
            .collect::<Vec<_>>();
        assert_eq!(raw_shared.len(), 2);
        assert_eq!(
            raw_shared
                .iter()
                .map(|entry| entry.source_layer.unwrap())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([SourceLayer::Global, SourceLayer::Workspace])
        );

        for (name, layer, canonical_path) in [
            (
                "shared",
                SourceLayer::Workspace,
                workspace_assets.join("commands/shared.md"),
            ),
            (
                "global-only",
                SourceLayer::Global,
                global.join("commands/global-only.md"),
            ),
            (
                "workspace-only",
                SourceLayer::Workspace,
                workspace_assets.join("commands/workspace-only.md"),
            ),
        ] {
            for (platform, reason_code) in [
                (PlatformId::Codex, "codex_command_unsupported"),
                (PlatformId::Hermes, "hermes_command_unsupported"),
            ] {
                let unsupported = first
                    .unsupported
                    .iter()
                    .find(|entry| {
                        entry.kind == AssetKind::Command
                            && entry.platform == platform
                            && entry.name == name
                            && entry.scope == InventoryScope::Workspace
                    })
                    .unwrap_or_else(|| {
                        panic!("missing workspace Command unsupported row: {platform:?}/{name}")
                    });
                assert_eq!(unsupported.reason_code, reason_code);
                assert_eq!(unsupported.source_layer, Some(layer));
                assert_eq!(unsupported.canonical_path, Some(canonical_path.clone()));
            }
        }

        let forbidden_platform_roots = [
            home.join(".cursor/commands"),
            home.join(".claude/commands"),
            home.join(".codex/prompts"),
            home.join(".codex/commands"),
            home.join(".hermes/commands"),
            workspace.join(".codex/prompts"),
            workspace.join(".codex/commands"),
            workspace.join(".hermes/commands"),
        ];
        assert!(first.entries.iter().all(|entry| {
            !forbidden_platform_roots
                .iter()
                .any(|root| entry.path.starts_with(root))
        }));
        assert!(first.issues.iter().all(|issue| {
            !forbidden_platform_roots
                .iter()
                .any(|root| issue.path.starts_with(root))
        }));

        let serialized = serde_json::to_string(&first).unwrap();
        assert!(!serialized.contains(COMMAND_BODY_SENTINEL));
        assert!(!serialized.contains(HOME_PLATFORM_SENTINEL));
        assert!(!serialized.contains("home-only"));
        assert!(!serialized.contains("home-legacy"));
        assert!(!serialized.contains("workspace-legacy"));
        assert!(!serialized.contains(".codex/commands"));
        assert!(!serialized.contains(".hermes/commands"));
    }

    #[test]
    fn hook_halves_split_across_layers_are_both_cross_layer_conflicts() {
        let temp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let global = root.join("home/.ai-config");
        let workspace = root.join("workspace/.ai-config");
        let request = InventoryRequest {
            canonical_layers: vec![
                CanonicalLayerRoot {
                    layer: SourceLayer::Global,
                    asset_root: global.clone(),
                },
                CanonicalLayerRoot {
                    layer: SourceLayer::Workspace,
                    asset_root: workspace.clone(),
                },
            ],
            deploy_base: root.join("workspace"),
            scope: InventoryScope::Workspace,
        };
        write(
            &global.join("hooks.json"),
            r#"{"version":1,"hooks":{"afterShellExecution":[{"command":"./hooks/split.sh"}]}}"#,
        );
        write(&workspace.join("hooks/split.sh"), "#!/bin/sh\nexit 0\n");

        let report = inventory(&request).unwrap();
        let findings = report
            .entries
            .iter()
            .filter(|entry| {
                entry.kind == AssetKind::Hook
                    && entry.name == "split.sh"
                    && entry.provenance == InventoryProvenance::Canonical
            })
            .collect::<Vec<_>>();
        assert_eq!(
            findings.len(),
            2,
            "both layer-local halves must remain visible"
        );
        assert_eq!(
            findings
                .iter()
                .filter_map(|entry| entry.source_layer)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([SourceLayer::Global, SourceLayer::Workspace])
        );
        assert!(
            findings.iter().all(|entry| {
                entry.reason_code == "cross_layer_hook_pair"
                    && entry.blocking
                    && entry.classification == InventoryClassification::Foreign
                    && entry.ownership_state == InventoryOwnershipState::Foreign
                    && !entry.owned
                    && !entry.selectable
            }),
            "split Hook halves must both be cross-layer conflicts: {findings:?}"
        );
        assert!(!report.entries.iter().any(|entry| {
            entry.kind == AssetKind::Hook
                && entry.name == "split.sh"
                && entry.classification == InventoryClassification::CanonicalSource
        }));
    }

    #[cfg(unix)]
    #[test]
    fn workspace_hook_inventory_uses_complete_same_layer_pairs_and_local_platform_targets() {
        const HOOK_BODY_SENTINEL: &str = "workspace-hook-body-must-not-serialize";
        const HOME_PLATFORM_SENTINEL: &str = "workspace-hook-home-platform-must-not-be-read";
        const AGENT_HOOKS_SENTINEL: &str = "legacy-agent-hooks-must-not-be-read";
        const RENDERER_SENTINEL: &str = "hook-renderer-must-not-run-during-inventory";

        let temp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let home = root.join("home");
        let workspace = root.join("workspace");
        let global = home.join(".ai-config");
        let workspace_assets = workspace.join(".ai-config");
        let request = InventoryRequest {
            canonical_layers: vec![
                CanonicalLayerRoot {
                    layer: SourceLayer::Global,
                    asset_root: global.clone(),
                },
                CanonicalLayerRoot {
                    layer: SourceLayer::Workspace,
                    asset_root: workspace_assets.clone(),
                },
            ],
            deploy_base: workspace.clone(),
            scope: InventoryScope::Workspace,
        };

        write(
            &global.join("hooks.json"),
            &format!(
                r#"{{
  "version": 1,
  "hooks": {{
    "sessionStart": [
      {{
        "command": "./hooks/shared.py {RENDERER_SENTINEL}",
        "matcher": "{HOOK_BODY_SENTINEL}"
      }},
      {{
        "command": "./hooks/global-only.py {RENDERER_SENTINEL}",
        "matcher": "{HOOK_BODY_SENTINEL}"
      }}
    ]
  }}
}}"#
            ),
        );
        write(
            &global.join("hooks/shared.py"),
            &format!("# global shared\n# {HOOK_BODY_SENTINEL}\n"),
        );
        write(
            &global.join("hooks/global-only.py"),
            &format!("# global only\n# {HOOK_BODY_SENTINEL}\n"),
        );
        write(
            &workspace_assets.join("hooks.json"),
            &format!(
                r#"{{
  "version": 1,
  "hooks": {{
    "beforeShellExecution": [
      {{
        "command": "./hooks/shared.py {RENDERER_SENTINEL}",
        "matcher": "{HOOK_BODY_SENTINEL}"
      }},
      {{
        "command": "./hooks/workspace-only.py {RENDERER_SENTINEL}",
        "matcher": "{HOOK_BODY_SENTINEL}"
      }}
    ]
  }}
}}"#
            ),
        );
        write(
            &workspace_assets.join("hooks/shared.py"),
            &format!("# workspace shared\n# {HOOK_BODY_SENTINEL}\n"),
        );
        write(
            &workspace_assets.join("hooks/workspace-only.py"),
            &format!("# workspace only\n# {HOOK_BODY_SENTINEL}\n"),
        );

        let effective_sources = [
            ("shared.py", workspace_assets.join("hooks/shared.py")),
            ("global-only.py", global.join("hooks/global-only.py")),
            (
                "workspace-only.py",
                workspace_assets.join("hooks/workspace-only.py"),
            ),
        ];
        for (directory, config) in [
            (".cursor/hooks", workspace.join(".cursor/hooks.json")),
            (".codex/hooks", workspace.join(".codex/hooks.json")),
            (".claude/hooks", workspace.join(".claude/settings.json")),
        ] {
            let hooks = effective_sources
                .iter()
                .map(|(name, _)| {
                    serde_json::json!({
                        "command": format!("{directory}/{name}"),
                        "managedBy": "untrusted-inventory-marker",
                        "hook": name,
                    })
                })
                .collect::<Vec<_>>();
            let hooks = if directory == ".cursor/hooks" {
                serde_json::json!({
                    "afterShellExecution": hooks,
                })
            } else {
                let groups = hooks
                    .into_iter()
                    .map(|hook| serde_json::json!({ "hooks": [hook] }))
                    .collect::<Vec<_>>();
                serde_json::json!({
                    "PostToolUse": groups,
                })
            };
            let document = serde_json::json!({
                "hooks": hooks,
                "rendererSentinel": RENDERER_SENTINEL,
                "permissions": {
                    "private": HOOK_BODY_SENTINEL,
                },
            });
            write(&config, &serde_json::to_string_pretty(&document).unwrap());
            for (name, source) in &effective_sources {
                let target = workspace.join(directory).join(name);
                fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
                std::os::unix::fs::symlink(source.as_std_path(), target.as_std_path()).unwrap();
            }
        }

        for path in [
            home.join(".cursor/hooks.json"),
            home.join(".codex/hooks.json"),
            home.join(".codex/config.toml"),
            home.join(".claude/settings.json"),
            home.join(".hermes/config.yaml"),
        ] {
            write(&path, HOME_PLATFORM_SENTINEL);
        }
        write(
            &home.join(".hermes/agent-hooks/home-only.sh"),
            AGENT_HOOKS_SENTINEL,
        );
        write(
            &workspace.join(".hermes/agent-hooks/workspace-only.sh"),
            AGENT_HOOKS_SENTINEL,
        );

        let home_before = path_content_digest(&home).unwrap();
        let workspace_before = path_content_digest(&workspace).unwrap();
        let first = inventory(&request).unwrap();
        let second = inventory(&request).unwrap();
        assert_eq!(first.plan_digest, second.plan_digest);
        assert_eq!(first.entries, second.entries);
        assert_eq!(first.unsupported, second.unsupported);
        assert_eq!(first.issues, second.issues);
        assert_eq!(path_content_digest(&home).unwrap(), home_before);
        assert_eq!(path_content_digest(&workspace).unwrap(), workspace_before);

        let raw_shared = first
            .entries
            .iter()
            .filter(|entry| {
                entry.kind == AssetKind::Hook
                    && entry.name == "shared.py"
                    && entry.provenance == InventoryProvenance::Canonical
            })
            .collect::<Vec<_>>();
        assert_eq!(
            raw_shared.len(),
            2,
            "both complete same-layer Hook pairs remain visible as raw canonical rows"
        );
        assert_eq!(
            raw_shared
                .iter()
                .map(|entry| entry.source_layer.unwrap())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([SourceLayer::Global, SourceLayer::Workspace])
        );

        let component = |path: &Utf8Path, entry_key: Option<&str>| {
            first
                .entries
                .iter()
                .find(|entry| {
                    entry.kind == AssetKind::Hook
                        && entry.path == path
                        && entry.entry_key.as_deref() == entry_key
                })
                .unwrap_or_else(|| {
                    panic!("missing workspace Hook component: {path}#{:?}", entry_key)
                })
        };
        for (name, source) in &effective_sources {
            let expected_layer = if *name == "global-only.py" {
                SourceLayer::Global
            } else {
                SourceLayer::Workspace
            };
            for (platform, config, directory, format, trust) in [
                (
                    PlatformId::Cursor,
                    workspace.join(".cursor/hooks.json"),
                    ".cursor/hooks",
                    "json",
                    TrustRequirement::None,
                ),
                (
                    PlatformId::Codex,
                    workspace.join(".codex/hooks.json"),
                    ".codex/hooks",
                    "json",
                    TrustRequirement::TrustedProjectWithIndependentReview,
                ),
                (
                    PlatformId::Claude,
                    workspace.join(".claude/settings.json"),
                    ".claude/hooks",
                    "json",
                    TrustRequirement::None,
                ),
            ] {
                let binding = component(&config, Some(&format!("hooks.{name}")));
                assert_eq!(binding.name, *name);
                assert_eq!(binding.provenance, InventoryProvenance::PlatformCurrent);
                assert_eq!(binding.source_layer, Some(expected_layer));
                assert_eq!(binding.canonical_path, Some(source.clone()));
                assert_eq!(binding.consumers, vec![platform]);
                assert_eq!(binding.scope, InventoryScope::Workspace);
                assert_eq!(binding.format.as_deref(), Some(format));
                assert_eq!(binding.trust_requirement, trust);
                assert!(binding.currently_consumed);
                assert_eq!(binding.ownership_state, InventoryOwnershipState::Foreign);
                assert!(!binding.owned && !binding.selectable && !binding.followed);

                let script = component(&workspace.join(directory).join(name), None);
                assert_eq!(script.name, *name);
                assert_eq!(script.classification, InventoryClassification::ManagedLink);
                assert_eq!(script.source_layer, Some(expected_layer));
                assert_eq!(script.canonical_path, Some(source.clone()));
                assert_eq!(script.consumers, vec![platform]);
                assert_eq!(script.scope, InventoryScope::Workspace);
                assert_eq!(script.format.as_deref(), Some("file"));
                assert_eq!(script.trust_requirement, trust);
                assert!(script.currently_consumed);
                assert!(script.owned && !script.selectable && !script.followed);
            }

            let unsupported = first
                .unsupported
                .iter()
                .find(|entry| {
                    entry.kind == AssetKind::Hook
                        && entry.platform == PlatformId::Hermes
                        && entry.name == *name
                        && entry.scope == InventoryScope::Workspace
                })
                .unwrap_or_else(|| panic!("missing Hermes workspace Hook unsupported row: {name}"));
            assert_eq!(unsupported.reason_code, "hermes_workspace_hook_unsupported");
            assert_eq!(unsupported.source_layer, Some(expected_layer));
            assert_eq!(unsupported.canonical_path, Some(source.clone()));
        }

        let serialized = serde_json::to_string(&first).unwrap();
        assert!(!serialized.contains(HOOK_BODY_SENTINEL));
        assert!(!serialized.contains(HOME_PLATFORM_SENTINEL));
        assert!(!serialized.contains(AGENT_HOOKS_SENTINEL));
        assert!(!serialized.contains(RENDERER_SENTINEL));
        assert!(!serialized.contains("agent-hooks"));
        assert!(!serialized.contains("home-only"));
        assert!(first.entries.iter().all(|entry| {
            !entry.path.starts_with(home.join(".cursor"))
                && !entry.path.starts_with(home.join(".codex"))
                && !entry.path.starts_with(home.join(".claude"))
                && !entry.path.starts_with(home.join(".hermes"))
                && !entry.path.starts_with(workspace.join(".hermes"))
        }));
        assert!(first.issues.iter().all(|issue| {
            !issue.path.starts_with(home.join(".cursor"))
                && !issue.path.starts_with(home.join(".codex"))
                && !issue.path.starts_with(home.join(".claude"))
                && !issue.path.starts_with(home.join(".hermes"))
                && !issue.path.starts_with(workspace.join(".hermes"))
        }));
    }
}
