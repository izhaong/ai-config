//! Explicit platform-to-canonical import planning and durable rollback.
//!
//! Plans contain paths, fingerprints, reason codes and secret key names only. Platform bodies
//! are re-read from caller-approved roots at apply time and never serialized into a plan or
//! transaction manifest.

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::{Builder, NamedTempFile};

use crate::error::CoreError;
use crate::model::{AssetKind, PlatformId};
use crate::paths::{self, SyncRoots};
use crate::platform;
use crate::projection::fingerprint::{path_content_digest, path_fingerprint};
use crate::projection::mcp::entry_fingerprint::codex_mcp_entry;
use crate::projection::mcp::source::load_mcp_definition_at;
use crate::projection::model::{FingerprintType, PathFingerprint, SourceLayer};

pub const IMPORT_PLAN_SCHEMA_VERSION: u16 = 1;
const IMPORT_TRANSACTION_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRequest {
    pub kind: AssetKind,
    pub name: String,
    pub source_platform: PlatformId,
    pub source_path: Utf8PathBuf,
    pub approved_source_root: Utf8PathBuf,
    pub destination_layer: SourceLayer,
    pub destination_asset_root: Utf8PathBuf,
    pub replace: bool,
}

/// Resolves one explicit platform item into the canonical source layer for the supplied scope.
/// It is pure path selection: callers still must build a reviewed plan and bind apply to its
/// digest plus action ID before any source file can be written.
pub fn import_request_for_sync_roots(
    kind: AssetKind,
    name: &str,
    source_platform: PlatformId,
    roots: &SyncRoots,
    replace: bool,
) -> Result<ImportRequest, CoreError> {
    let (destination_layer, destination_asset_root) =
        if paths::is_project_deploy_base(&roots.deploy_base) {
            (SourceLayer::Project, roots.asset_root.clone())
        } else {
            (SourceLayer::Global, roots.asset_root.clone())
        };
    let (source_path, approved_source_root) = import_source_location(
        kind,
        name,
        source_platform,
        &roots.deploy_base,
        &roots.asset_root,
        destination_layer,
    )?;
    Ok(ImportRequest {
        kind,
        name: name.to_owned(),
        source_platform,
        source_path,
        approved_source_root,
        destination_layer,
        destination_asset_root,
        replace,
    })
}

fn import_source_location(
    kind: AssetKind,
    name: &str,
    source_platform: PlatformId,
    deploy_base: &Utf8Path,
    asset_root: &Utf8Path,
    destination_layer: SourceLayer,
) -> Result<(Utf8PathBuf, Utf8PathBuf), CoreError> {
    if kind == AssetKind::Prompt {
        if source_platform != PlatformId::Codex || destination_layer != SourceLayer::Project {
            return Err(CoreError::NotImplemented(
                "prompt import currently supports only codex AGENTS into a project source layer",
            ));
        }
        return Ok((deploy_base.join("AGENTS.md"), deploy_base.to_path_buf()));
    }
    if kind == AssetKind::Mcp && !matches!(source_platform, PlatformId::Cursor | PlatformId::Codex)
    {
        return Err(CoreError::NotImplemented(
            "MCP import currently supports only Cursor JSON or Codex TOML containers",
        ));
    }
    let platform_path = if kind == AssetKind::Mcp && source_platform == PlatformId::Codex {
        deploy_base.join(".codex/config.toml")
    } else {
        platform::kind_asset_path(source_platform, kind, deploy_base, asset_root).ok_or(
            CoreError::NotImplemented(
                "this platform and asset kind do not have a lossless import mapping",
            ),
        )?
    };
    if kind == AssetKind::Mcp {
        let approved_root = platform_path.parent().ok_or_else(|| {
            CoreError::InvalidPath("platform MCP path has no approved parent".to_owned())
        })?;
        let approved_root = approved_root.to_path_buf();
        return Ok((platform_path, approved_root));
    }
    let source_path = match kind {
        AssetKind::Skill => platform_path.join(name),
        AssetKind::Rule => {
            let extension = match source_platform {
                PlatformId::Cursor | PlatformId::Hermes => "mdc",
                PlatformId::Claude => "md",
                PlatformId::AgentsManager | PlatformId::Codex => {
                    return Err(CoreError::NotImplemented(
                        "this platform rule format has no lossless import mapping",
                    ))
                }
            };
            platform_path.join(format!("{name}.{extension}"))
        }
        AssetKind::Agent | AssetKind::Command => platform_path.join(format!("{name}.md")),
        AssetKind::Mcp | AssetKind::Prompt | AssetKind::Hook => unreachable!("handled above"),
    };
    Ok((source_path, platform_path))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportActionKind {
    CreateSource,
    ReplaceSource,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportSecretStatus {
    Clear,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportSecretPreflight {
    pub status: ImportSecretStatus,
    pub key_names: Vec<String>,
    pub reason_codes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportNormalizedDiff {
    pub change: String,
    pub source_digest: String,
    pub destination_digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportAction {
    pub action_id: String,
    pub kind: AssetKind,
    pub name: String,
    pub source_platform: PlatformId,
    pub source_path: Utf8PathBuf,
    pub approved_source_root: Utf8PathBuf,
    pub source_precondition: PathFingerprint,
    pub destination_layer: SourceLayer,
    pub destination_asset_root: Utf8PathBuf,
    pub destination_path: Utf8PathBuf,
    pub destination_precondition: PathFingerprint,
    pub action: ImportActionKind,
    pub normalized_diff: ImportNormalizedDiff,
    pub secret_preflight: ImportSecretPreflight,
    pub blocking_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportPlan {
    pub schema_version: u16,
    pub plan_digest: String,
    pub actions: Vec<ImportAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportApplyOptions {
    expected_plan_digest: String,
    selected_action_ids: BTreeSet<String>,
}

impl ImportApplyOptions {
    pub fn new(
        expected_plan_digest: impl Into<String>,
        selected_action_ids: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            expected_plan_digest: expected_plan_digest.into(),
            selected_action_ids: selected_action_ids.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportApplyReport {
    pub transaction_id: Option<String>,
    pub applied: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RollbackStatus {
    RolledBack,
    Drifted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportRollbackReport {
    pub transaction_id: String,
    pub status: RollbackStatus,
    pub restored: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImportTransactionManifest {
    schema_version: u16,
    transaction_id: String,
    plan_digest: String,
    status: TransactionStatus,
    actions: Vec<ImportTransactionAction>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TransactionStatus {
    Prepared,
    Applying,
    Applied,
    RolledBack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImportTransactionAction {
    action_id: String,
    kind: AssetKind,
    destination_asset_root: Utf8PathBuf,
    destination_path: Utf8PathBuf,
    before: PathFingerprint,
    after: PathFingerprint,
    backup_path: Option<Utf8PathBuf>,
    applied: bool,
}

pub fn build_import_plan(requests: &[ImportRequest]) -> Result<ImportPlan, CoreError> {
    let mut requests = requests.to_vec();
    requests.sort_by(|left, right| {
        destination_path(left)
            .cmp(&destination_path(right))
            .then_with(|| left.source_path.cmp(&right.source_path))
    });
    let mut destinations = BTreeSet::new();
    let mut actions = Vec::new();
    for request in requests {
        validate_request(&request)?;
        let destination_path = destination_path(&request);
        if !destinations.insert(destination_path.clone()) {
            return Err(CoreError::InvalidPath(
                "import plan contains duplicate canonical destinations".to_owned(),
            ));
        }
        let source_precondition = import_fingerprint(request.kind, &request.source_path)?;
        validate_source_shape(request.kind, &source_precondition)?;
        let destination_precondition = import_fingerprint(request.kind, &destination_path)?;
        let prepared = prepare_source(&request)?;
        let destination_digest = if destination_precondition.entry_type == FingerprintType::Missing
        {
            None
        } else {
            Some(import_path_digest(request.kind, &destination_path)?)
        };
        if destination_digest.as_deref() == Some(prepared.digest.as_str()) {
            continue;
        }
        let action = if destination_precondition.entry_type == FingerprintType::Missing {
            ImportActionKind::CreateSource
        } else if request.replace {
            ImportActionKind::ReplaceSource
        } else {
            return Err(CoreError::InvalidPath(format!(
                "canonical source already exists with different content: {destination_path}"
            )));
        };
        let change = match action {
            ImportActionKind::CreateSource => "create",
            ImportActionKind::ReplaceSource => "replace",
        };
        let mut planned = ImportAction {
            action_id: String::new(),
            kind: request.kind,
            name: request.name,
            source_platform: request.source_platform,
            source_path: request.source_path,
            approved_source_root: request.approved_source_root,
            source_precondition,
            destination_layer: request.destination_layer,
            destination_asset_root: request.destination_asset_root,
            destination_path,
            destination_precondition: destination_precondition.clone(),
            action,
            normalized_diff: ImportNormalizedDiff {
                change: change.to_owned(),
                source_digest: prepared.digest,
                destination_digest,
            },
            secret_preflight: prepared.secret_preflight,
            blocking_reasons: prepared.blocking_reasons,
        };
        planned.action_id = stable_digest(&planned)?;
        actions.push(planned);
    }
    let digest_input = json!({
        "schema_version": IMPORT_PLAN_SCHEMA_VERSION,
        "actions": actions,
    });
    Ok(ImportPlan {
        schema_version: IMPORT_PLAN_SCHEMA_VERSION,
        plan_digest: stable_digest(&digest_input)?,
        actions,
    })
}

pub fn apply_import_plan(
    plan: &ImportPlan,
    options: &ImportApplyOptions,
    transaction_root: &Utf8Path,
) -> Result<ImportApplyReport, CoreError> {
    validate_plan(plan)?;
    if options.expected_plan_digest != plan.plan_digest {
        return Err(CoreError::InvalidPath(
            "import plan digest changed after review".to_owned(),
        ));
    }
    if options.selected_action_ids.is_empty() {
        return Err(CoreError::InvalidPath(
            "import apply requires explicit action IDs".to_owned(),
        ));
    }
    for selected in &options.selected_action_ids {
        if !plan
            .actions
            .iter()
            .any(|action| &action.action_id == selected)
        {
            return Err(CoreError::InvalidPath(
                "selected import action ID is not part of this plan".to_owned(),
            ));
        }
    }
    if plan.actions.iter().any(|action| {
        action.secret_preflight.status == ImportSecretStatus::Blocked
            || !action.blocking_reasons.is_empty()
    }) {
        return Err(CoreError::InvalidPath(
            "import plan contains a blocking action; no selected action was applied".to_owned(),
        ));
    }
    ensure_private_directory(transaction_root)?;
    let _lock = ImportApplyLock::acquire(transaction_root)?;
    let selected = plan
        .actions
        .iter()
        .filter(|action| options.selected_action_ids.contains(&action.action_id))
        .collect::<Vec<_>>();
    let frozen = selected
        .into_iter()
        .map(|action| Ok((action, preflight_action(action)?)))
        .collect::<Result<Vec<_>, CoreError>>()?;

    let mut manifest_file = Builder::new()
        .prefix("migration-")
        .suffix(".json")
        .tempfile_in(transaction_root.as_std_path())
        .map_err(CoreError::Io)?;
    let manifest_path = Utf8PathBuf::from_path_buf(manifest_file.path().to_path_buf())
        .map_err(|path| CoreError::InvalidPath(path.to_string_lossy().into_owned()))?;
    let transaction_id = manifest_path
        .file_stem()
        .ok_or_else(|| CoreError::InvalidPath("transaction manifest has no ID".to_owned()))?
        .to_owned();
    let backup_root = transaction_root.join(format!("{transaction_id}.d"));
    fs::create_dir(backup_root.as_std_path())?;
    set_mode(&backup_root, 0o700)?;
    sync_directory(transaction_root)?;
    let mut manifest = ImportTransactionManifest {
        schema_version: IMPORT_TRANSACTION_SCHEMA_VERSION,
        transaction_id: transaction_id.clone(),
        plan_digest: plan.plan_digest.clone(),
        status: TransactionStatus::Prepared,
        actions: frozen
            .iter()
            .enumerate()
            .map(|(index, (action, source))| ImportTransactionAction {
                action_id: action.action_id.clone(),
                kind: action.kind,
                destination_asset_root: action.destination_asset_root.clone(),
                destination_path: action.destination_path.clone(),
                before: action.destination_precondition.clone(),
                after: source.expected_fingerprint(),
                backup_path: (action.action == ImportActionKind::ReplaceSource)
                    .then(|| backup_root.join(format!("{index}.backup"))),
                applied: false,
            })
            .collect(),
    };
    write_manifest_handle(&mut manifest_file, &manifest)?;
    set_mode(&manifest_path, 0o600)?;
    let (_file, kept_path) = manifest_file
        .keep()
        .map_err(|error| CoreError::Io(error.error))?;
    let manifest_path = Utf8PathBuf::from_path_buf(kept_path)
        .map_err(|path| CoreError::InvalidPath(path.to_string_lossy().into_owned()))?;
    sync_directory(transaction_root)?;

    for (index, (action, source)) in frozen.into_iter().enumerate() {
        manifest.status = TransactionStatus::Applying;
        write_manifest_path(&manifest_path, &manifest)?;
        if let Err(error) = apply_one_import(action, source, &manifest.actions[index]) {
            let rollback = rollback_recoverable_actions(&manifest.actions);
            if rollback.is_ok() {
                manifest.status = TransactionStatus::RolledBack;
                let _ = write_manifest_path(&manifest_path, &manifest);
                return Err(error);
            }
            return Err(CoreError::ProjectionLedger(format!(
                "import apply failed and rollback could not safely restore every action: {error}"
            )));
        }
        manifest.actions[index].applied = true;
        if let Err(error) = write_manifest_path(&manifest_path, &manifest) {
            if rollback_recoverable_actions(&manifest.actions).is_ok() {
                manifest.status = TransactionStatus::RolledBack;
                let _ = write_manifest_path(&manifest_path, &manifest);
                return Err(error);
            }
            return Err(CoreError::ProjectionLedger(
                "import manifest update failed and rollback could not safely restore every action"
                    .to_owned(),
            ));
        }
    }
    manifest.status = TransactionStatus::Applied;
    if let Err(error) = write_manifest_path(&manifest_path, &manifest) {
        let _ = rollback_recoverable_actions(&manifest.actions);
        return Err(error);
    }
    Ok(ImportApplyReport {
        transaction_id: Some(transaction_id),
        applied: manifest
            .actions
            .iter()
            .filter(|action| action.applied)
            .count(),
        skipped: plan.actions.len()
            - manifest
                .actions
                .iter()
                .filter(|action| action.applied)
                .count(),
    })
}

pub fn rollback_import_transaction(
    transaction_root: &Utf8Path,
    transaction_id: &str,
    approved_destination_roots: &[Utf8PathBuf],
) -> Result<ImportRollbackReport, CoreError> {
    if !safe_name(transaction_id) {
        return Err(CoreError::InvalidPath(
            "invalid migration transaction ID".to_owned(),
        ));
    }
    validate_private_directory(transaction_root)?;
    let _lock = ImportApplyLock::acquire(transaction_root)?;
    let manifest_path = transaction_root.join(format!("{transaction_id}.json"));
    let metadata = fs::symlink_metadata(manifest_path.as_std_path())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CoreError::InvalidPath(
            "migration transaction manifest is not a regular file".to_owned(),
        ));
    }
    let mut manifest: ImportTransactionManifest =
        serde_json::from_slice(&fs::read(manifest_path.as_std_path())?)?;
    if manifest.schema_version != IMPORT_TRANSACTION_SCHEMA_VERSION
        || manifest.transaction_id != transaction_id
        || manifest.status == TransactionStatus::RolledBack
    {
        return Err(CoreError::InvalidPath(
            "migration transaction is not rollback-ready".to_owned(),
        ));
    }
    for action in manifest.actions.iter().rev() {
        validate_manifest_action(
            action,
            transaction_root,
            transaction_id,
            approved_destination_roots,
        )?;
    }
    let restored = match rollback_recoverable_actions(&manifest.actions) {
        Ok(restored) => restored,
        Err(CoreError::InvalidPath(_)) => {
            return Ok(ImportRollbackReport {
                transaction_id: transaction_id.to_owned(),
                status: RollbackStatus::Drifted,
                restored: 0,
            })
        }
        Err(error) => return Err(error),
    };
    manifest.status = TransactionStatus::RolledBack;
    write_manifest_path(&manifest_path, &manifest)?;
    Ok(ImportRollbackReport {
        transaction_id: transaction_id.to_owned(),
        status: RollbackStatus::RolledBack,
        restored,
    })
}

struct PreparedSource {
    digest: String,
    secret_preflight: ImportSecretPreflight,
    blocking_reasons: Vec<String>,
}

struct FrozenSource {
    digest: String,
    secret_preflight: ImportSecretPreflight,
    blocking_reasons: Vec<String>,
    content: FrozenContent,
}

enum FrozenContent {
    File {
        bytes: Vec<u8>,
        mode: Option<u32>,
    },
    Directory {
        root_mode: Option<u32>,
        entries: Vec<FrozenTreeEntry>,
    },
}

struct FrozenTreeEntry {
    relative_path: Utf8PathBuf,
    directory: bool,
    bytes: Vec<u8>,
    mode: Option<u32>,
}

struct ImportApplyLock {
    path: Utf8PathBuf,
    _file: fs::File,
}

impl ImportApplyLock {
    fn acquire(transaction_root: &Utf8Path) -> Result<Self, CoreError> {
        validate_private_directory(transaction_root)?;
        let path = transaction_root.join(".agents-manager-import.lock");
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.as_std_path())
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    CoreError::InvalidPath(
                        "another import apply is active or its lock requires explicit inspection"
                            .to_owned(),
                    )
                } else {
                    CoreError::Io(error)
                }
            })?;
        Ok(Self { path, _file: file })
    }
}

impl Drop for ImportApplyLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.path.as_std_path());
    }
}

impl FrozenSource {
    fn expected_fingerprint(&self) -> PathFingerprint {
        match &self.content {
            FrozenContent::File { mode, .. } => PathFingerprint {
                entry_type: FingerprintType::File,
                digest: Some(self.digest.clone()),
                link_target: None,
                mode: *mode,
            },
            FrozenContent::Directory { root_mode, .. } => PathFingerprint {
                entry_type: FingerprintType::Directory,
                digest: Some(self.digest.clone()),
                link_target: None,
                mode: *root_mode,
            },
        }
    }
}

fn prepare_source(request: &ImportRequest) -> Result<PreparedSource, CoreError> {
    let frozen = freeze_source(request)?;
    Ok(PreparedSource {
        digest: frozen.digest,
        secret_preflight: frozen.secret_preflight,
        blocking_reasons: frozen.blocking_reasons,
    })
}

fn freeze_source(request: &ImportRequest) -> Result<FrozenSource, CoreError> {
    let blocking_reasons = format_blocking_reasons(request);
    let clear = ImportSecretPreflight {
        status: ImportSecretStatus::Clear,
        key_names: Vec::new(),
        reason_codes: Vec::new(),
    };
    if request.kind == AssetKind::Skill {
        let (root_mode, entries, digest) = freeze_tree(&request.source_path)?;
        return Ok(FrozenSource {
            digest,
            secret_preflight: clear,
            blocking_reasons,
            content: FrozenContent::Directory { root_mode, entries },
        });
    }
    let source_metadata = fs::symlink_metadata(request.source_path.as_std_path())?;
    if source_metadata.file_type().is_symlink() || !source_metadata.is_file() {
        return Err(CoreError::InvalidPath(
            "import source is not a direct regular file".to_owned(),
        ));
    }
    let source_bytes = fs::read(request.source_path.as_std_path())?;
    if request.kind == AssetKind::Mcp {
        let entry = platform_mcp_entry_from_bytes(request, &source_bytes)?;
        let secret_preflight = inspect_mcp_secrets(&entry);
        let canonical = canonical_mcp_value(request, entry);
        let bytes = serde_json::to_vec_pretty(&canonical)?;
        return Ok(FrozenSource {
            digest: file_bytes_digest(&bytes),
            secret_preflight,
            blocking_reasons,
            content: FrozenContent::File {
                bytes,
                mode: path_mode(&source_metadata),
            },
        });
    }
    Ok(FrozenSource {
        digest: file_bytes_digest(&source_bytes),
        secret_preflight: clear,
        blocking_reasons,
        content: FrozenContent::File {
            bytes: source_bytes,
            mode: path_mode(&source_metadata),
        },
    })
}

fn freeze_tree(root: &Utf8Path) -> Result<(Option<u32>, Vec<FrozenTreeEntry>, String), CoreError> {
    let metadata = fs::symlink_metadata(root.as_std_path())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CoreError::InvalidPath(
            "skill import source is not a direct directory".to_owned(),
        ));
    }
    let root_mode = path_mode(&metadata);
    let mut entries = Vec::new();
    freeze_tree_entries(root, root, &mut entries)?;
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let mut hasher = Sha256::new();
    hasher.update(b"tree-with-mode\0");
    hasher.update(root_mode.unwrap_or_default().to_le_bytes());
    for entry in &entries {
        hasher.update(entry.relative_path.as_str().as_bytes());
        hasher.update([0]);
        hasher.update([u8::from(entry.directory)]);
        hasher.update(entry.mode.unwrap_or_default().to_le_bytes());
        hasher.update(&entry.bytes);
        hasher.update([0]);
    }
    Ok((root_mode, entries, hex::encode(hasher.finalize())))
}

fn freeze_tree_entries(
    root: &Utf8Path,
    current: &Utf8Path,
    entries: &mut Vec<FrozenTreeEntry>,
) -> Result<(), CoreError> {
    let mut children = fs::read_dir(current.as_std_path())?.collect::<Result<Vec<_>, _>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let path = Utf8PathBuf::from_path_buf(child.path())
            .map_err(|path| CoreError::InvalidPath(path.to_string_lossy().into_owned()))?;
        let metadata = fs::symlink_metadata(path.as_std_path())?;
        if metadata.file_type().is_symlink() || (!metadata.is_dir() && !metadata.is_file()) {
            return Err(CoreError::InvalidPath(
                "skill import refuses symlinks and non-regular entries".to_owned(),
            ));
        }
        let relative_path = path
            .strip_prefix(root)
            .map_err(|_| CoreError::InvalidPath("skill entry escaped its root".to_owned()))?
            .to_path_buf();
        let directory = metadata.is_dir();
        let bytes = if directory {
            Vec::new()
        } else {
            fs::read(path.as_std_path())?
        };
        entries.push(FrozenTreeEntry {
            relative_path,
            directory,
            bytes,
            mode: path_mode(&metadata),
        });
        if directory {
            freeze_tree_entries(root, &path, entries)?;
        }
    }
    Ok(())
}

fn format_blocking_reasons(request: &ImportRequest) -> Vec<String> {
    match (request.kind, request.source_platform) {
        (AssetKind::Agent, PlatformId::Codex) => {
            vec!["codex_agent_requires_explicit_format_conversion".to_owned()]
        }
        (AssetKind::Hook, _) => vec!["hook_import_requires_compound_asset_conversion".to_owned()],
        _ => Vec::new(),
    }
}

fn preflight_action(action: &ImportAction) -> Result<FrozenSource, CoreError> {
    validate_action_paths(action)?;
    if import_fingerprint(action.kind, &action.source_path)? != action.source_precondition {
        return Err(CoreError::InvalidPath(
            "import source changed after the plan was reviewed".to_owned(),
        ));
    }
    if import_fingerprint(action.kind, &action.destination_path)? != action.destination_precondition
    {
        return Err(CoreError::InvalidPath(
            "canonical destination changed after the plan was reviewed".to_owned(),
        ));
    }
    let request = request_from_action(action);
    let frozen = freeze_source(&request)?;
    if frozen.digest != action.normalized_diff.source_digest
        || frozen.secret_preflight != action.secret_preflight
        || frozen.blocking_reasons != action.blocking_reasons
    {
        return Err(CoreError::InvalidPath(
            "normalized import source changed after review".to_owned(),
        ));
    }
    if frozen.secret_preflight.status == ImportSecretStatus::Blocked {
        return Err(CoreError::InvalidPath(format!(
            "import secret preflight blocked keys: {}",
            frozen.secret_preflight.key_names.join(",")
        )));
    }
    if !frozen.blocking_reasons.is_empty() {
        return Err(CoreError::InvalidPath(format!(
            "import format preflight blocked: {}",
            frozen.blocking_reasons.join(",")
        )));
    }
    Ok(frozen)
}

fn apply_one_import(
    action: &ImportAction,
    source: FrozenSource,
    transaction_action: &ImportTransactionAction,
) -> Result<(), CoreError> {
    ensure_destination_parent(action)?;
    if import_fingerprint(action.kind, &action.destination_path)? != action.destination_precondition
    {
        return Err(CoreError::InvalidPath(
            "canonical destination changed immediately before import apply".to_owned(),
        ));
    }
    let backup_path = if action.action == ImportActionKind::ReplaceSource {
        let backup = transaction_action.backup_path.as_ref().ok_or_else(|| {
            CoreError::InvalidPath("replace import has no planned backup path".to_owned())
        })?;
        if backup.exists() {
            return Err(CoreError::InvalidPath(
                "migration backup path already exists".to_owned(),
            ));
        }
        rename_and_sync(&action.destination_path, backup)?;
        Some(backup.clone())
    } else {
        None
    };
    let write_result = write_canonical_source(source, &action.destination_path);
    if let Err(error) = write_result {
        if let Some(backup) = &backup_path {
            let _ = rename_and_sync(backup, &action.destination_path);
        }
        return Err(error);
    }
    let after = match (|| {
        if action.kind == AssetKind::Mcp {
            load_mcp_definition_at(&action.destination_path)?;
        }
        import_fingerprint(action.kind, &action.destination_path)
    })() {
        Ok(after) => after,
        Err(error) => {
            let _ = remove_exact_path(&action.destination_path);
            if let Some(backup) = &backup_path {
                let _ = rename_and_sync(backup, &action.destination_path);
            }
            return Err(error);
        }
    };
    if after != transaction_action.after {
        let _ = remove_exact_path(&action.destination_path);
        if let Some(backup) = &backup_path {
            let _ = rename_and_sync(backup, &action.destination_path);
        }
        return Err(CoreError::InvalidPath(
            "canonical import post-state did not match the durable intent".to_owned(),
        ));
    }
    Ok(())
}

fn write_canonical_source(source: FrozenSource, destination: &Utf8Path) -> Result<(), CoreError> {
    let FrozenSource { content, .. } = source;
    let FrozenContent::File { bytes, mode } = content else {
        let FrozenContent::Directory { root_mode, entries } = content else {
            unreachable!();
        };
        return copy_directory_atomically(root_mode, entries, destination);
    };
    let parent = destination
        .parent()
        .ok_or_else(|| CoreError::InvalidPath("canonical destination has no parent".to_owned()))?;
    let mut temporary = NamedTempFile::new_in(parent.as_std_path()).map_err(CoreError::Io)?;
    temporary.write_all(&bytes)?;
    set_optional_mode(
        Utf8Path::from_path(temporary.path())
            .ok_or_else(|| CoreError::InvalidPath("temporary path is not UTF-8".to_owned()))?,
        mode,
    )?;
    temporary.as_file_mut().sync_all()?;
    temporary
        .persist_noclobber(destination.as_std_path())
        .map_err(|error| CoreError::Io(error.error))?;
    sync_directory(parent)?;
    Ok(())
}

fn copy_directory_atomically(
    root_mode: Option<u32>,
    entries: Vec<FrozenTreeEntry>,
    destination: &Utf8Path,
) -> Result<(), CoreError> {
    let parent = destination
        .parent()
        .ok_or_else(|| CoreError::InvalidPath("canonical destination has no parent".to_owned()))?;
    let temporary = Builder::new()
        .prefix(".agents-manager-import-")
        .tempdir_in(parent.as_std_path())
        .map_err(CoreError::Io)?;
    let temporary_path = Utf8Path::from_path(temporary.path())
        .ok_or_else(|| CoreError::InvalidPath("temporary path is not UTF-8".to_owned()))?;
    for entry in entries {
        let target = temporary_path.join(&entry.relative_path);
        if entry.directory {
            fs::create_dir(target.as_std_path())?;
        } else {
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target.as_std_path())?;
            output.write_all(&entry.bytes)?;
            output.sync_all()?;
        }
        set_optional_mode(&target, entry.mode)?;
    }
    set_optional_mode(temporary_path, root_mode)?;
    sync_tree_directories(temporary_path)?;
    let kept = Utf8PathBuf::from_path_buf(temporary.keep())
        .map_err(|path| CoreError::InvalidPath(path.to_string_lossy().into_owned()))?;
    rename_and_sync(&kept, destination)?;
    Ok(())
}

fn rollback_recoverable_actions(actions: &[ImportTransactionAction]) -> Result<usize, CoreError> {
    enum RecoveryStep {
        Noop,
        Remove(Utf8PathBuf),
        Restore {
            destination: Utf8PathBuf,
            backup: Utf8PathBuf,
            remove_destination: bool,
        },
    }

    let mut steps = Vec::with_capacity(actions.len());
    for action in actions.iter().rev() {
        let current = import_fingerprint(action.kind, &action.destination_path)?;
        let backup = action
            .backup_path
            .as_ref()
            .map(|path| import_fingerprint(action.kind, path))
            .transpose()?;
        let backup_missing = backup.as_ref().map_or(true, |fingerprint| {
            fingerprint.entry_type == FingerprintType::Missing
        });

        let step = if current == action.before && backup_missing {
            RecoveryStep::Noop
        } else if current == action.after {
            match (&action.backup_path, backup) {
                (None, None) => RecoveryStep::Remove(action.destination_path.clone()),
                (Some(path), Some(fingerprint)) if fingerprint == action.before => {
                    RecoveryStep::Restore {
                        destination: action.destination_path.clone(),
                        backup: path.clone(),
                        remove_destination: true,
                    }
                }
                _ => {
                    return Err(CoreError::InvalidPath(
                        "migration rollback refused a missing or drifted backup".to_owned(),
                    ));
                }
            }
        } else if current.entry_type == FingerprintType::Missing {
            match (&action.backup_path, backup) {
                (Some(path), Some(fingerprint)) if fingerprint == action.before => {
                    RecoveryStep::Restore {
                        destination: action.destination_path.clone(),
                        backup: path.clone(),
                        remove_destination: false,
                    }
                }
                _ => {
                    return Err(CoreError::InvalidPath(
                        "migration rollback refused an incomplete drifted state".to_owned(),
                    ));
                }
            }
        } else {
            return Err(CoreError::InvalidPath(
                "migration rollback refused a drifted post-state".to_owned(),
            ));
        };
        steps.push(step);
    }

    let mut restored = 0;
    for step in steps {
        match step {
            RecoveryStep::Noop => {}
            RecoveryStep::Remove(destination) => {
                remove_exact_path(&destination)?;
                restored += 1;
            }
            RecoveryStep::Restore {
                destination,
                backup,
                remove_destination,
            } => {
                if remove_destination {
                    remove_exact_path(&destination)?;
                }
                rename_and_sync(&backup, &destination)?;
                restored += 1;
            }
        }
    }
    Ok(restored)
}

fn remove_exact_path(path: &Utf8Path) -> Result<(), CoreError> {
    let metadata = fs::symlink_metadata(path.as_std_path())?;
    if metadata.file_type().is_symlink() {
        return Err(CoreError::InvalidPath(
            "rollback refuses a symlink destination".to_owned(),
        ));
    }
    if metadata.is_file() {
        fs::remove_file(path.as_std_path())?;
    } else if metadata.is_dir() {
        remove_tree_no_links(path)?;
    } else {
        return Err(CoreError::InvalidPath(
            "rollback refuses a non-regular destination".to_owned(),
        ));
    }
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

fn rename_and_sync(source: &Utf8Path, destination: &Utf8Path) -> Result<(), CoreError> {
    let source_parent = source
        .parent()
        .ok_or_else(|| CoreError::InvalidPath("rename source has no parent".to_owned()))?;
    let destination_parent = destination
        .parent()
        .ok_or_else(|| CoreError::InvalidPath("rename destination has no parent".to_owned()))?;
    fs::rename(source.as_std_path(), destination.as_std_path())?;
    sync_directory(destination_parent)?;
    if source_parent != destination_parent {
        sync_directory(source_parent)?;
    }
    Ok(())
}

fn sync_directory(path: &Utf8Path) -> Result<(), CoreError> {
    fs::File::open(path.as_std_path())?.sync_all()?;
    Ok(())
}

fn sync_tree_directories(path: &Utf8Path) -> Result<(), CoreError> {
    let mut children = fs::read_dir(path.as_std_path())?.collect::<Result<Vec<_>, _>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let child = Utf8PathBuf::from_path_buf(child.path())
            .map_err(|path| CoreError::InvalidPath(path.to_string_lossy().into_owned()))?;
        let metadata = fs::symlink_metadata(child.as_std_path())?;
        if metadata.file_type().is_symlink() || (!metadata.is_dir() && !metadata.is_file()) {
            return Err(CoreError::InvalidPath(
                "temporary skill tree became unsafe before commit".to_owned(),
            ));
        }
        if metadata.is_dir() {
            sync_tree_directories(&child)?;
        }
    }
    sync_directory(path)
}

fn remove_tree_no_links(path: &Utf8Path) -> Result<(), CoreError> {
    for entry in fs::read_dir(path.as_std_path())? {
        let entry = entry?;
        let child = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|path| CoreError::InvalidPath(path.to_string_lossy().into_owned()))?;
        let metadata = fs::symlink_metadata(child.as_std_path())?;
        if metadata.file_type().is_symlink() {
            return Err(CoreError::InvalidPath(
                "rollback refuses a tree containing symlinks".to_owned(),
            ));
        }
        if metadata.is_dir() {
            remove_tree_no_links(&child)?;
        } else if metadata.is_file() {
            fs::remove_file(child.as_std_path())?;
        } else {
            return Err(CoreError::InvalidPath(
                "rollback refuses a tree containing non-regular entries".to_owned(),
            ));
        }
    }
    fs::remove_dir(path.as_std_path())?;
    Ok(())
}

fn validate_plan(plan: &ImportPlan) -> Result<(), CoreError> {
    if plan.schema_version != IMPORT_PLAN_SCHEMA_VERSION {
        return Err(CoreError::InvalidPath(
            "unsupported import plan schema".to_owned(),
        ));
    }
    let digest_input = json!({
        "schema_version": plan.schema_version,
        "actions": plan.actions,
    });
    if stable_digest(&digest_input)? != plan.plan_digest {
        return Err(CoreError::InvalidPath(
            "import plan digest does not match its actions".to_owned(),
        ));
    }
    for action in &plan.actions {
        let mut without_id = action.clone();
        without_id.action_id.clear();
        if stable_digest(&without_id)? != action.action_id {
            return Err(CoreError::InvalidPath(
                "import action ID does not match its action".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_manifest_action(
    action: &ImportTransactionAction,
    transaction_root: &Utf8Path,
    transaction_id: &str,
    approved_destination_roots: &[Utf8PathBuf],
) -> Result<(), CoreError> {
    if !approved_destination_roots
        .iter()
        .any(|root| root == &action.destination_asset_root)
    {
        return Err(CoreError::InvalidPath(
            "migration manifest destination root is not caller-approved".to_owned(),
        ));
    }
    validate_destination_root(&action.destination_asset_root)?;
    let relative = action
        .destination_path
        .strip_prefix(&action.destination_asset_root)
        .map_err(|_| {
            CoreError::InvalidPath(
                "migration manifest destination escapes its canonical root".to_owned(),
            )
        })?;
    if relative.as_str().is_empty() {
        return Err(CoreError::InvalidPath(
            "migration manifest cannot target the canonical root itself".to_owned(),
        ));
    }
    if let Some(backup) = &action.backup_path {
        let expected_backup_root = transaction_root.join(format!("{transaction_id}.d"));
        if backup.parent() != Some(expected_backup_root.as_path()) {
            return Err(CoreError::InvalidPath(
                "migration manifest backup escapes its transaction root".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_request(request: &ImportRequest) -> Result<(), CoreError> {
    if !safe_name(&request.name) || request.source_platform == PlatformId::AgentsManager {
        return Err(CoreError::InvalidPath(
            "invalid import asset identity".to_owned(),
        ));
    }
    if request.kind == AssetKind::Prompt && request.name != "AGENTS" {
        return Err(CoreError::InvalidPath(
            "only the AGENTS project prompt may be imported".to_owned(),
        ));
    }
    validate_approved_source(request)?;
    validate_destination_root(&request.destination_asset_root)?;
    Ok(())
}

fn validate_action_paths(action: &ImportAction) -> Result<(), CoreError> {
    let request = request_from_action(action);
    validate_request(&request)?;
    if destination_path(&request) != action.destination_path {
        return Err(CoreError::InvalidPath(
            "import destination is not canonical for this asset".to_owned(),
        ));
    }
    Ok(())
}

fn request_from_action(action: &ImportAction) -> ImportRequest {
    ImportRequest {
        kind: action.kind,
        name: action.name.clone(),
        source_platform: action.source_platform,
        source_path: action.source_path.clone(),
        approved_source_root: action.approved_source_root.clone(),
        destination_layer: action.destination_layer,
        destination_asset_root: action.destination_asset_root.clone(),
        replace: action.action == ImportActionKind::ReplaceSource,
    }
}

fn validate_approved_source(request: &ImportRequest) -> Result<(), CoreError> {
    let metadata = fs::symlink_metadata(request.approved_source_root.as_std_path())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CoreError::InvalidPath(
            "approved import source root is unsafe".to_owned(),
        ));
    }
    let relative = request
        .source_path
        .strip_prefix(&request.approved_source_root)
        .map_err(|_| CoreError::InvalidPath("import source escapes approved root".to_owned()))?;
    let mut current = request.approved_source_root.clone();
    for component in relative.iter() {
        current.push(component);
        let metadata = fs::symlink_metadata(current.as_std_path())?;
        if metadata.file_type().is_symlink() {
            return Err(CoreError::InvalidPath(
                "import source path contains a symlink".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_destination_root(root: &Utf8Path) -> Result<(), CoreError> {
    let parent = root
        .parent()
        .ok_or_else(|| CoreError::InvalidPath("canonical asset root has no parent".to_owned()))?;
    let parent_metadata = fs::symlink_metadata(parent.as_std_path())?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(CoreError::InvalidPath(
            "canonical asset root parent is unsafe".to_owned(),
        ));
    }
    match fs::symlink_metadata(root.as_std_path()) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(
            CoreError::InvalidPath("canonical asset root is unsafe".to_owned()),
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CoreError::Io(error)),
    }
}

fn validate_source_shape(kind: AssetKind, fingerprint: &PathFingerprint) -> Result<(), CoreError> {
    let valid = if kind == AssetKind::Skill {
        fingerprint.entry_type == FingerprintType::Directory
    } else {
        fingerprint.entry_type == FingerprintType::File
    };
    if valid {
        Ok(())
    } else {
        Err(CoreError::InvalidPath(
            "import source has an unsupported path shape".to_owned(),
        ))
    }
}

fn ensure_destination_parent(action: &ImportAction) -> Result<(), CoreError> {
    validate_destination_root(&action.destination_asset_root)?;
    if !action.destination_asset_root.exists() {
        fs::create_dir(action.destination_asset_root.as_std_path())?;
        let parent = action.destination_asset_root.parent().ok_or_else(|| {
            CoreError::InvalidPath("canonical asset root has no parent".to_owned())
        })?;
        sync_directory(parent)?;
    }
    let relative = action
        .destination_path
        .strip_prefix(&action.destination_asset_root)
        .map_err(|_| {
            CoreError::InvalidPath("canonical destination escapes asset root".to_owned())
        })?;
    let components = relative.iter().collect::<Vec<_>>();
    let mut current = action.destination_asset_root.clone();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        current.push(component);
        match fs::symlink_metadata(current.as_std_path()) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(CoreError::InvalidPath(
                    "canonical destination parent is unsafe".to_owned(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(current.as_std_path())?;
                let parent = current.parent().ok_or_else(|| {
                    CoreError::InvalidPath("canonical destination parent has no parent".to_owned())
                })?;
                sync_directory(parent)?;
            }
            Err(error) => return Err(CoreError::Io(error)),
        }
    }
    Ok(())
}

fn ensure_private_directory(path: &Utf8Path) -> Result<(), CoreError> {
    match fs::symlink_metadata(path.as_std_path()) {
        Ok(_) => validate_private_directory(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or_else(|| {
                CoreError::InvalidPath("transaction root has no parent".to_owned())
            })?;
            let parent_metadata = fs::symlink_metadata(parent.as_std_path())?;
            if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
                return Err(CoreError::InvalidPath(
                    "transaction root parent is unsafe".to_owned(),
                ));
            }
            fs::create_dir(path.as_std_path())?;
            set_mode(path, 0o700)?;
            sync_directory(parent)?;
            sync_directory(path)
        }
        Err(error) => Err(CoreError::Io(error)),
    }
}

fn validate_private_directory(path: &Utf8Path) -> Result<(), CoreError> {
    let metadata = fs::symlink_metadata(path.as_std_path())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CoreError::InvalidPath(
            "transaction root is unsafe".to_owned(),
        ));
    }
    set_mode(path, 0o700)
}

fn write_manifest_handle(
    file: &mut NamedTempFile,
    manifest: &ImportTransactionManifest,
) -> Result<(), CoreError> {
    file.as_file_mut().seek(SeekFrom::Start(0))?;
    file.as_file_mut().set_len(0)?;
    serde_json::to_writer_pretty(file.as_file_mut(), manifest)?;
    file.as_file_mut().write_all(b"\n")?;
    file.as_file_mut().sync_all()?;
    Ok(())
}

fn write_manifest_path(
    path: &Utf8Path,
    manifest: &ImportTransactionManifest,
) -> Result<(), CoreError> {
    let metadata = fs::symlink_metadata(path.as_std_path())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CoreError::InvalidPath(
            "migration manifest became unsafe".to_owned(),
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| CoreError::InvalidPath("migration manifest has no parent".to_owned()))?;
    let parent_metadata = fs::symlink_metadata(parent.as_std_path())?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(CoreError::InvalidPath(
            "migration manifest parent became unsafe".to_owned(),
        ));
    }
    let mut temporary = NamedTempFile::new_in(parent.as_std_path()).map_err(CoreError::Io)?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), manifest)?;
    temporary.as_file_mut().write_all(b"\n")?;
    let temporary_path = Utf8Path::from_path(temporary.path())
        .ok_or_else(|| CoreError::InvalidPath("manifest temporary path is not UTF-8".to_owned()))?;
    set_mode(temporary_path, 0o600)?;
    temporary.as_file_mut().sync_all()?;
    temporary
        .persist(path.as_std_path())
        .map_err(|error| CoreError::Io(error.error))?;
    fs::File::open(parent.as_std_path())?.sync_all()?;
    Ok(())
}

fn destination_path(request: &ImportRequest) -> Utf8PathBuf {
    match request.kind {
        AssetKind::Skill => request
            .destination_asset_root
            .join("skills")
            .join(&request.name),
        AssetKind::Rule => request
            .destination_asset_root
            .join("rules")
            .join(format!("{}.mdc", request.name)),
        AssetKind::Mcp => request
            .destination_asset_root
            .join("mcp/servers")
            .join(format!("{}.json", request.name)),
        AssetKind::Agent => request
            .destination_asset_root
            .join("agents")
            .join(format!("{}.md", request.name)),
        AssetKind::Command => request
            .destination_asset_root
            .join("commands")
            .join(format!("{}.md", request.name)),
        AssetKind::Prompt => request.destination_asset_root.join("prompts/AGENTS.md"),
        AssetKind::Hook => request
            .destination_asset_root
            .join("hooks")
            .join(&request.name),
    }
}

fn platform_mcp_entry_from_bytes(
    request: &ImportRequest,
    source_bytes: &[u8],
) -> Result<Value, CoreError> {
    let mut entry = match request.source_platform {
        PlatformId::Cursor => {
            let document: Value = serde_json::from_slice(source_bytes)?;
            document
                .get("mcpServers")
                .and_then(Value::as_object)
                .and_then(|servers| servers.get(&request.name))
                .cloned()
        }
        PlatformId::Codex => {
            let document = std::str::from_utf8(source_bytes)
                .map_err(|_| CoreError::InvalidPath("Codex MCP TOML is not UTF-8".to_owned()))?;
            codex_mcp_entry(document, &request.name)?
        }
        _ => {
            return Err(CoreError::InvalidPath(
                "this import slice accepts only Cursor or Codex MCP containers".to_owned(),
            ))
        }
    }
    .ok_or_else(|| CoreError::InvalidPath("selected MCP entry is missing".to_owned()))?;
    if request.source_platform == PlatformId::Codex {
        entry
            .as_object_mut()
            .ok_or_else(|| CoreError::InvalidPath("selected MCP entry is not a table".to_owned()))?
            .remove("type");
    }
    Ok(entry)
}

fn inspect_mcp_secrets(entry: &Value) -> ImportSecretPreflight {
    let mut keys = BTreeSet::new();
    let mut reasons = BTreeSet::new();
    for field in ["env", "headers"] {
        if let Some(values) = entry.get(field).and_then(Value::as_object) {
            for (key, value) in values {
                let is_reference = value.as_str().and_then(secret_reference).is_some();
                if !is_reference {
                    keys.insert(key.to_owned());
                    reasons.insert("literal_secret_value".to_owned());
                }
            }
        }
    }
    if let Some(headers) = entry.get("http_headers").and_then(Value::as_object) {
        for (key, value) in headers {
            if credential_like_key(key) && !value.is_null() {
                keys.insert(key.to_owned());
                reasons.insert("literal_secret_value".to_owned());
            }
        }
    }
    if let Some(headers) = entry.get("env_http_headers").and_then(Value::as_object) {
        for (key, value) in headers {
            let valid_reference = value.as_str().is_some_and(safe_secret_key);
            if !valid_reference {
                keys.insert(key.to_owned());
                reasons.insert("invalid_environment_reference".to_owned());
            }
        }
    }
    if entry
        .get("bearer_token_env_var")
        .is_some_and(|value| !value.as_str().is_some_and(safe_secret_key))
    {
        keys.insert("bearer_token_env_var".to_owned());
        reasons.insert("invalid_environment_reference".to_owned());
    }
    if let Some(fields) = entry.as_object() {
        for (key, value) in fields {
            if matches!(
                key.as_str(),
                "env" | "headers" | "http_headers" | "env_http_headers" | "bearer_token_env_var"
            ) {
                continue;
            }
            if credential_like_key(key) && !value.is_null() {
                keys.insert(key.to_owned());
                reasons.insert("credential_like_field".to_owned());
            }
            if contains_placeholder(value) {
                keys.insert(key.to_owned());
                reasons.insert("secret_reference_outside_env_or_headers".to_owned());
            }
            inspect_nested_secret_fields(value, &mut keys, &mut reasons);
        }
    }
    if entry
        .get("url")
        .and_then(Value::as_str)
        .is_some_and(url_has_userinfo)
    {
        keys.insert("URL_USERINFO".to_owned());
        reasons.insert("url_userinfo_credential".to_owned());
    }
    if entry
        .get("args")
        .and_then(Value::as_array)
        .is_some_and(|args| args_have_credential_literal(args))
    {
        keys.insert("COMMAND_ARGUMENT".to_owned());
        reasons.insert("credential_like_argument".to_owned());
    }
    ImportSecretPreflight {
        status: if reasons.is_empty() {
            ImportSecretStatus::Clear
        } else {
            ImportSecretStatus::Blocked
        },
        key_names: keys.into_iter().collect(),
        reason_codes: reasons.into_iter().collect(),
    }
}

fn inspect_nested_secret_fields(
    value: &Value,
    keys: &mut BTreeSet<String>,
    reasons: &mut BTreeSet<String>,
) {
    match value {
        Value::Object(fields) => {
            for (key, nested) in fields {
                if credential_like_key(key) && !nested.is_null() {
                    keys.insert(key.to_owned());
                    reasons.insert("credential_like_field".to_owned());
                }
                if key.eq_ignore_ascii_case("url") && nested.as_str().is_some_and(url_has_userinfo)
                {
                    keys.insert("URL_USERINFO".to_owned());
                    reasons.insert("url_userinfo_credential".to_owned());
                }
                if key.eq_ignore_ascii_case("args")
                    && nested
                        .as_array()
                        .is_some_and(|values| args_have_credential_literal(values))
                {
                    keys.insert("COMMAND_ARGUMENT".to_owned());
                    reasons.insert("credential_like_argument".to_owned());
                }
                inspect_nested_secret_fields(nested, keys, reasons);
            }
        }
        Value::Array(values) => {
            for nested in values {
                inspect_nested_secret_fields(nested, keys, reasons);
            }
        }
        Value::String(value) => {
            if url_has_userinfo(value) {
                keys.insert("URL_USERINFO".to_owned());
                reasons.insert("url_userinfo_credential".to_owned());
            }
            if url_has_credential_query(value) {
                keys.insert("URL_QUERY".to_owned());
                reasons.insert("url_query_credential".to_owned());
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn credential_like_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase();
    [
        "token",
        "password",
        "secret",
        "api-key",
        "api_key",
        "apikey",
        "credential",
        "authorization",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn canonical_mcp_value(request: &ImportRequest, entry: Value) -> Value {
    let transport = if entry.get("url").is_some() {
        "http"
    } else {
        "stdio"
    };
    json!({
        "transport": transport,
        "enabled": true,
        "config": entry,
        "targets": [platform_name(request.source_platform)],
    })
}

fn platform_name(platform: PlatformId) -> &'static str {
    match platform {
        PlatformId::AgentsManager => "agentsmanager",
        PlatformId::Cursor => "cursor",
        PlatformId::Codex => "codex",
        PlatformId::Claude => "claude",
        PlatformId::Hermes => "hermes",
    }
}

fn secret_reference(value: &str) -> Option<&str> {
    value
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
        .filter(|key| safe_secret_key(key))
}

fn safe_secret_key(key: &str) -> bool {
    !key.is_empty()
        && key.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_uppercase() || (index != 0 && byte.is_ascii_digit())
        })
}

fn contains_placeholder(value: &Value) -> bool {
    match value {
        Value::String(value) => value.contains("${"),
        Value::Array(values) => values.iter().any(contains_placeholder),
        Value::Object(values) => values.values().any(contains_placeholder),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

fn url_has_userinfo(value: &str) -> bool {
    value.split_once("://").is_some_and(|(_, rest)| {
        let authority = rest.split('/').next().unwrap_or(rest);
        authority.contains('@')
    })
}

fn url_has_credential_query(value: &str) -> bool {
    if !value.contains("://") {
        return false;
    }
    let Some((_, query_and_fragment)) = value.split_once('?') else {
        return false;
    };
    let query = query_and_fragment.split('#').next().unwrap_or_default();
    query.split('&').any(|pair| {
        let raw_key = pair.split_once('=').map_or(pair, |(key, _)| key);
        credential_like_key(&percent_decode_query_key(raw_key))
    })
}

fn percent_decode_query_key(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = String::with_capacity(value.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_nibble(bytes[index + 1]), hex_nibble(bytes[index + 2]))
            {
                decoded.push(char::from((high << 4) | low));
                index += 3;
                continue;
            }
        }
        decoded.push(if bytes[index] == b'+' {
            ' '
        } else {
            char::from(bytes[index])
        });
        index += 1;
    }
    decoded
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn args_have_credential_literal(args: &[Value]) -> bool {
    args.iter().enumerate().any(|(index, value)| {
        let Some(value) = value.as_str() else {
            return false;
        };
        let lower = value.to_ascii_lowercase();
        let credential = ["token", "password", "secret", "api-key", "apikey"]
            .iter()
            .any(|needle| lower.contains(needle));
        credential
            && (lower
                .split_once('=')
                .is_some_and(|(_, value)| !value.is_empty())
                || args
                    .get(index + 1)
                    .and_then(Value::as_str)
                    .is_some_and(|next| !next.starts_with('-')))
    })
}

fn file_bytes_digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"file\0");
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn import_path_digest(kind: AssetKind, path: &Utf8Path) -> Result<String, CoreError> {
    if kind == AssetKind::Skill {
        Ok(freeze_tree(path)?.2)
    } else {
        path_content_digest(path)
    }
}

fn import_fingerprint(kind: AssetKind, path: &Utf8Path) -> Result<PathFingerprint, CoreError> {
    let mut fingerprint = path_fingerprint(path)?;
    if kind == AssetKind::Skill && fingerprint.entry_type != FingerprintType::Missing {
        fingerprint.digest = Some(freeze_tree(path)?.2);
    }
    Ok(fingerprint)
}

fn stable_digest(value: &impl Serialize) -> Result<String, CoreError> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(value)?)))
}

fn safe_name(value: &str) -> bool {
    !value.is_empty() && value != "." && value != ".." && !value.contains(['/', '\\', '\0'])
}

#[cfg(unix)]
fn set_mode(path: &Utf8Path, mode: u32) -> Result<(), CoreError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path.as_std_path(), fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(unix)]
fn path_mode(metadata: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;

    Some(metadata.permissions().mode())
}

#[cfg(not(unix))]
fn path_mode(_metadata: &fs::Metadata) -> Option<u32> {
    None
}

fn set_optional_mode(path: &Utf8Path, mode: Option<u32>) -> Result<(), CoreError> {
    if let Some(mode) = mode {
        set_mode(path, mode & 0o777)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_mode(_path: &Utf8Path, _mode: u32) -> Result<(), CoreError> {
    Ok(())
}
