//! Transactional source-first projection executor.
//!
//! The initial direct-link slice deliberately accepts only proven `CreateLink` and `Noop`
//! actions. Generated rendering, adoption, copy fallback and cleanup are added by later T006
//! tests; they must never fall back to the legacy materialize path.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
#[cfg(unix)]
use std::io::Read;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_yaml::{Mapping, Value as YamlValue};

use crate::error::CoreError;

use super::fingerprint::{path_content_digest, path_fingerprint};
use super::ledger::ProjectionLedger;
use super::mcp::codex_toml::{render_codex_mcp_toml, TomlServerIntent};
use super::mcp::cursor_json::{render_cursor_mcp_json, JsonServerIntent};
use super::mcp::entry_fingerprint::{
    inspect_codex_mcp_entries, inspect_cursor_mcp_entries, inspect_hermes_mcp_entries,
    McpEntryFingerprint,
};
use super::mcp::hermes_yaml::{render_hermes_mcp_yaml, YamlServerIntent};
use super::mcp::source::load_mcp_definition_at;
use super::model::{LedgerMutation, ProjectionMode, ProjectionRecord, ProjectionSurface};
use super::planner::{
    GeneratedContainerRenderer, McpProjectionMember, ProjectionAction, ProjectionActionKind,
    ProjectionPlan,
};

static TEMP_LINK_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const APPLY_LOCK_NAME: &str = ".ai-config-projection.lock";

/// Resolve a secret only when a plan-bound MCP source declares its exact variable name. Core
/// deliberately has no environment or HOME fallback: callers choose their own secret boundary.
/// Implementations must not surface returned values through errors or logs.
pub trait McpSecretProvider {
    fn resolve(&self, key: &str) -> Result<Option<String>, CoreError>;
}

/// Apply-time dependencies. The deploy root is an explicit allowlist boundary, never HOME.
pub struct ExecutorContext<'a> {
    ledger: &'a dyn ProjectionLedger,
    deploy_base: Utf8PathBuf,
    backup_root: Utf8PathBuf,
    mcp_secret_provider: Option<&'a dyn McpSecretProvider>,
}

impl<'a> ExecutorContext<'a> {
    pub fn new(
        ledger: &'a dyn ProjectionLedger,
        deploy_base: Utf8PathBuf,
        backup_root: Utf8PathBuf,
    ) -> Self {
        Self {
            ledger,
            deploy_base,
            backup_root,
            mcp_secret_provider: None,
        }
    }

    /// Inject a caller-owned secret resolver for MCP source hydration. The executor never
    /// consults process environment variables or user home directories on its own.
    pub fn with_mcp_secret_provider(mut self, provider: &'a dyn McpSecretProvider) -> Self {
        self.mcp_secret_provider = Some(provider);
        self
    }
}

/// Explicit authorization data for a plan. Adopt selection is added before adopt becomes
/// executable; an empty selection never upgrades a candidate to a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOptions {
    expected_plan_digest: String,
    selected_action_ids: BTreeSet<String>,
}

impl ApplyOptions {
    pub fn for_plan(plan: &ProjectionPlan) -> Self {
        Self {
            expected_plan_digest: plan.plan_digest.clone(),
            selected_action_ids: BTreeSet::new(),
        }
    }

    /// Explicitly authorize a subset of `AdoptEquivalent` candidates from this exact plan.
    /// The executor validates every supplied ID before it writes anything.
    pub fn with_selected_action_ids(
        plan: &ProjectionPlan,
        selected_action_ids: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            expected_plan_digest: plan.plan_digest.clone(),
            selected_action_ids: selected_action_ids.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplyActionStatus {
    Applied,
    Unchanged,
    Skipped,
    Conflict,
    Failed,
    RolledBack,
    RollbackFailed,
    NotApplied,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ApplyActionReport {
    pub action_id: String,
    pub status: ApplyActionStatus,
}

/// A source-first MCP server skipped because the caller-owned resolver did not provide every
/// declared secret. This deliberately reports variable names only: secret values never enter
/// the plan, report, or error surface.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct McpSkippedMemberReport {
    pub entry_key: String,
    pub missing_secret_keys: Vec<String>,
}

/// Transaction-level failure classification. It deliberately carries a stable code rather than
/// the underlying error text, so reports remain safe to serialize without exposing source data.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ApplyFailure {
    pub code: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ApplyReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
    pub changed: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub conflict: usize,
    pub failed: usize,
    pub rolled_back: usize,
    pub rollback_failed: usize,
    pub not_applied: usize,
    pub actions: Vec<ApplyActionReport>,
    pub mcp_skipped_members: Vec<McpSkippedMemberReport>,
    pub failure: Option<ApplyFailure>,
}

impl ApplyReport {
    fn for_plan(plan: &ProjectionPlan) -> Self {
        let actions = plan
            .actions
            .iter()
            .enumerate()
            .map(|(index, _)| ApplyActionReport {
                action_id: plan
                    .action_ids
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| format!("invalid-action-{index}")),
                status: ApplyActionStatus::NotApplied,
            })
            .collect();
        Self {
            transaction_id: None,
            changed: 0,
            unchanged: 0,
            skipped: 0,
            conflict: 0,
            failed: 0,
            rolled_back: 0,
            rollback_failed: 0,
            not_applied: 0,
            actions,
            mcp_skipped_members: Vec::new(),
            failure: None,
        }
    }

    fn set_status(&mut self, index: usize, status: ApplyActionStatus) {
        if let Some(action) = self.actions.get_mut(index) {
            action.status = status;
        }
    }

    fn recount(&mut self) {
        self.changed = 0;
        self.unchanged = 0;
        self.skipped = 0;
        self.conflict = 0;
        self.failed = 0;
        self.rolled_back = 0;
        self.rollback_failed = 0;
        self.not_applied = 0;
        for action in &self.actions {
            match action.status {
                ApplyActionStatus::Applied => self.changed += 1,
                ApplyActionStatus::Unchanged => self.unchanged += 1,
                ApplyActionStatus::Skipped => self.skipped += 1,
                ApplyActionStatus::Conflict => self.conflict += 1,
                ApplyActionStatus::Failed => self.failed += 1,
                ApplyActionStatus::RolledBack => self.rolled_back += 1,
                ApplyActionStatus::RollbackFailed => self.rollback_failed += 1,
                ApplyActionStatus::NotApplied => self.not_applied += 1,
            }
        }
    }

    fn mark_rollback_result(&mut self, index: usize, restored: bool) {
        let rollback_failed = !restored
            || self
                .actions
                .get(index)
                .is_some_and(|action| action.status == ApplyActionStatus::RollbackFailed);
        self.set_status(
            index,
            if rollback_failed {
                ApplyActionStatus::RollbackFailed
            } else {
                ApplyActionStatus::RolledBack
            },
        );
        self.recount();
    }
}

/// An apply error preserves the final transaction report. Callers can surface the precise
/// post-rollback action statuses while still using the contained `CoreError` for exit handling.
#[derive(Debug)]
pub struct ProjectionApplyError {
    pub error: CoreError,
    pub report: Box<ApplyReport>,
}

impl std::fmt::Display for ProjectionApplyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for ProjectionApplyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// A multi-plan transaction failed after at least one plan began staging. The contained reports
/// describe every slice: completed mutations become rolled back, the failing action remains
/// failed, and slices not entered remain not applied.
#[derive(Debug)]
pub struct ProjectionTransactionError {
    pub error: CoreError,
    pub reports: Vec<ApplyReport>,
}

impl std::fmt::Display for ProjectionTransactionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for ProjectionTransactionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

enum FileUndo {
    RemoveCreated {
        target: Utf8PathBuf,
        source: Utf8PathBuf,
        created_parents: Vec<Utf8PathBuf>,
    },
    RestoreRemoved {
        target: Utf8PathBuf,
        raw_link_target: Utf8PathBuf,
    },
    RestoreBackup {
        target: Utf8PathBuf,
        source: Utf8PathBuf,
        backup: Utf8PathBuf,
    },
    RemoveCopied {
        target: Utf8PathBuf,
        digest: String,
        created_parents: Vec<Utf8PathBuf>,
    },
    RestoreCopiedBackup {
        target: Utf8PathBuf,
        digest: String,
        backup: Utf8PathBuf,
    },
    RestoreGenerated {
        target: Utf8PathBuf,
        rendered_digest: String,
        backup: Option<Utf8PathBuf>,
        created_parents: Vec<Utf8PathBuf>,
    },
}

/// Mutable state shared by one or more plan applications. It is intentionally private: callers
/// receive only committed reports, never a capability to bypass rollback.
#[derive(Default)]
struct TransactionJournal {
    mutations: Vec<LedgerMutation>,
    undo: Vec<(usize, FileUndo)>,
}

struct CreatedLink {
    target: Utf8PathBuf,
    source: Utf8PathBuf,
    created_parents: Vec<Utf8PathBuf>,
}

struct CreatedCopy {
    target: Utf8PathBuf,
    digest: String,
    created_parents: Vec<Utf8PathBuf>,
    replaced_backup: Option<Utf8PathBuf>,
}

struct SafeParent {
    path: Utf8PathBuf,
    created_parents: Vec<Utf8PathBuf>,
}

/// The manifest deliberately has no asset body or rendered bytes. It is enough to audit a
/// recovery: where the prior target was moved, plus its apply-time digest and mode.
#[derive(Serialize, Deserialize)]
struct BackupManifest {
    target_path: Utf8PathBuf,
    backup_path: Utf8PathBuf,
    digest: Option<String>,
    mode: Option<u32>,
}

const DURABLE_ADOPTION_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DurableAdoptionStatus {
    Applying,
    Applied,
    RolledBack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DurableAdoptionManifest {
    schema_version: u16,
    transaction_id: String,
    deploy_base: Utf8PathBuf,
    status: DurableAdoptionStatus,
    actions: Vec<DurableAdoptionAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DurableAdoptionAction {
    action_id: String,
    projection_id: super::model::ProjectionId,
    target_path: Utf8PathBuf,
    source_path: Utf8PathBuf,
    source_fingerprint: String,
    before: super::model::PathFingerprint,
    after: Option<super::model::PathFingerprint>,
    backup_path: Option<Utf8PathBuf>,
    expected_record: Option<ProjectionRecord>,
}

struct PreparedDurableAdoption {
    manifest_path: Utf8PathBuf,
    action_backup_root: Utf8PathBuf,
    manifest: DurableAdoptionManifest,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProjectionRollbackReport {
    pub transaction_id: String,
    pub restored: usize,
}

/// A create-new lock deliberately fails closed. We do not infer that an existing lock is stale:
/// a user or a separate process must inspect and clear it explicitly.
struct ApplyLock {
    path: Utf8PathBuf,
    _file: fs::File,
}

impl ApplyLock {
    fn acquire(deploy_base: &Utf8Path) -> Result<Self, CoreError> {
        let path = deploy_base.join(APPLY_LOCK_NAME);
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.as_std_path())
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    CoreError::InvalidPath(
                        "another projection apply is active or requires explicit lock inspection"
                            .to_owned(),
                    )
                } else {
                    CoreError::Io(error)
                }
            })?;
        Ok(Self { path, _file: file })
    }
}

impl Drop for ApplyLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.path.as_std_path());
    }
}

/// Apply the safe direct-link subset of a source-first plan.
pub fn apply_projection_plan(
    plan: &ProjectionPlan,
    context: &ExecutorContext<'_>,
    options: ApplyOptions,
) -> Result<ApplyReport, ProjectionApplyError> {
    if !options.selected_action_ids.is_empty() {
        return match apply_projection_plans_transactionally([(plan, options)], context) {
            Ok(mut reports) => Ok(reports.remove(0)),
            Err(mut failure) => {
                let report = if failure.reports.is_empty() {
                    ApplyReport::for_plan(plan)
                } else {
                    failure.reports.remove(0)
                };
                Err(ProjectionApplyError {
                    error: failure.error,
                    report: Box::new(report),
                })
            }
        };
    }
    if let Err(error) = fs::create_dir_all(context.deploy_base.as_std_path()) {
        return Err(preflight_failure(
            ApplyReport::for_plan(plan),
            "deploy_base_unavailable",
            CoreError::Io(error),
        ));
    }
    let _lock = match ApplyLock::acquire(&context.deploy_base) {
        Ok(lock) => lock,
        Err(error) => {
            return Err(preflight_failure(
                ApplyReport::for_plan(plan),
                "apply_lock_unavailable",
                error,
            ));
        }
    };
    let mut journal = TransactionJournal::default();
    let mut report = match stage_projection_plan(plan, context, options, &mut journal) {
        Ok(report) => report,
        Err(error) => {
            let code = error
                .report
                .failure
                .as_ref()
                .map(|failure| failure.code.clone())
                .unwrap_or_else(|| "action_apply_failed".to_owned());
            return Err(transaction_failure(
                *error.report,
                journal.undo,
                context,
                None,
                &code,
                error.error,
            ));
        }
    };
    if let Err(error) = context.ledger.apply_batch(&journal.mutations) {
        return Err(transaction_failure(
            report,
            journal.undo,
            context,
            None,
            "ledger_apply_failed",
            error,
        ));
    }
    report.recount();
    Ok(report)
}

/// Apply multiple independently planned slices as one filesystem and ledger transaction.
///
/// The planner may split direct assets, generated MCP containers, and other capability domains
/// into separate plans. This executor boundary keeps their file undo journal and apply lock
/// shared, then performs exactly one ownership-ledger commit after every slice has staged.
pub fn apply_projection_plans_transactionally<'plan>(
    plans: impl IntoIterator<Item = (&'plan ProjectionPlan, ApplyOptions)>,
    context: &ExecutorContext<'_>,
) -> Result<Vec<ApplyReport>, ProjectionTransactionError> {
    let plans = plans.into_iter().collect::<Vec<_>>();
    fs::create_dir_all(context.deploy_base.as_std_path()).map_err(|error| {
        ProjectionTransactionError {
            error: CoreError::Io(error),
            reports: Vec::new(),
        }
    })?;
    let _lock =
        ApplyLock::acquire(&context.deploy_base).map_err(|error| ProjectionTransactionError {
            error,
            reports: Vec::new(),
        })?;
    let mut durable =
        prepare_durable_adoption(&plans, context).map_err(|error| ProjectionTransactionError {
            error,
            reports: Vec::new(),
        })?;
    let durable_context = durable.as_ref().map(|prepared| ExecutorContext {
        ledger: context.ledger,
        deploy_base: context.deploy_base.clone(),
        backup_root: prepared.action_backup_root.clone(),
        mcp_secret_provider: context.mcp_secret_provider,
    });
    let execution_context = durable_context.as_ref().unwrap_or(context);
    let mut journal = TransactionJournal::default();
    let mut reports = Vec::new();
    let mut completed_undo_ranges = Vec::new();
    let mut plans = plans.into_iter();

    while let Some((plan, options)) = plans.next() {
        let undo_start = journal.undo.len();
        match stage_projection_plan(plan, execution_context, options, &mut journal) {
            Ok(report) => {
                completed_undo_ranges.push((reports.len(), undo_start..journal.undo.len()));
                reports.push(report);
            }
            Err(error) => {
                let mut failed_report = *error.report;
                let rollback_results = rollback(&journal.undo, execution_context);
                mark_completed_plan_rollbacks(
                    &mut reports,
                    &completed_undo_ranges,
                    &journal.undo,
                    &rollback_results,
                );
                mark_current_plan_rollbacks(
                    &mut failed_report,
                    undo_start,
                    &journal.undo,
                    &rollback_results,
                );
                reports.push(failed_report);
                reports.extend(plans.map(|(remaining, _)| unapplied_report(remaining)));
                mark_durable_adoption_rolled_back(durable.as_mut());
                return Err(ProjectionTransactionError {
                    error: error.error,
                    reports,
                });
            }
        }
    }
    if let Err(error) = context.ledger.apply_batch(&journal.mutations) {
        let rollback_results = rollback(&journal.undo, execution_context);
        mark_completed_plan_rollbacks(
            &mut reports,
            &completed_undo_ranges,
            &journal.undo,
            &rollback_results,
        );
        mark_durable_adoption_rolled_back(durable.as_mut());
        return Err(ProjectionTransactionError { error, reports });
    }
    if let Some(prepared) = durable.as_mut() {
        if let Err(error) = finalize_durable_adoption(prepared, &journal, context.ledger) {
            let rollback_results = rollback(&journal.undo, execution_context);
            mark_completed_plan_rollbacks(
                &mut reports,
                &completed_undo_ranges,
                &journal.undo,
                &rollback_results,
            );
            let removals = prepared
                .manifest
                .actions
                .iter()
                .cloned()
                .map(|action| LedgerMutation::Remove(action.projection_id))
                .collect::<Vec<_>>();
            let ledger_rollback = context.ledger.apply_batch(&removals);
            mark_durable_adoption_rolled_back(Some(prepared));
            return Err(ProjectionTransactionError {
                error: if ledger_rollback.is_ok() {
                    error
                } else {
                    CoreError::ProjectionLedger(
                        "durable adoption manifest failed and ownership rollback also failed"
                            .to_owned(),
                    )
                },
                reports,
            });
        }
        for report in &mut reports {
            report.transaction_id = Some(prepared.manifest.transaction_id.clone());
        }
    }
    Ok(reports)
}

fn prepare_durable_adoption(
    plans: &[(&ProjectionPlan, ApplyOptions)],
    context: &ExecutorContext<'_>,
) -> Result<Option<PreparedDurableAdoption>, CoreError> {
    let mut actions = Vec::new();
    for (plan, options) in plans {
        validate_selected_action_ids(plan, &options.selected_action_ids)?;
        if options.selected_action_ids.is_empty() {
            continue;
        }
        if plan.actions.iter().enumerate().any(|(index, action)| {
            action.kind != ProjectionActionKind::AdoptEquivalent
                || !options
                    .selected_action_ids
                    .contains(&plan.action_ids[index])
        }) {
            return Err(CoreError::InvalidPath(
                "durable adoption plans may contain only explicitly selected AdoptEquivalent actions"
                    .to_owned(),
            ));
        }
        for (index, action) in plan.actions.iter().enumerate() {
            let target = action.target.as_ref().ok_or_else(|| {
                CoreError::InvalidPath("durable adoption action has no target".to_owned())
            })?;
            let source = action.members.first().ok_or_else(|| {
                CoreError::InvalidPath("durable adoption action has no source member".to_owned())
            })?;
            let before = action.precondition.clone().ok_or_else(|| {
                CoreError::InvalidPath("durable adoption action has no precondition".to_owned())
            })?;
            actions.push(DurableAdoptionAction {
                action_id: plan.action_ids[index].clone(),
                projection_id: direct_member_id(action)?,
                target_path: target.path.clone(),
                source_path: source.source.absolute_path.clone(),
                source_fingerprint: source.source.fingerprint.clone(),
                before,
                after: None,
                backup_path: None,
                expected_record: None,
            });
        }
    }
    if actions.is_empty() {
        return Ok(None);
    }
    let mut targets = BTreeSet::new();
    if actions
        .iter()
        .any(|action| !targets.insert(action.target_path.clone()))
    {
        return Err(CoreError::InvalidPath(
            "durable adoption contains duplicate targets".to_owned(),
        ));
    }
    ensure_safe_backup_root(&context.backup_root, &actions[0].target_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            context.backup_root.as_std_path(),
            fs::Permissions::from_mode(0o700),
        )?;
    }
    for _ in 0..1_024 {
        let transaction_id = format!("adopt-{:016x}", backup_nonce()?);
        let manifest_path = context.backup_root.join(format!("{transaction_id}.json"));
        let action_backup_root = context.backup_root.join(format!("{transaction_id}.d"));
        match fs::create_dir(action_backup_root.as_std_path()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(CoreError::Io(error)),
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                action_backup_root.as_std_path(),
                fs::Permissions::from_mode(0o700),
            )?;
        }
        let manifest = DurableAdoptionManifest {
            schema_version: DURABLE_ADOPTION_SCHEMA_VERSION,
            transaction_id,
            deploy_base: context.deploy_base.clone(),
            status: DurableAdoptionStatus::Applying,
            actions: actions.clone(),
        };
        match write_durable_manifest_create(&manifest_path, &manifest) {
            Ok(()) => {
                sync_directory(&context.backup_root)?;
                return Ok(Some(PreparedDurableAdoption {
                    manifest_path,
                    action_backup_root,
                    manifest,
                }));
            }
            Err(CoreError::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_dir(action_backup_root.as_std_path());
            }
            Err(error) => {
                let _ = fs::remove_dir(action_backup_root.as_std_path());
                return Err(error);
            }
        }
    }
    Err(CoreError::InvalidPath(
        "could not reserve a durable adoption transaction ID".to_owned(),
    ))
}

fn finalize_durable_adoption(
    prepared: &mut PreparedDurableAdoption,
    journal: &TransactionJournal,
    ledger: &dyn ProjectionLedger,
) -> Result<(), CoreError> {
    for action in &mut prepared.manifest.actions {
        let backup = journal.undo.iter().find_map(|(_, undo)| match undo {
            FileUndo::RestoreBackup {
                target,
                source,
                backup,
            } if target == &action.target_path && source == &action.source_path => {
                Some(backup.clone())
            }
            _ => None,
        });
        let backup = backup.ok_or_else(|| {
            CoreError::ProjectionLedger(
                "durable adoption completed without its rollback backup".to_owned(),
            )
        })?;
        let after = path_fingerprint(&action.target_path)?;
        if after.entry_type != super::model::FingerprintType::Symlink
            || after.link_target.as_ref() != Some(&action.source_path)
        {
            return Err(CoreError::InvalidPath(
                "durable adoption post-state is not the expected canonical link".to_owned(),
            ));
        }
        let record = ledger.get(&action.projection_id)?.ok_or_else(|| {
            CoreError::ProjectionLedger(
                "durable adoption ownership record was not committed".to_owned(),
            )
        })?;
        if record.target_path != action.target_path || record.source_path != action.source_path {
            return Err(CoreError::ProjectionLedger(
                "durable adoption ownership record does not match its target".to_owned(),
            ));
        }
        action.after = Some(after);
        action.backup_path = Some(backup);
        action.expected_record = Some(record);
    }
    prepared.manifest.status = DurableAdoptionStatus::Applied;
    write_durable_manifest_atomic(&prepared.manifest_path, &prepared.manifest)
}

fn mark_durable_adoption_rolled_back(prepared: Option<&mut PreparedDurableAdoption>) {
    if let Some(prepared) = prepared {
        prepared.manifest.status = DurableAdoptionStatus::RolledBack;
        let _ = write_durable_manifest_atomic(&prepared.manifest_path, &prepared.manifest);
    }
}

fn write_durable_manifest_create(
    path: &Utf8Path,
    manifest: &DurableAdoptionManifest,
) -> Result<(), CoreError> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path.as_std_path())?;
    set_private_file_permissions(path)?;
    serde_json::to_writer_pretty(&mut file, manifest)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

fn write_durable_manifest_atomic(
    path: &Utf8Path,
    manifest: &DurableAdoptionManifest,
) -> Result<(), CoreError> {
    let metadata = fs::symlink_metadata(path.as_std_path())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CoreError::InvalidPath(
            "durable adoption manifest became unsafe".to_owned(),
        ));
    }
    let parent = path.parent().ok_or_else(|| {
        CoreError::InvalidPath("durable adoption manifest has no parent".to_owned())
    })?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent.as_std_path())?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), manifest)?;
    temporary.as_file_mut().write_all(b"\n")?;
    temporary.as_file_mut().sync_all()?;
    let temporary_path = Utf8Path::from_path(temporary.path()).ok_or_else(|| {
        CoreError::InvalidPath("durable adoption temporary path is not UTF-8".to_owned())
    })?;
    set_private_file_permissions(temporary_path)?;
    temporary
        .persist(path.as_std_path())
        .map_err(|error| CoreError::Io(error.error))?;
    sync_directory(parent)
}

fn sync_directory(path: &Utf8Path) -> Result<(), CoreError> {
    fs::File::open(path.as_std_path())?.sync_all()?;
    Ok(())
}

pub fn rollback_projection_transaction(
    context: &ExecutorContext<'_>,
    transaction_id: &str,
) -> Result<ProjectionRollbackReport, CoreError> {
    if !transaction_id.starts_with("adopt-")
        || !transaction_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(CoreError::InvalidPath(
            "invalid durable adoption transaction ID".to_owned(),
        ));
    }
    let _lock = ApplyLock::acquire(&context.deploy_base)?;
    ensure_safe_backup_root(&context.backup_root, &context.deploy_base)?;
    let manifest_path = context.backup_root.join(format!("{transaction_id}.json"));
    let metadata = fs::symlink_metadata(manifest_path.as_std_path())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CoreError::InvalidPath(
            "durable adoption manifest is unsafe".to_owned(),
        ));
    }
    let mut manifest: DurableAdoptionManifest =
        serde_json::from_slice(&fs::read(manifest_path.as_std_path())?)?;
    if manifest.schema_version != DURABLE_ADOPTION_SCHEMA_VERSION
        || manifest.transaction_id != transaction_id
        || manifest.deploy_base != context.deploy_base
        || !matches!(
            manifest.status,
            DurableAdoptionStatus::Applying | DurableAdoptionStatus::Applied
        )
    {
        return Err(CoreError::InvalidPath(
            "durable adoption transaction is not rollback-ready".to_owned(),
        ));
    }
    let action_backup_root = context.backup_root.join(format!("{transaction_id}.d"));
    let backup_root_metadata = fs::symlink_metadata(action_backup_root.as_std_path())?;
    if backup_root_metadata.file_type().is_symlink() || !backup_root_metadata.is_dir() {
        return Err(CoreError::InvalidPath(
            "durable adoption backup root is unsafe".to_owned(),
        ));
    }

    let mut restore_actions = Vec::new();
    let mut removals = Vec::new();
    for action in &manifest.actions {
        let safe_target = ensure_safe_target_path(&action.target_path, &context.deploy_base)?;
        let target = path_fingerprint(&safe_target)?;
        let record = context.ledger.get(&action.projection_id)?;
        let backup = match action.backup_path.clone() {
            Some(backup) => Some(backup),
            None => find_durable_backup(&action_backup_root, action)?,
        };
        let backup = match backup {
            Some(backup) => match fs::symlink_metadata(backup.as_std_path()) {
                Ok(_) => {
                    ensure_safe_durable_backup(&action_backup_root, &backup)?;
                    Some(backup)
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(CoreError::Io(error)),
            },
            None => None,
        };
        let target_is_link = target.entry_type == super::model::FingerprintType::Symlink
            && target.link_target.as_ref() == Some(&action.source_path);

        match manifest.status {
            DurableAdoptionStatus::Applied => {
                let after = action.after.as_ref().ok_or_else(|| {
                    CoreError::InvalidPath("durable adoption has no post-state".to_owned())
                })?;
                let backup = backup.ok_or_else(|| {
                    CoreError::InvalidPath("durable adoption has no backup path".to_owned())
                })?;
                if target != *after
                    || !target_is_link
                    || path_fingerprint(&backup)? != action.before
                    || !record
                        .as_ref()
                        .is_some_and(|record| durable_record_matches(record, action, &target))
                {
                    return Err(CoreError::InvalidPath(
                        "durable adoption rollback refused target, backup, or ownership drift"
                            .to_owned(),
                    ));
                }
                restore_actions.push((action.clone(), safe_target, backup));
                removals.push(LedgerMutation::Remove(action.projection_id.clone()));
            }
            DurableAdoptionStatus::Applying => match backup {
                Some(backup) if path_fingerprint(&backup)? == action.before => {
                    if target_is_link {
                        if let Some(record) = record.as_ref() {
                            if !durable_record_matches(record, action, &target) {
                                return Err(CoreError::ProjectionLedger(
                                    "durable adoption rollback refused ownership ledger drift"
                                        .to_owned(),
                                ));
                            }
                            removals.push(LedgerMutation::Remove(action.projection_id.clone()));
                        }
                        restore_actions.push((action.clone(), safe_target, backup));
                    } else if target.entry_type == super::model::FingerprintType::Missing {
                        if record.is_some() {
                            return Err(CoreError::ProjectionLedger(
                                "durable adoption has ownership without its canonical link"
                                    .to_owned(),
                            ));
                        }
                        restore_actions.push((action.clone(), safe_target, backup));
                    } else {
                        return Err(CoreError::InvalidPath(
                            "durable adoption applying state has an unexpected target".to_owned(),
                        ));
                    }
                }
                None if target == action.before && record.is_none() => {}
                None => {
                    return Err(CoreError::InvalidPath(
                        "durable adoption applying state is missing its recoverable backup"
                            .to_owned(),
                    ));
                }
                Some(_) => {
                    return Err(CoreError::InvalidPath(
                        "durable adoption applying state has a drifted backup".to_owned(),
                    ));
                }
            },
            DurableAdoptionStatus::RolledBack => unreachable!("validated above"),
        }
    }

    let mut restored = Vec::new();
    for (action, safe_target, backup) in restore_actions.iter().rev() {
        let current = path_fingerprint(safe_target)?;
        if current.entry_type == super::model::FingerprintType::Symlink {
            if current.link_target.as_ref() != Some(&action.source_path) {
                return Err(CoreError::InvalidPath(
                    "durable adoption target changed during rollback".to_owned(),
                ));
            }
            fs::remove_file(safe_target.as_std_path())?;
        } else if current.entry_type != super::model::FingerprintType::Missing {
            return Err(CoreError::InvalidPath(
                "durable adoption target changed during rollback".to_owned(),
            ));
        }
        if let Err(error) = fs::rename(backup.as_std_path(), safe_target.as_std_path()) {
            let safe_parent = safe_target.parent().unwrap_or(&context.deploy_base);
            let _ = create_sibling_symlink(&action.source_path, safe_target, safe_parent);
            return Err(CoreError::Io(error));
        }
        restored.push((action.clone(), safe_target.clone(), backup.clone()));
    }
    if let Err(error) = context.ledger.apply_batch(&removals) {
        let mut reapply_failed = false;
        for (action, safe_target, backup) in restored.iter().rev() {
            let safe_parent = safe_target.parent().unwrap_or(&context.deploy_base);
            if fs::rename(safe_target.as_std_path(), backup.as_std_path()).is_err()
                || create_sibling_symlink(&action.source_path, safe_target, safe_parent).is_err()
            {
                reapply_failed = true;
            }
        }
        return Err(if reapply_failed {
            CoreError::ProjectionLedger(
                "ownership rollback failed and adopted links could not all be restored".to_owned(),
            )
        } else {
            error
        });
    }
    manifest.status = DurableAdoptionStatus::RolledBack;
    write_durable_manifest_atomic(&manifest_path, &manifest)?;
    Ok(ProjectionRollbackReport {
        transaction_id: transaction_id.to_owned(),
        restored: restored.len(),
    })
}

fn unapplied_report(plan: &ProjectionPlan) -> ApplyReport {
    let mut report = ApplyReport::for_plan(plan);
    report.recount();
    report
}

fn mark_completed_plan_rollbacks(
    reports: &mut [ApplyReport],
    completed_undo_ranges: &[(usize, std::ops::Range<usize>)],
    undo: &[(usize, FileUndo)],
    rollback_results: &[(usize, bool)],
) {
    for (rollback_offset, ((action_index, restored), _)) in
        rollback_results.iter().zip(undo.iter().rev()).enumerate()
    {
        let undo_index = undo.len() - rollback_offset - 1;
        if let Some((report_index, _)) = completed_undo_ranges
            .iter()
            .find(|(_, range)| range.contains(&undo_index))
        {
            reports[*report_index].mark_rollback_result(*action_index, *restored);
        }
    }
}

fn mark_current_plan_rollbacks(
    report: &mut ApplyReport,
    undo_start: usize,
    undo: &[(usize, FileUndo)],
    rollback_results: &[(usize, bool)],
) {
    for (rollback_offset, ((action_index, restored), _)) in
        rollback_results.iter().zip(undo.iter().rev()).enumerate()
    {
        let undo_index = undo.len() - rollback_offset - 1;
        if undo_index >= undo_start {
            report.mark_rollback_result(*action_index, *restored);
        }
    }
}

/// Stage one plan beneath a caller-owned apply lock. No ledger mutation is committed here.
fn stage_projection_plan(
    plan: &ProjectionPlan,
    context: &ExecutorContext<'_>,
    options: ApplyOptions,
    journal: &mut TransactionJournal,
) -> Result<ApplyReport, ProjectionApplyError> {
    let mut report = ApplyReport::for_plan(plan);
    if options.expected_plan_digest != plan.plan_digest {
        return Err(preflight_failure(
            report,
            "plan_digest_mismatch",
            CoreError::InvalidPath(
                "apply options do not authorize this projection plan digest".to_owned(),
            ),
        ));
    }
    if let Err(error) = validate_selected_action_ids(plan, &options.selected_action_ids) {
        return Err(preflight_failure(report, "invalid_selected_action", error));
    }
    let mut has_blocking_report_only = false;
    for (index, action) in plan.actions.iter().enumerate() {
        if action.kind != ProjectionActionKind::ReportOnly {
            continue;
        }
        if is_missing_secret_skip(action) {
            report.set_status(index, ApplyActionStatus::Skipped);
            report
                .mcp_skipped_members
                .extend(
                    action
                        .mcp_members
                        .iter()
                        .map(|member| McpSkippedMemberReport {
                            entry_key: member.entry_key.clone(),
                            missing_secret_keys: member.missing_secret_keys.clone(),
                        }),
                );
        } else {
            report.set_status(index, ApplyActionStatus::Conflict);
            has_blocking_report_only = true;
        }
    }
    if has_blocking_report_only {
        return Err(preflight_failure(
            report,
            "blocking_conflict",
            CoreError::InvalidPath(
                "projection plan contains blocking report-only actions".to_owned(),
            ),
        ));
    }
    for (index, action) in plan.actions.iter().enumerate() {
        let outcome = match action.kind {
            ProjectionActionKind::CreateLink => {
                apply_create_link(action, context).and_then(|created| {
                    journal.mutations.push(direct_record_mutation(action)?);
                    journal.undo.push((
                        index,
                        FileUndo::RemoveCreated {
                            target: created.target,
                            source: created.source,
                            created_parents: created.created_parents,
                        },
                    ));
                    report.set_status(index, ApplyActionStatus::Applied);
                    Ok(())
                })
            }
            ProjectionActionKind::RemoveManagedLink => apply_remove_managed_link(action, context)
                .and_then(|removed| {
                    journal
                        .mutations
                        .push(LedgerMutation::Remove(direct_member_id(action)?));
                    journal.undo.push((
                        index,
                        FileUndo::RestoreRemoved {
                            target: removed.0,
                            raw_link_target: removed.1,
                        },
                    ));
                    report.set_status(index, ApplyActionStatus::Applied);
                    Ok(())
                }),
            ProjectionActionKind::CopyFallback => {
                apply_copy_fallback(action, context).and_then(|created| {
                    journal.mutations.push(copy_record_mutation(action)?);
                    let undo_operation = match created.replaced_backup {
                        Some(backup) => FileUndo::RestoreCopiedBackup {
                            target: created.target,
                            digest: created.digest,
                            backup,
                        },
                        None => FileUndo::RemoveCopied {
                            target: created.target,
                            digest: created.digest,
                            created_parents: created.created_parents,
                        },
                    };
                    journal.undo.push((index, undo_operation));
                    report.set_status(index, ApplyActionStatus::Applied);
                    Ok(())
                })
            }
            ProjectionActionKind::RemoveManagedCopy => apply_remove_managed_copy(action, context)
                .and_then(|removed| {
                    journal
                        .mutations
                        .push(LedgerMutation::Remove(direct_member_id(action)?));
                    journal.undo.push((
                        index,
                        FileUndo::RestoreCopiedBackup {
                            target: removed.0,
                            digest: removed.1,
                            backup: removed.2,
                        },
                    ));
                    report.set_status(index, ApplyActionStatus::Applied);
                    Ok(())
                }),
            ProjectionActionKind::Noop if action.members.is_empty() => {
                // Generated MCP-only batches already proved every named entry against the
                // persistent ledger during planning. They have no direct-link member from which
                // a direct record mutation could be derived, and must remain no-op.
                report.set_status(index, ApplyActionStatus::Unchanged);
                Ok(())
            }
            ProjectionActionKind::Noop => record_mutation_for_action(action).map(|mutation| {
                journal.mutations.push(mutation);
                report.set_status(index, ApplyActionStatus::Unchanged);
            }),
            ProjectionActionKind::AdoptEquivalent => {
                if options
                    .selected_action_ids
                    .contains(&plan.action_ids[index])
                {
                    apply_adopt_equivalent(action, context).and_then(|adopted| {
                        journal.mutations.push(direct_record_mutation(action)?);
                        journal.undo.push((
                            index,
                            FileUndo::RestoreBackup {
                                target: adopted.0,
                                source: adopted.1,
                                backup: adopted.2,
                            },
                        ));
                        report.set_status(index, ApplyActionStatus::Applied);
                        Ok(())
                    })
                } else {
                    report.set_status(index, ApplyActionStatus::Skipped);
                    Ok(())
                }
            }
            ProjectionActionKind::UpsertGeneratedBatch => match action.generated_renderer {
                Some(GeneratedContainerRenderer::HookJson) => {
                    apply_hook_generated_upsert(action, context).map(|applied| {
                        journal.mutations.extend(applied.mutations);
                        if let Some(undo_action) = applied.undo {
                            journal.undo.push((index, undo_action));
                            report.set_status(index, ApplyActionStatus::Applied);
                        } else {
                            report.set_status(index, ApplyActionStatus::Skipped);
                        }
                    })
                }
                Some(GeneratedContainerRenderer::HermesUnifiedYaml) => {
                    apply_hermes_unified_yaml_upsert(action, context).map(|applied| {
                        journal.mutations.extend(applied.mutations);
                        report.mcp_skipped_members.extend(applied.skipped_members);
                        if let Some(undo_action) = applied.undo {
                            journal.undo.push((index, undo_action));
                            report.set_status(index, ApplyActionStatus::Applied);
                        } else {
                            report.set_status(index, ApplyActionStatus::Skipped);
                        }
                    })
                }
                _ => apply_mcp_generated_upsert(action, context).map(|applied| {
                    journal.mutations.extend(applied.mutations);
                    report.mcp_skipped_members.extend(applied.skipped_members);
                    if let Some(undo_action) = applied.undo {
                        journal.undo.push((index, undo_action));
                        report.set_status(index, ApplyActionStatus::Applied);
                    } else {
                        report.set_status(index, ApplyActionStatus::Skipped);
                    }
                }),
            },
            ProjectionActionKind::RemoveGeneratedEntries => match action.generated_renderer {
                Some(GeneratedContainerRenderer::HookJson) => {
                    apply_hook_generated_retraction(action, context).map(|applied| {
                        journal.mutations.extend(applied.mutations);
                        if let Some(undo_action) = applied.undo {
                            journal.undo.push((index, undo_action));
                            report.set_status(index, ApplyActionStatus::Applied);
                        } else {
                            report.set_status(index, ApplyActionStatus::Skipped);
                        }
                    })
                }
                Some(GeneratedContainerRenderer::HermesUnifiedYaml) => {
                    apply_hermes_unified_yaml_retraction(action, context).map(|applied| {
                        journal.mutations.extend(applied.mutations);
                        if let Some(undo_action) = applied.undo {
                            journal.undo.push((index, undo_action));
                            report.set_status(index, ApplyActionStatus::Applied);
                        } else {
                            report.set_status(index, ApplyActionStatus::Skipped);
                        }
                    })
                }
                _ => apply_mcp_generated_retraction(action, context).map(|applied| {
                    journal.mutations.extend(applied.mutations);
                    if let Some(undo_action) = applied.undo {
                        journal.undo.push((index, undo_action));
                        report.set_status(index, ApplyActionStatus::Applied);
                    } else {
                        report.set_status(index, ApplyActionStatus::Skipped);
                    }
                }),
            },
            ProjectionActionKind::CleanupOrphan => Err(CoreError::NotImplemented(
                "projection action requires its dedicated transactional executor slice",
            )),
            ProjectionActionKind::ReportOnly => {
                report.set_status(index, ApplyActionStatus::Skipped);
                Ok(())
            }
        };
        if let Err(error) = outcome {
            return Err(staging_failure(
                report,
                Some(index),
                "action_apply_failed",
                error,
            ));
        }
    }
    report.recount();
    Ok(report)
}

fn preflight_failure(
    mut report: ApplyReport,
    code: &str,
    error: CoreError,
) -> ProjectionApplyError {
    report.failure = Some(ApplyFailure {
        code: code.to_owned(),
    });
    report.recount();
    ProjectionApplyError {
        error,
        report: Box::new(report),
    }
}

fn staging_failure(
    mut report: ApplyReport,
    failed_index: Option<usize>,
    code: &str,
    error: CoreError,
) -> ProjectionApplyError {
    if let Some(index) = failed_index {
        report.set_status(index, ApplyActionStatus::Failed);
    }
    report.failure = Some(ApplyFailure {
        code: code.to_owned(),
    });
    report.recount();
    ProjectionApplyError {
        error,
        report: Box::new(report),
    }
}

fn transaction_failure(
    mut report: ApplyReport,
    undo: Vec<(usize, FileUndo)>,
    context: &ExecutorContext<'_>,
    failed_index: Option<usize>,
    code: &str,
    error: CoreError,
) -> ProjectionApplyError {
    if let Some(index) = failed_index {
        report.set_status(index, ApplyActionStatus::Failed);
    }
    for (index, restored) in rollback(&undo, context) {
        report.set_status(
            index,
            if restored {
                ApplyActionStatus::RolledBack
            } else {
                ApplyActionStatus::RollbackFailed
            },
        );
    }
    report.failure = Some(ApplyFailure {
        code: code.to_owned(),
    });
    report.recount();
    ProjectionApplyError {
        error,
        report: Box::new(report),
    }
}

fn validate_selected_action_ids(
    plan: &ProjectionPlan,
    selected_action_ids: &BTreeSet<String>,
) -> Result<(), CoreError> {
    if plan.action_ids.len() != plan.actions.len() {
        return Err(CoreError::InvalidPath(
            "projection plan action IDs do not match its actions".to_owned(),
        ));
    }
    for selected_id in selected_action_ids {
        let Some(index) = plan.action_ids.iter().position(|id| id == selected_id) else {
            return Err(CoreError::InvalidPath(
                "selected projection action ID is not part of this plan".to_owned(),
            ));
        };
        if plan.actions[index].kind != ProjectionActionKind::AdoptEquivalent {
            return Err(CoreError::InvalidPath(
                "only an AdoptEquivalent action may be explicitly selected".to_owned(),
            ));
        }
    }
    Ok(())
}

fn apply_create_link(
    action: &ProjectionAction,
    context: &ExecutorContext<'_>,
) -> Result<CreatedLink, CoreError> {
    let target = action
        .target
        .as_ref()
        .ok_or_else(|| CoreError::InvalidPath("CreateLink action has no target".to_owned()))?;
    let source = action
        .members
        .first()
        .map(|member| &member.source.absolute_path)
        .ok_or_else(|| {
            CoreError::InvalidPath("CreateLink action has no source member".to_owned())
        })?;
    ensure_target_is_allowed(&target.path, &context.deploy_base)?;
    let expected = action.precondition.as_ref().ok_or_else(|| {
        CoreError::InvalidPath("CreateLink action has no target precondition".to_owned())
    })?;
    let actual = path_fingerprint(&target.path)?;
    if &actual != expected {
        return Err(CoreError::InvalidPath(
            "projection target changed after the plan was created".to_owned(),
        ));
    }
    if actual.entry_type != super::model::FingerprintType::Missing {
        return Err(CoreError::InvalidPath(
            "CreateLink may only create a previously missing target".to_owned(),
        ));
    }
    if !source.exists() {
        return Err(CoreError::ConfigNotFound {
            path: source.to_string(),
            hint: "重新生成投影计划，确认 canonical source 仍存在".to_owned(),
        });
    }
    ensure_source_matches_plan(action, source)?;

    let parent = ensure_safe_target_parent(&target.path, &context.deploy_base)?;
    create_sibling_symlink(source, &target.path, &parent.path)?;
    Ok(CreatedLink {
        target: target.path.clone(),
        source: source.clone(),
        created_parents: parent.created_parents,
    })
}

/// Remove only the exact link captured by a retract plan.
///
/// This intentionally does not share the legacy `remove_dest_path` helper: that helper can
/// remove files and directories, whereas a source-first retract is permitted to unlink only a
/// symlink that still points at the planned canonical source.
fn apply_remove_managed_link(
    action: &ProjectionAction,
    context: &ExecutorContext<'_>,
) -> Result<(Utf8PathBuf, Utf8PathBuf), CoreError> {
    let target = action.target.as_ref().ok_or_else(|| {
        CoreError::InvalidPath("RemoveManagedLink action has no target".to_owned())
    })?;
    let source = action
        .members
        .first()
        .map(|member| &member.source.absolute_path)
        .ok_or_else(|| {
            CoreError::InvalidPath("RemoveManagedLink action has no source member".to_owned())
        })?;
    ensure_target_is_allowed(&target.path, &context.deploy_base)?;
    let expected = action.precondition.as_ref().ok_or_else(|| {
        CoreError::InvalidPath("RemoveManagedLink action has no target precondition".to_owned())
    })?;
    let actual = path_fingerprint(&target.path)?;
    if &actual != expected {
        return Err(CoreError::InvalidPath(
            "projection target changed after the plan was created".to_owned(),
        ));
    }
    if actual.entry_type != super::model::FingerprintType::Symlink
        || actual.link_target.as_ref() != Some(source)
    {
        return Err(CoreError::InvalidPath(
            "RemoveManagedLink may only unlink the exact planned source link".to_owned(),
        ));
    }
    ensure_source_matches_plan(action, source)?;
    let raw_link_target = actual.link_target.expect("checked above");
    fs::remove_file(target.path.as_std_path())?;
    Ok((target.path.clone(), raw_link_target))
}

/// Adopt an equivalent unmanaged target only after the caller has selected its stable action ID.
/// The old target is moved to a private backup before the replacement link is made, so a later
/// failure (including the ledger batch) can restore it without copying user data.
fn apply_adopt_equivalent(
    action: &ProjectionAction,
    context: &ExecutorContext<'_>,
) -> Result<(Utf8PathBuf, Utf8PathBuf, Utf8PathBuf), CoreError> {
    let target = action
        .target
        .as_ref()
        .ok_or_else(|| CoreError::InvalidPath("AdoptEquivalent action has no target".to_owned()))?;
    let source = action
        .members
        .first()
        .map(|member| &member.source.absolute_path)
        .ok_or_else(|| {
            CoreError::InvalidPath("AdoptEquivalent action has no source member".to_owned())
        })?;
    ensure_target_is_allowed(&target.path, &context.deploy_base)?;
    let expected = action.precondition.as_ref().ok_or_else(|| {
        CoreError::InvalidPath("AdoptEquivalent action has no target precondition".to_owned())
    })?;
    if &path_fingerprint(&target.path)? != expected {
        return Err(CoreError::InvalidPath(
            "projection target changed after the plan was created".to_owned(),
        ));
    }
    if !source.exists() {
        return Err(CoreError::ConfigNotFound {
            path: source.to_string(),
            hint: "重新生成投影计划，确认 canonical source 仍存在".to_owned(),
        });
    }
    ensure_source_matches_plan(action, source)?;

    let backup = allocate_backup_path(&context.backup_root, &target.path)?;
    write_backup_manifest(&backup, &target.path, expected)?;
    fs::rename(target.path.as_std_path(), backup.as_std_path())?;
    let parent = match ensure_safe_target_parent(&target.path, &context.deploy_base) {
        Ok(parent) => parent,
        Err(error) => {
            let _ = fs::rename(backup.as_std_path(), target.path.as_std_path());
            return Err(error);
        }
    };
    if let Err(error) = create_sibling_symlink(source, &target.path, &parent.path) {
        let _ = fs::rename(backup.as_std_path(), target.path.as_std_path());
        return Err(error);
    }
    Ok((target.path.clone(), source.clone(), backup))
}

/// Materialize an explicitly planned copy without following any source symlink. This is not a
/// fallback inside link creation: the action kind and the reviewed plan both make the copied
/// state visible before any bytes are written.
fn apply_copy_fallback(
    action: &ProjectionAction,
    context: &ExecutorContext<'_>,
) -> Result<CreatedCopy, CoreError> {
    let target = action
        .target
        .as_ref()
        .ok_or_else(|| CoreError::InvalidPath("CopyFallback action has no target".to_owned()))?;
    let source = action
        .members
        .first()
        .map(|member| &member.source.absolute_path)
        .ok_or_else(|| {
            CoreError::InvalidPath("CopyFallback action has no source member".to_owned())
        })?;
    ensure_target_is_allowed(&target.path, &context.deploy_base)?;
    let expected = action.precondition.as_ref().ok_or_else(|| {
        CoreError::InvalidPath("CopyFallback action has no target precondition".to_owned())
    })?;
    let actual = path_fingerprint(&target.path)?;
    if &actual != expected {
        return Err(CoreError::InvalidPath(
            "projection target changed after the plan was created".to_owned(),
        ));
    }
    let replacing_proven_copy = action.state.as_deref() == Some("copied")
        && action.kind == ProjectionActionKind::CopyFallback;
    if actual.entry_type != super::model::FingerprintType::Missing && !replacing_proven_copy {
        return Err(CoreError::InvalidPath(
            "CopyFallback may only create a missing target or refresh a ledger-proven copy"
                .to_owned(),
        ));
    }
    if actual.entry_type != super::model::FingerprintType::Missing
        && action.ownership_fingerprint.as_deref() != actual.digest.as_deref()
    {
        return Err(CoreError::InvalidPath(
            "CopyFallback refresh requires a matching ledger ownership digest".to_owned(),
        ));
    }
    if !source.exists() {
        return Err(CoreError::ConfigNotFound {
            path: source.to_string(),
            hint: "重新生成投影计划，确认 canonical source 仍存在".to_owned(),
        });
    }
    ensure_source_matches_plan(action, source)?;

    let parent = ensure_safe_target_parent(&target.path, &context.deploy_base)?;
    let temporary = copy_temporary_path(&parent.path, &target.path)?;
    if let Err(error) = copy_path_without_symlinks(source, &temporary) {
        let _ = remove_copied_path(&temporary, None);
        return Err(error);
    }
    let copied = path_fingerprint(&temporary)?;
    let digest = copied.digest.ok_or_else(|| {
        CoreError::InvalidPath("temporary copied target has no digest".to_owned())
    })?;

    if actual.entry_type == super::model::FingerprintType::Missing {
        if let Err(error) = fs::rename(temporary.as_std_path(), target.path.as_std_path()) {
            let _ = remove_copied_path(&temporary, None);
            return Err(CoreError::Io(error));
        }
        return Ok(CreatedCopy {
            target: target.path.clone(),
            digest,
            created_parents: parent.created_parents,
            replaced_backup: None,
        });
    }

    let backup = allocate_backup_path(&context.backup_root, &target.path)?;
    write_backup_manifest(&backup, &target.path, expected)?;
    fs::rename(target.path.as_std_path(), backup.as_std_path())?;
    if let Err(error) = fs::rename(temporary.as_std_path(), target.path.as_std_path()) {
        let _ = fs::rename(backup.as_std_path(), target.path.as_std_path());
        let _ = remove_copied_path(&temporary, None);
        return Err(CoreError::Io(error));
    }
    Ok(CreatedCopy {
        target: target.path.clone(),
        digest,
        created_parents: parent.created_parents,
        replaced_backup: Some(backup),
    })
}

/// A copied target is never unlinked by shape alone. The planner puts the ledger target digest
/// in `ownership_fingerprint`; apply verifies it again immediately before moving it to backup.
fn apply_remove_managed_copy(
    action: &ProjectionAction,
    context: &ExecutorContext<'_>,
) -> Result<(Utf8PathBuf, String, Utf8PathBuf), CoreError> {
    let target = action.target.as_ref().ok_or_else(|| {
        CoreError::InvalidPath("RemoveManagedCopy action has no target".to_owned())
    })?;
    ensure_target_is_allowed(&target.path, &context.deploy_base)?;
    let expected = action.precondition.as_ref().ok_or_else(|| {
        CoreError::InvalidPath("RemoveManagedCopy action has no target precondition".to_owned())
    })?;
    let actual = path_fingerprint(&target.path)?;
    if &actual != expected {
        return Err(CoreError::InvalidPath(
            "projection target changed after the plan was created".to_owned(),
        ));
    }
    if !matches!(
        actual.entry_type,
        super::model::FingerprintType::File | super::model::FingerprintType::Directory
    ) {
        return Err(CoreError::InvalidPath(
            "RemoveManagedCopy may only remove a regular file or directory".to_owned(),
        ));
    }
    let digest = actual.digest.clone().ok_or_else(|| {
        CoreError::InvalidPath("RemoveManagedCopy target has no digest".to_owned())
    })?;
    if action.ownership_fingerprint.as_deref() != Some(digest.as_str()) {
        return Err(CoreError::InvalidPath(
            "RemoveManagedCopy requires a matching ledger ownership digest".to_owned(),
        ));
    }
    ensure_path_has_no_symlinks(&target.path)?;
    let backup = allocate_backup_path(&context.backup_root, &target.path)?;
    write_backup_manifest(&backup, &target.path, expected)?;
    fs::rename(target.path.as_std_path(), backup.as_std_path())?;
    Ok((target.path.clone(), digest, backup))
}

fn ensure_source_matches_plan(
    action: &ProjectionAction,
    source: &Utf8Path,
) -> Result<(), CoreError> {
    let member = action.members.first().ok_or_else(|| {
        CoreError::InvalidPath("projection action has no source member".to_owned())
    })?;
    let actual = path_content_digest(source)?;
    if actual != member.source.fingerprint {
        return Err(CoreError::InvalidPath(
            "canonical source changed after the plan was created".to_owned(),
        ));
    }
    Ok(())
}

struct AppliedGeneratedMcp {
    mutations: Vec<LedgerMutation>,
    undo: Option<FileUndo>,
    skipped_members: Vec<McpSkippedMemberReport>,
}

struct AppliedGeneratedHook {
    mutations: Vec<LedgerMutation>,
    undo: Option<FileUndo>,
}

struct AppliedHermesUnifiedYaml {
    mutations: Vec<LedgerMutation>,
    undo: Option<FileUndo>,
    skipped_members: Vec<McpSkippedMemberReport>,
}

/// Render all supported Hermes user domains in memory and replace config.yaml exactly once.
/// The plan carries only source references and secret key names; MCP values are hydrated only
/// after source fingerprints have been revalidated here.
fn apply_hermes_unified_yaml_upsert(
    action: &ProjectionAction,
    context: &ExecutorContext<'_>,
) -> Result<AppliedHermesUnifiedYaml, CoreError> {
    if action.generated_renderer != Some(GeneratedContainerRenderer::HermesUnifiedYaml) {
        return Err(CoreError::InvalidPath(
            "Hermes unified action has the wrong renderer".to_owned(),
        ));
    }
    let target = action
        .target
        .as_ref()
        .ok_or_else(|| CoreError::InvalidPath("Hermes unified action has no target".to_owned()))?;
    let expected = action.precondition.as_ref().ok_or_else(|| {
        CoreError::InvalidPath("Hermes unified action has no target precondition".to_owned())
    })?;
    ensure_target_is_allowed(&target.path, &context.deploy_base)?;
    let actual = path_fingerprint(&target.path)?;
    if &actual != expected {
        return Err(CoreError::InvalidPath(
            "projection target changed after the plan was created".to_owned(),
        ));
    }
    if !matches!(
        actual.entry_type,
        super::model::FingerprintType::Missing | super::model::FingerprintType::File
    ) {
        return Err(CoreError::InvalidPath(
            "Hermes config.yaml must be a regular file or be absent".to_owned(),
        ));
    }
    for member in &action.members {
        ensure_hook_member_source_matches_plan(member)?;
    }
    let existing = match actual.entry_type {
        super::model::FingerprintType::Missing => String::new(),
        super::model::FingerprintType::File => fs::read_to_string(target.path.as_std_path())?,
        _ => unreachable!("validated above"),
    };
    let mut intents = Vec::new();
    let mut skipped_members = Vec::new();
    for member in &action.mcp_members {
        match load_and_hydrate_mcp_member(member, context)? {
            HydratedMcpMember::Ready(intent) => intents.push(intent),
            HydratedMcpMember::MissingSecrets {
                entry_key,
                missing_secret_keys,
            } => skipped_members.push(McpSkippedMemberReport {
                entry_key,
                missing_secret_keys,
            }),
        }
    }
    let rendered = render_hermes_unified_yaml(&existing, &action.members, &intents)?;
    let parent = ensure_safe_target_parent(&target.path, &context.deploy_base)?;
    let temporary = generated_temporary_path(&parent.path, &target.path)?;
    if let Err(error) = write_private_generated_file(&temporary, rendered.as_bytes()) {
        let _ = fs::remove_file(temporary.as_std_path());
        return Err(error);
    }
    let rendered_digest = path_fingerprint(&temporary)?
        .digest
        .ok_or_else(|| CoreError::InvalidPath("rendered Hermes config has no digest".to_owned()))?;
    let mut mutations = action
        .members
        .iter()
        .map(|member| {
            LedgerMutation::Upsert(ProjectionRecord {
                id: member.id.clone(),
                mode: ProjectionMode::GeneratedYaml,
                source_path: member.source.absolute_path.clone(),
                target_path: target.path.clone(),
                entry_key: member.entry_key.clone(),
                source_fingerprint: member.source.fingerprint.clone(),
                entry_fingerprint: None,
                target_fingerprint: rendered_digest.clone(),
                applied_at: Utc::now(),
            })
        })
        .collect::<Vec<_>>();
    let applied_names = intents
        .iter()
        .map(|intent| intent.name.as_str())
        .collect::<BTreeSet<_>>();
    for member in &action.mcp_members {
        if !applied_names.contains(member.name.as_str()) {
            continue;
        }
        mutations.push(LedgerMutation::Upsert(ProjectionRecord {
            id: member.id.clone(),
            mode: ProjectionMode::GeneratedYaml,
            source_path: member.source.absolute_path.clone(),
            target_path: target.path.clone(),
            entry_key: Some(member.entry_key.clone()),
            source_fingerprint: member.source.fingerprint.clone(),
            entry_fingerprint: None,
            target_fingerprint: rendered_digest.clone(),
            applied_at: Utc::now(),
        }));
    }
    let backup = if actual.entry_type == super::model::FingerprintType::File {
        let backup = allocate_backup_path(&context.backup_root, &target.path)?;
        write_backup_manifest(&backup, &target.path, expected)?;
        if let Err(error) = fs::rename(target.path.as_std_path(), backup.as_std_path()) {
            let _ = fs::remove_file(temporary.as_std_path());
            return Err(CoreError::Io(error));
        }
        if let Err(error) = set_private_file_permissions(&backup) {
            let _ = fs::rename(backup.as_std_path(), target.path.as_std_path());
            let _ = fs::remove_file(temporary.as_std_path());
            return Err(error);
        }
        Some(backup)
    } else {
        None
    };
    if let Err(error) = fs::rename(temporary.as_std_path(), target.path.as_std_path()) {
        if let Some(backup) = &backup {
            let _ = fs::rename(backup.as_std_path(), target.path.as_std_path());
        }
        let _ = fs::remove_file(temporary.as_std_path());
        return Err(CoreError::Io(error));
    }
    Ok(AppliedHermesUnifiedYaml {
        mutations,
        undo: Some(FileUndo::RestoreGenerated {
            target: target.path.clone(),
            rendered_digest,
            backup,
            created_parents: parent.created_parents,
        }),
        skipped_members,
    })
}

fn apply_hermes_unified_yaml_retraction(
    action: &ProjectionAction,
    context: &ExecutorContext<'_>,
) -> Result<AppliedHermesUnifiedYaml, CoreError> {
    if action.generated_renderer != Some(GeneratedContainerRenderer::HermesUnifiedYaml) {
        return Err(CoreError::InvalidPath(
            "Hermes unified removal has the wrong renderer".to_owned(),
        ));
    }
    let target = action
        .target
        .as_ref()
        .ok_or_else(|| CoreError::InvalidPath("Hermes unified removal has no target".to_owned()))?;
    let expected = action.precondition.as_ref().ok_or_else(|| {
        CoreError::InvalidPath("Hermes unified removal has no target precondition".to_owned())
    })?;
    ensure_target_is_allowed(&target.path, &context.deploy_base)?;
    let actual = path_fingerprint(&target.path)?;
    if &actual != expected || actual.entry_type != super::model::FingerprintType::File {
        return Err(CoreError::InvalidPath(
            "Hermes unified removal requires the exact planned config.yaml file".to_owned(),
        ));
    }
    let existing = fs::read_to_string(target.path.as_std_path())?;
    let rendered =
        render_hermes_unified_yaml_removal(&existing, &action.members, &action.mcp_members)?;
    let parent = ensure_safe_target_parent(&target.path, &context.deploy_base)?;
    let temporary = generated_temporary_path(&parent.path, &target.path)?;
    if let Err(error) = write_private_generated_file(&temporary, rendered.as_bytes()) {
        let _ = fs::remove_file(temporary.as_std_path());
        return Err(error);
    }
    let rendered_digest = path_fingerprint(&temporary)?
        .digest
        .ok_or_else(|| CoreError::InvalidPath("rendered Hermes config has no digest".to_owned()))?;
    let mut mutations = action
        .members
        .iter()
        .map(|member| LedgerMutation::Remove(member.id.clone()))
        .collect::<Vec<_>>();
    mutations.extend(
        action
            .mcp_members
            .iter()
            .map(|member| LedgerMutation::Remove(member.id.clone())),
    );
    let backup = allocate_backup_path(&context.backup_root, &target.path)?;
    write_backup_manifest(&backup, &target.path, expected)?;
    if let Err(error) = fs::rename(target.path.as_std_path(), backup.as_std_path()) {
        let _ = fs::remove_file(temporary.as_std_path());
        return Err(CoreError::Io(error));
    }
    if let Err(error) = set_private_file_permissions(&backup) {
        let _ = fs::rename(backup.as_std_path(), target.path.as_std_path());
        let _ = fs::remove_file(temporary.as_std_path());
        return Err(error);
    }
    if let Err(error) = fs::rename(temporary.as_std_path(), target.path.as_std_path()) {
        let _ = fs::rename(backup.as_std_path(), target.path.as_std_path());
        let _ = fs::remove_file(temporary.as_std_path());
        return Err(CoreError::Io(error));
    }
    Ok(AppliedHermesUnifiedYaml {
        mutations,
        undo: Some(FileUndo::RestoreGenerated {
            target: target.path.clone(),
            rendered_digest,
            backup: Some(backup),
            created_parents: parent.created_parents,
        }),
        skipped_members: Vec::new(),
    })
}

fn render_hermes_unified_yaml(
    existing: &str,
    members: &[super::planner::ProjectionMember],
    mcp_intents: &[HydratedMcpIntent],
) -> Result<String, CoreError> {
    let mut root = if existing.trim().is_empty() {
        YamlValue::Mapping(Mapping::new())
    } else {
        serde_yaml::from_str::<YamlValue>(existing).map_err(|_| {
            CoreError::InvalidPath("Hermes config.yaml must be valid YAML".to_owned())
        })?
    };
    let mapping = root.as_mapping_mut().ok_or_else(|| {
        CoreError::InvalidPath("Hermes config.yaml root must be a mapping".to_owned())
    })?;
    let key = |name: &str| YamlValue::String(name.to_owned());

    let mcp_servers = mapping
        .entry(key("mcp_servers"))
        .or_insert_with(|| YamlValue::Mapping(Mapping::new()))
        .as_mapping_mut()
        .ok_or_else(|| CoreError::InvalidPath("Hermes mcp_servers must be a mapping".to_owned()))?;
    for intent in mcp_intents {
        mcp_servers.insert(
            key(&intent.name),
            serde_yaml::to_value(&intent.config)
                .map_err(|error| CoreError::InvalidPath(error.to_string()))?,
        );
    }

    let skills = mapping
        .entry(key("skills"))
        .or_insert_with(|| YamlValue::Mapping(Mapping::new()))
        .as_mapping_mut()
        .ok_or_else(|| CoreError::InvalidPath("Hermes skills must be a mapping".to_owned()))?;
    let external_dirs = skills
        .entry(key("external_dirs"))
        .or_insert_with(|| YamlValue::Sequence(Vec::new()))
        .as_sequence_mut()
        .ok_or_else(|| {
            CoreError::InvalidPath("Hermes skills.external_dirs must be a sequence".to_owned())
        })?;
    for member in members
        .iter()
        .filter(|member| member.id.kind == crate::model::AssetKind::Skill)
    {
        let path = YamlValue::String(member.source.absolute_path.to_string());
        if !external_dirs.contains(&path) {
            external_dirs.push(path);
        }
    }

    let hooks = mapping
        .entry(key("hooks"))
        .or_insert_with(|| YamlValue::Mapping(Mapping::new()))
        .as_mapping_mut()
        .ok_or_else(|| CoreError::InvalidPath("Hermes hooks must be a mapping".to_owned()))?;
    let lifecycle = hooks
        .entry(key("PostToolUse"))
        .or_insert_with(|| YamlValue::Sequence(Vec::new()))
        .as_sequence_mut()
        .ok_or_else(|| {
            CoreError::InvalidPath("Hermes Hook entries must be a sequence".to_owned())
        })?;
    for member in members
        .iter()
        .filter(|member| member.id.kind == crate::model::AssetKind::Hook)
    {
        lifecycle.retain(|entry| !yaml_hook_is_managed(entry, &member.id.name));
        let mut binding = Mapping::new();
        binding.insert(
            key("command"),
            YamlValue::String(format!(".hermes/hooks/{}", member.id.name)),
        );
        binding.insert(key("managedBy"), YamlValue::String("ai-config".to_owned()));
        binding.insert(key("hook"), YamlValue::String(member.id.name.clone()));
        lifecycle.push(YamlValue::Mapping(binding));
    }
    serde_yaml::to_string(&root).map_err(|error| CoreError::InvalidPath(error.to_string()))
}

fn render_hermes_unified_yaml_removal(
    existing: &str,
    members: &[super::planner::ProjectionMember],
    mcp_members: &[McpProjectionMember],
) -> Result<String, CoreError> {
    let mut root = serde_yaml::from_str::<YamlValue>(existing)
        .map_err(|_| CoreError::InvalidPath("Hermes config.yaml must be valid YAML".to_owned()))?;
    let mapping = root.as_mapping_mut().ok_or_else(|| {
        CoreError::InvalidPath("Hermes config.yaml root must be a mapping".to_owned())
    })?;
    let key = |name: &str| YamlValue::String(name.to_owned());
    if let Some(servers) = mapping
        .get_mut(key("mcp_servers"))
        .and_then(YamlValue::as_mapping_mut)
    {
        for member in mcp_members {
            servers.remove(key(&member.name));
        }
    }
    if let Some(external_dirs) = mapping
        .get_mut(key("skills"))
        .and_then(YamlValue::as_mapping_mut)
        .and_then(|skills| skills.get_mut(key("external_dirs")))
        .and_then(YamlValue::as_sequence_mut)
    {
        for member in members
            .iter()
            .filter(|member| member.id.kind == crate::model::AssetKind::Skill)
        {
            external_dirs
                .retain(|entry| entry.as_str() != Some(member.source.absolute_path.as_str()));
        }
    }
    if let Some(hooks) = mapping
        .get_mut(key("hooks"))
        .and_then(YamlValue::as_mapping_mut)
    {
        for entries in hooks.values_mut() {
            let Some(entries) = entries.as_sequence_mut() else {
                continue;
            };
            for member in members
                .iter()
                .filter(|member| member.id.kind == crate::model::AssetKind::Hook)
            {
                entries.retain(|entry| !yaml_hook_is_managed(entry, &member.id.name));
            }
        }
    }
    serde_yaml::to_string(&root).map_err(|error| CoreError::InvalidPath(error.to_string()))
}

fn yaml_hook_is_managed(entry: &YamlValue, name: &str) -> bool {
    let Some(mapping) = entry.as_mapping() else {
        return false;
    };
    mapping
        .get(YamlValue::String("managedBy".to_owned()))
        .and_then(YamlValue::as_str)
        == Some("ai-config")
        && mapping
            .get(YamlValue::String("hook".to_owned()))
            .and_then(YamlValue::as_str)
            == Some(name)
}

/// Hook bindings are source-free generated entries: the canonical script/bundle is represented
/// by the paired direct-link action, while this action owns only a named, marked binding.
fn apply_hook_generated_upsert(
    action: &ProjectionAction,
    context: &ExecutorContext<'_>,
) -> Result<AppliedGeneratedHook, CoreError> {
    let (target, expected, actual, existing) = hook_target_for_write(action, context)?;
    for member in &action.members {
        ensure_hook_member_source_matches_plan(member)?;
    }
    let rendered = render_hook_json(&existing, &action.members, &[])?;
    replace_hook_target(action, context, target, expected, actual, rendered, false)
}

fn apply_hook_generated_retraction(
    action: &ProjectionAction,
    context: &ExecutorContext<'_>,
) -> Result<AppliedGeneratedHook, CoreError> {
    let (target, expected, actual, existing) = hook_target_for_write(action, context)?;
    let removals = action
        .members
        .iter()
        .map(|member| member.id.name.clone())
        .collect::<Vec<_>>();
    let rendered = render_hook_json(&existing, &[], &removals)?;
    replace_hook_target(action, context, target, expected, actual, rendered, true)
}

fn hook_target_for_write<'a>(
    action: &'a ProjectionAction,
    context: &ExecutorContext<'_>,
) -> Result<
    (
        &'a super::model::ProjectionTarget,
        &'a super::model::PathFingerprint,
        super::model::PathFingerprint,
        String,
    ),
    CoreError,
> {
    if action.members.is_empty()
        || !action.mcp_members.is_empty()
        || action.generated_renderer != Some(GeneratedContainerRenderer::HookJson)
    {
        return Err(CoreError::InvalidPath(
            "generated Hook action is not a source-first Hook JSON batch".to_owned(),
        ));
    }
    let target = action
        .target
        .as_ref()
        .ok_or_else(|| CoreError::InvalidPath("Hook generated action has no target".to_owned()))?;
    let expected = action.precondition.as_ref().ok_or_else(|| {
        CoreError::InvalidPath("Hook generated action has no target precondition".to_owned())
    })?;
    ensure_target_is_allowed(&target.path, &context.deploy_base)?;
    let actual = path_fingerprint(&target.path)?;
    if &actual != expected {
        return Err(CoreError::InvalidPath(
            "projection target changed after the plan was created".to_owned(),
        ));
    }
    if !matches!(
        actual.entry_type,
        super::model::FingerprintType::Missing | super::model::FingerprintType::File
    ) {
        return Err(CoreError::InvalidPath(
            "Hook generated target must be a regular file or be absent".to_owned(),
        ));
    }
    let existing = match actual.entry_type {
        super::model::FingerprintType::Missing => "{\"hooks\":{}}".to_owned(),
        super::model::FingerprintType::File => fs::read_to_string(target.path.as_std_path())?,
        _ => unreachable!("validated above"),
    };
    Ok((target, expected, actual, existing))
}

fn ensure_hook_member_source_matches_plan(
    member: &super::planner::ProjectionMember,
) -> Result<(), CoreError> {
    if path_content_digest(&member.source.absolute_path)? != member.source.fingerprint {
        return Err(CoreError::InvalidPath(
            "canonical Hook source changed after the plan was created".to_owned(),
        ));
    }
    Ok(())
}

fn render_hook_json(
    existing: &str,
    additions: &[super::planner::ProjectionMember],
    removals: &[String],
) -> Result<String, CoreError> {
    let mut document: Value = serde_json::from_str(existing).map_err(|_| {
        CoreError::InvalidPath("Hook generated container must be valid JSON".to_owned())
    })?;
    let root = document.as_object_mut().ok_or_else(|| {
        CoreError::InvalidPath("Hook generated container must be a JSON object".to_owned())
    })?;
    let hooks = root
        .entry("hooks".to_owned())
        .or_insert_with(|| Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .ok_or_else(|| CoreError::InvalidPath("Hook `hooks` field must be an object".to_owned()))?;
    let removals = removals.iter().collect::<BTreeSet<_>>();
    for entries in hooks.values_mut() {
        let entries = entries.as_array_mut().ok_or_else(|| {
            CoreError::InvalidPath("Hook lifecycle entries must be arrays".to_owned())
        })?;
        entries.retain(|entry| !hook_entry_is_owned_by(entry, &removals));
    }
    hooks.retain(|_, entries| {
        entries
            .as_array()
            .is_some_and(|entries| !entries.is_empty())
    });

    for member in additions {
        let platform = match &member.id.surface {
            ProjectionSurface::PlatformBinding { platform, .. } => *platform,
            _ => {
                return Err(CoreError::InvalidPath(
                    "Hook generated member has no platform binding surface".to_owned(),
                ))
            }
        };
        let lifecycle = match platform {
            crate::model::PlatformId::Cursor => "afterShellExecution",
            crate::model::PlatformId::Codex | crate::model::PlatformId::Claude => "PostToolUse",
            crate::model::PlatformId::Hermes | crate::model::PlatformId::AiConfig => {
                return Err(CoreError::NotImplemented(
                    "Hook YAML projection requires its dedicated transactional renderer",
                ))
            }
        };
        let command = match platform {
            crate::model::PlatformId::Cursor => format!(".cursor/hooks/{}", member.id.name),
            crate::model::PlatformId::Codex => format!(".codex/hooks/{}", member.id.name),
            crate::model::PlatformId::Claude => format!(".claude/hooks/{}", member.id.name),
            crate::model::PlatformId::Hermes | crate::model::PlatformId::AiConfig => unreachable!(),
        };
        let entry = if platform == crate::model::PlatformId::Cursor {
            serde_json::json!({
                "command": command,
                "managedBy": "ai-config",
                "hook": member.id.name,
            })
        } else {
            serde_json::json!({
                "hooks": [{
                    "type": "command",
                    "command": command,
                    "managedBy": "ai-config",
                    "hook": member.id.name,
                }]
            })
        };
        hooks
            .entry(lifecycle.to_owned())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| {
                CoreError::InvalidPath("Hook lifecycle entries must be arrays".to_owned())
            })?
            .push(entry);
    }
    serde_json::to_string_pretty(&document)
        .map_err(|error| CoreError::InvalidPath(error.to_string()))
}

fn hook_entry_is_owned_by(entry: &Value, removals: &BTreeSet<&String>) -> bool {
    let owned = |entry: &Value| {
        entry.get("managedBy").and_then(Value::as_str) == Some("ai-config")
            && entry
                .get("hook")
                .and_then(Value::as_str)
                .is_some_and(|name| removals.iter().any(|candidate| candidate.as_str() == name))
    };
    owned(entry)
        || entry
            .get("hooks")
            .and_then(Value::as_array)
            .is_some_and(|nested| nested.iter().any(owned))
}

fn replace_hook_target(
    action: &ProjectionAction,
    context: &ExecutorContext<'_>,
    target: &super::model::ProjectionTarget,
    expected: &super::model::PathFingerprint,
    actual: super::model::PathFingerprint,
    rendered: String,
    retract: bool,
) -> Result<AppliedGeneratedHook, CoreError> {
    let parent = ensure_safe_target_parent(&target.path, &context.deploy_base)?;
    let temporary = generated_temporary_path(&parent.path, &target.path)?;
    if let Err(error) = write_private_generated_file(&temporary, rendered.as_bytes()) {
        let _ = fs::remove_file(temporary.as_std_path());
        return Err(error);
    }
    let rendered_digest = path_fingerprint(&temporary)?
        .digest
        .ok_or_else(|| CoreError::InvalidPath("rendered Hook target has no digest".to_owned()))?;
    let mutations = if retract {
        action
            .members
            .iter()
            .map(|member| LedgerMutation::Remove(member.id.clone()))
            .collect()
    } else {
        action
            .members
            .iter()
            .map(|member| {
                Ok(LedgerMutation::Upsert(ProjectionRecord {
                    id: member.id.clone(),
                    mode: ProjectionMode::GeneratedJson,
                    source_path: member.source.absolute_path.clone(),
                    target_path: target.path.clone(),
                    entry_key: member.entry_key.clone(),
                    source_fingerprint: member.source.fingerprint.clone(),
                    entry_fingerprint: None,
                    target_fingerprint: rendered_digest.clone(),
                    applied_at: Utc::now(),
                }))
            })
            .collect::<Result<Vec<_>, CoreError>>()?
    };
    let backup = if actual.entry_type == super::model::FingerprintType::File {
        let backup = allocate_backup_path(&context.backup_root, &target.path)?;
        write_backup_manifest(&backup, &target.path, expected)?;
        if let Err(error) = fs::rename(target.path.as_std_path(), backup.as_std_path()) {
            let _ = fs::remove_file(temporary.as_std_path());
            return Err(CoreError::Io(error));
        }
        if let Err(error) = set_private_file_permissions(&backup) {
            let _ = fs::rename(backup.as_std_path(), target.path.as_std_path());
            let _ = fs::remove_file(temporary.as_std_path());
            return Err(error);
        }
        Some(backup)
    } else {
        None
    };
    if let Err(error) = fs::rename(temporary.as_std_path(), target.path.as_std_path()) {
        if let Some(backup) = &backup {
            let _ = fs::rename(backup.as_std_path(), target.path.as_std_path());
        }
        let _ = fs::remove_file(temporary.as_std_path());
        return Err(CoreError::Io(error));
    }
    Ok(AppliedGeneratedHook {
        mutations,
        undo: Some(FileUndo::RestoreGenerated {
            target: target.path.clone(),
            rendered_digest,
            backup,
            created_parents: parent.created_parents,
        }),
    })
}

/// Apply exactly one plan-bound MCP container batch. Non-MCP generated actions stay fail-closed
/// until their own renderer/ownership transaction exists.
fn apply_mcp_generated_upsert(
    action: &ProjectionAction,
    context: &ExecutorContext<'_>,
) -> Result<AppliedGeneratedMcp, CoreError> {
    if !action.members.is_empty() || action.mcp_members.is_empty() {
        return Err(CoreError::InvalidPath(
            "generated upsert is not a source-first MCP batch".to_owned(),
        ));
    }
    let renderer = action
        .generated_renderer
        .ok_or_else(|| CoreError::InvalidPath("MCP generated action has no renderer".to_owned()))?;
    if !matches!(
        renderer,
        GeneratedContainerRenderer::McpJson
            | GeneratedContainerRenderer::McpToml
            | GeneratedContainerRenderer::McpYaml
    ) {
        return Err(CoreError::InvalidPath(
            "generated upsert renderer is not an MCP container renderer".to_owned(),
        ));
    }
    let target = action
        .target
        .as_ref()
        .ok_or_else(|| CoreError::InvalidPath("MCP generated action has no target".to_owned()))?;
    let expected = action.precondition.as_ref().ok_or_else(|| {
        CoreError::InvalidPath("MCP generated action has no target precondition".to_owned())
    })?;
    ensure_target_is_allowed(&target.path, &context.deploy_base)?;
    let actual = path_fingerprint(&target.path)?;
    if &actual != expected {
        return Err(CoreError::InvalidPath(
            "projection target changed after the plan was created".to_owned(),
        ));
    }
    if !matches!(
        actual.entry_type,
        super::model::FingerprintType::Missing | super::model::FingerprintType::File
    ) {
        return Err(CoreError::InvalidPath(
            "MCP generated target must be a regular file or be absent".to_owned(),
        ));
    }

    // Re-validate every canonical file before reading any target or allocating a temporary path.
    // This makes a plan stale rather than allowing a changed source to be rendered implicitly.
    let hydrated_members = action
        .mcp_members
        .iter()
        .map(|member| load_and_hydrate_mcp_member(member, context))
        .collect::<Result<Vec<_>, _>>()?;
    let mut intents = Vec::new();
    let mut skipped_members = Vec::new();
    for hydrated in hydrated_members {
        match hydrated {
            HydratedMcpMember::Ready(intent) => intents.push(intent),
            HydratedMcpMember::MissingSecrets {
                entry_key,
                missing_secret_keys,
            } => skipped_members.push(McpSkippedMemberReport {
                entry_key,
                missing_secret_keys,
            }),
        }
    }
    if intents.is_empty() {
        return Ok(AppliedGeneratedMcp {
            mutations: Vec::new(),
            undo: None,
            skipped_members,
        });
    }
    let applied_member_names = intents
        .iter()
        .map(|intent| intent.name.as_str())
        .collect::<BTreeSet<_>>();
    let existing = match actual.entry_type {
        super::model::FingerprintType::Missing => empty_mcp_container(renderer),
        super::model::FingerprintType::File => fs::read_to_string(target.path.as_std_path())?,
        _ => unreachable!("validated above"),
    };
    let rendered = render_mcp_container(renderer, &existing, &intents, &[])?;
    let entry_fingerprints = inspect_mcp_entries(renderer, &rendered)?
        .into_iter()
        .map(|entry| (entry.name, entry.digest))
        .collect::<BTreeMap<_, _>>();

    let parent = ensure_safe_target_parent(&target.path, &context.deploy_base)?;
    let temporary = generated_temporary_path(&parent.path, &target.path)?;
    if let Err(error) = write_private_generated_file(&temporary, rendered.as_bytes()) {
        let _ = fs::remove_file(temporary.as_std_path());
        return Err(error);
    }
    let rendered_digest = path_fingerprint(&temporary)?
        .digest
        .ok_or_else(|| CoreError::InvalidPath("rendered MCP target has no digest".to_owned()))?;
    // All fallible ledger mutation construction is completed before target replacement. The
    // content digest is identical after the atomic rename, so no post-swap validation step can
    // strand a newly rendered container without a rollback record.
    let mode = mcp_projection_mode(renderer)?;
    let mutations = action
        .mcp_members
        .iter()
        .filter(|member| applied_member_names.contains(member.name.as_str()))
        .map(|member| {
            let entry_fingerprint =
                entry_fingerprints
                    .get(&member.name)
                    .cloned()
                    .ok_or_else(|| {
                        CoreError::InvalidPath(
                            "MCP renderer did not emit a planned server entry".to_owned(),
                        )
                    })?;
            Ok(LedgerMutation::Upsert(ProjectionRecord {
                id: member.id.clone(),
                mode,
                source_path: member.source.absolute_path.clone(),
                target_path: target.path.clone(),
                entry_key: Some(member.entry_key.clone()),
                source_fingerprint: member.source.fingerprint.clone(),
                entry_fingerprint: Some(entry_fingerprint),
                target_fingerprint: rendered_digest.clone(),
                applied_at: Utc::now(),
            }))
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    let backup = if actual.entry_type == super::model::FingerprintType::File {
        let backup = allocate_backup_path(&context.backup_root, &target.path)?;
        write_backup_manifest(&backup, &target.path, expected)?;
        if let Err(error) = fs::rename(target.path.as_std_path(), backup.as_std_path()) {
            let _ = fs::remove_file(temporary.as_std_path());
            return Err(CoreError::Io(error));
        }
        if let Err(error) = set_private_file_permissions(&backup) {
            let _ = fs::rename(backup.as_std_path(), target.path.as_std_path());
            let _ = fs::remove_file(temporary.as_std_path());
            return Err(error);
        }
        Some(backup)
    } else {
        None
    };
    if let Err(error) = fs::rename(temporary.as_std_path(), target.path.as_std_path()) {
        if let Some(backup) = &backup {
            let _ = fs::rename(backup.as_std_path(), target.path.as_std_path());
        }
        let _ = fs::remove_file(temporary.as_std_path());
        return Err(CoreError::Io(error));
    }

    Ok(AppliedGeneratedMcp {
        mutations,
        undo: Some(FileUndo::RestoreGenerated {
            target: target.path.clone(),
            rendered_digest,
            backup,
            created_parents: parent.created_parents,
        }),
        skipped_members,
    })
}

/// Retract only entries that the reviewed plan already proved against the ledger. This never
/// reads canonical source definitions or secrets: deletion is based on the plan-bound entry
/// names, the target precondition, and the entry-level ownership proof established by planning.
fn apply_mcp_generated_retraction(
    action: &ProjectionAction,
    context: &ExecutorContext<'_>,
) -> Result<AppliedGeneratedMcp, CoreError> {
    if !action.members.is_empty() || action.mcp_members.is_empty() {
        return Err(CoreError::InvalidPath(
            "generated removal is not a source-first MCP batch".to_owned(),
        ));
    }
    let renderer = action
        .generated_renderer
        .ok_or_else(|| CoreError::InvalidPath("MCP generated action has no renderer".to_owned()))?;
    if !matches!(
        renderer,
        GeneratedContainerRenderer::McpJson
            | GeneratedContainerRenderer::McpToml
            | GeneratedContainerRenderer::McpYaml
    ) {
        return Err(CoreError::InvalidPath(
            "generated removal renderer is not an MCP container renderer".to_owned(),
        ));
    }
    let target = action
        .target
        .as_ref()
        .ok_or_else(|| CoreError::InvalidPath("MCP generated action has no target".to_owned()))?;
    let expected = action.precondition.as_ref().ok_or_else(|| {
        CoreError::InvalidPath("MCP generated action has no target precondition".to_owned())
    })?;
    ensure_target_is_allowed(&target.path, &context.deploy_base)?;
    let actual = path_fingerprint(&target.path)?;
    if &actual != expected {
        return Err(CoreError::InvalidPath(
            "projection target changed after the plan was created".to_owned(),
        ));
    }
    if actual.entry_type != super::model::FingerprintType::File {
        return Err(CoreError::InvalidPath(
            "MCP generated removal requires an existing regular target file".to_owned(),
        ));
    }

    let existing = fs::read_to_string(target.path.as_std_path())?;
    let mut removals = action
        .mcp_members
        .iter()
        .map(|member| member.name.clone())
        .collect::<Vec<_>>();
    removals.sort();
    removals.dedup();
    let rendered = render_mcp_container(renderer, &existing, &[], &removals)?;
    let remaining_entries = inspect_mcp_entries(renderer, &rendered)?;
    if remaining_entries
        .iter()
        .any(|entry| removals.binary_search(&entry.name).is_ok())
    {
        return Err(CoreError::InvalidPath(
            "MCP renderer did not remove every planned server entry".to_owned(),
        ));
    }

    let parent = ensure_safe_target_parent(&target.path, &context.deploy_base)?;
    let temporary = generated_temporary_path(&parent.path, &target.path)?;
    if let Err(error) = write_private_generated_file(&temporary, rendered.as_bytes()) {
        let _ = fs::remove_file(temporary.as_std_path());
        return Err(error);
    }
    let rendered_digest = path_fingerprint(&temporary)?
        .digest
        .ok_or_else(|| CoreError::InvalidPath("rendered MCP target has no digest".to_owned()))?;
    let mutations = action
        .mcp_members
        .iter()
        .map(|member| LedgerMutation::Remove(member.id.clone()))
        .collect::<Vec<_>>();
    let backup = allocate_backup_path(&context.backup_root, &target.path)?;
    write_backup_manifest(&backup, &target.path, expected)?;
    if let Err(error) = fs::rename(target.path.as_std_path(), backup.as_std_path()) {
        let _ = fs::remove_file(temporary.as_std_path());
        return Err(CoreError::Io(error));
    }
    if let Err(error) = set_private_file_permissions(&backup) {
        let _ = fs::rename(backup.as_std_path(), target.path.as_std_path());
        let _ = fs::remove_file(temporary.as_std_path());
        return Err(error);
    }
    if let Err(error) = fs::rename(temporary.as_std_path(), target.path.as_std_path()) {
        let _ = fs::rename(backup.as_std_path(), target.path.as_std_path());
        let _ = fs::remove_file(temporary.as_std_path());
        return Err(CoreError::Io(error));
    }

    Ok(AppliedGeneratedMcp {
        mutations,
        undo: Some(FileUndo::RestoreGenerated {
            target: target.path.clone(),
            rendered_digest,
            backup: Some(backup),
            created_parents: parent.created_parents,
        }),
        skipped_members: Vec::new(),
    })
}

#[derive(Clone)]
struct HydratedMcpIntent {
    name: String,
    config: Value,
}

enum HydratedMcpMember {
    Ready(HydratedMcpIntent),
    MissingSecrets {
        entry_key: String,
        missing_secret_keys: Vec<String>,
    },
}

enum ResolvedMcpSecrets {
    Available(BTreeMap<String, String>),
    Missing(Vec<String>),
}

fn load_and_hydrate_mcp_member(
    member: &McpProjectionMember,
    context: &ExecutorContext<'_>,
) -> Result<HydratedMcpMember, CoreError> {
    if !member.missing_secret_keys.is_empty() {
        return Ok(HydratedMcpMember::MissingSecrets {
            entry_key: member.entry_key.clone(),
            missing_secret_keys: member.missing_secret_keys.clone(),
        });
    }
    let actual = path_content_digest(&member.source.absolute_path)?;
    if actual != member.source.fingerprint {
        return Err(CoreError::InvalidPath(
            "canonical source changed after the plan was created".to_owned(),
        ));
    }
    let definition = load_mcp_definition_at(&member.source.absolute_path)?;
    if definition.server.name != member.name {
        return Err(CoreError::InvalidPath(
            "canonical MCP source no longer matches the planned server name".to_owned(),
        ));
    }
    let mut expected_keys = member.secret_keys.clone();
    expected_keys.sort();
    expected_keys.dedup();
    if definition.server.secret_keys != expected_keys {
        return Err(CoreError::InvalidPath(
            "canonical MCP source secret references no longer match the plan".to_owned(),
        ));
    }
    match resolve_mcp_secrets(&expected_keys, context)? {
        ResolvedMcpSecrets::Available(secrets) => {
            let config = hydrate_mcp_config(definition.server.config, &expected_keys, &secrets)?;
            Ok(HydratedMcpMember::Ready(HydratedMcpIntent {
                name: member.name.clone(),
                config,
            }))
        }
        ResolvedMcpSecrets::Missing(missing_secret_keys) => Ok(HydratedMcpMember::MissingSecrets {
            entry_key: member.entry_key.clone(),
            missing_secret_keys,
        }),
    }
}

fn is_missing_secret_skip(action: &ProjectionAction) -> bool {
    action.state.as_deref() == Some("skipped")
        && action.reason_code == "mcp_missing_secret_keys"
        && !action.mcp_members.is_empty()
        && action
            .mcp_members
            .iter()
            .all(|member| !member.missing_secret_keys.is_empty())
}

fn resolve_mcp_secrets(
    secret_keys: &[String],
    context: &ExecutorContext<'_>,
) -> Result<ResolvedMcpSecrets, CoreError> {
    if secret_keys.is_empty() {
        return Ok(ResolvedMcpSecrets::Available(BTreeMap::new()));
    }
    let provider = context.mcp_secret_provider.ok_or_else(|| {
        CoreError::InvalidPath("MCP plan needs a caller-provided secret resolver".to_owned())
    })?;
    let mut secrets = BTreeMap::new();
    let mut missing = Vec::new();
    for key in secret_keys {
        match provider.resolve(key).map_err(|_| {
            CoreError::InvalidPath(format!("MCP secret lookup failed for declared key {key}"))
        })? {
            Some(value) => {
                secrets.insert(key.clone(), value);
            }
            None => missing.push(key.clone()),
        }
    }
    if missing.is_empty() {
        Ok(ResolvedMcpSecrets::Available(secrets))
    } else {
        Ok(ResolvedMcpSecrets::Missing(missing))
    }
}

fn hydrate_mcp_config(
    mut config: Value,
    secret_keys: &[String],
    secrets: &BTreeMap<String, String>,
) -> Result<Value, CoreError> {
    let root = config.as_object_mut().ok_or_else(|| {
        CoreError::InvalidPath("canonical MCP config must be an object".to_owned())
    })?;
    for field in ["env", "headers"] {
        let Some(values) = root.get_mut(field).and_then(Value::as_object_mut) else {
            continue;
        };
        for value in values.values_mut() {
            let placeholder = value.as_str().ok_or_else(|| {
                CoreError::InvalidPath(
                    "MCP secret reference is not a string placeholder".to_owned(),
                )
            })?;
            let key = placeholder
                .strip_prefix("${")
                .and_then(|value| value.strip_suffix('}'))
                .filter(|key| secret_keys.iter().any(|expected| expected == key))
                .ok_or_else(|| {
                    CoreError::InvalidPath(
                        "MCP secret reference does not match the plan".to_owned(),
                    )
                })?;
            let secret = secrets.get(key).cloned().ok_or_else(|| {
                CoreError::InvalidPath(
                    "MCP secret resolution did not cover a planned reference".to_owned(),
                )
            })?;
            *value = Value::String(secret);
        }
    }
    Ok(config)
}

fn render_mcp_container(
    renderer: GeneratedContainerRenderer,
    existing: &str,
    intents: &[HydratedMcpIntent],
    removals: &[String],
) -> Result<String, CoreError> {
    match renderer {
        GeneratedContainerRenderer::McpJson => render_cursor_mcp_json(
            existing,
            &intents
                .iter()
                .map(|intent| JsonServerIntent::new(&intent.name, intent.config.clone()))
                .collect::<Vec<_>>(),
            removals,
        ),
        GeneratedContainerRenderer::McpToml => render_codex_mcp_toml(
            existing,
            &intents
                .iter()
                .map(|intent| TomlServerIntent::new(&intent.name, intent.config.clone()))
                .collect::<Vec<_>>(),
            removals,
        ),
        GeneratedContainerRenderer::McpYaml => render_hermes_mcp_yaml(
            existing,
            &intents
                .iter()
                .map(|intent| YamlServerIntent::new(&intent.name, intent.config.clone()))
                .collect::<Vec<_>>(),
            removals,
        ),
        _ => Err(CoreError::InvalidPath(
            "generated upsert renderer is not an MCP container renderer".to_owned(),
        )),
    }
}

fn inspect_mcp_entries(
    renderer: GeneratedContainerRenderer,
    rendered: &str,
) -> Result<Vec<McpEntryFingerprint>, CoreError> {
    match renderer {
        GeneratedContainerRenderer::McpJson => inspect_cursor_mcp_entries(rendered),
        GeneratedContainerRenderer::McpToml => inspect_codex_mcp_entries(rendered),
        GeneratedContainerRenderer::McpYaml => inspect_hermes_mcp_entries(rendered),
        _ => Err(CoreError::InvalidPath(
            "generated upsert renderer is not an MCP container renderer".to_owned(),
        )),
    }
}

fn empty_mcp_container(renderer: GeneratedContainerRenderer) -> String {
    match renderer {
        GeneratedContainerRenderer::McpJson => "{\"mcpServers\":{}}".to_owned(),
        GeneratedContainerRenderer::McpToml | GeneratedContainerRenderer::McpYaml => String::new(),
        _ => String::new(),
    }
}

fn mcp_projection_mode(renderer: GeneratedContainerRenderer) -> Result<ProjectionMode, CoreError> {
    match renderer {
        GeneratedContainerRenderer::McpJson => Ok(ProjectionMode::GeneratedJson),
        GeneratedContainerRenderer::McpToml => Ok(ProjectionMode::GeneratedToml),
        GeneratedContainerRenderer::McpYaml => Ok(ProjectionMode::GeneratedYaml),
        _ => Err(CoreError::InvalidPath(
            "generated upsert renderer is not an MCP container renderer".to_owned(),
        )),
    }
}

fn generated_temporary_path(
    parent: &Utf8Path,
    target: &Utf8Path,
) -> Result<Utf8PathBuf, CoreError> {
    let filename = target
        .file_name()
        .ok_or_else(|| CoreError::InvalidPath("projection target has no file name".to_owned()))?;
    let sequence = TEMP_LINK_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(parent.join(format!(".{filename}.ai-config-generated-{sequence}.tmp")))
}

fn write_private_generated_file(path: &Utf8Path, content: &[u8]) -> Result<(), CoreError> {
    fs::write(path.as_std_path(), content)?;
    set_private_file_permissions(path)
}

fn set_private_file_permissions(path: &Utf8Path) -> Result<(), CoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path.as_std_path(), fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn copy_temporary_path(parent: &Utf8Path, target: &Utf8Path) -> Result<Utf8PathBuf, CoreError> {
    let filename = target
        .file_name()
        .ok_or_else(|| CoreError::InvalidPath("projection target has no file name".to_owned()))?;
    let sequence = TEMP_LINK_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(parent.join(format!(".{filename}.ai-config-copy-{sequence}.tmp")))
}

/// Copy only regular files/directories. In particular, never flatten or follow a symlink inside
/// a canonical tree: on Windows that would silently copy data outside the approved source.
fn copy_path_without_symlinks(source: &Utf8Path, destination: &Utf8Path) -> Result<(), CoreError> {
    let metadata = fs::symlink_metadata(source.as_std_path())?;
    if metadata.file_type().is_symlink() {
        return Err(CoreError::InvalidPath(
            "CopyFallback rejects symlinked canonical sources".to_owned(),
        ));
    }
    if metadata.is_file() {
        fs::copy(source.as_std_path(), destination.as_std_path())?;
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(CoreError::InvalidPath(
            "CopyFallback source must be a regular file or directory".to_owned(),
        ));
    }
    fs::create_dir(destination.as_std_path())?;
    copy_directory_without_symlinks(source, destination)
}

fn copy_directory_without_symlinks(
    source: &Utf8Path,
    destination: &Utf8Path,
) -> Result<(), CoreError> {
    for entry in fs::read_dir(source.as_std_path())? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let source_child = source.join(name.as_ref());
        let destination_child = destination.join(name.as_ref());
        let metadata = fs::symlink_metadata(source_child.as_std_path())?;
        if metadata.file_type().is_symlink() {
            return Err(CoreError::InvalidPath(
                "CopyFallback rejects source trees containing symlinks".to_owned(),
            ));
        }
        if metadata.is_dir() {
            fs::create_dir(destination_child.as_std_path())?;
            copy_directory_without_symlinks(&source_child, &destination_child)?;
        } else if metadata.is_file() {
            fs::copy(source_child.as_std_path(), destination_child.as_std_path())?;
        } else {
            return Err(CoreError::InvalidPath(
                "CopyFallback source tree contains a non-regular entry".to_owned(),
            ));
        }
    }
    Ok(())
}

fn ensure_path_has_no_symlinks(path: &Utf8Path) -> Result<(), CoreError> {
    let metadata = fs::symlink_metadata(path.as_std_path())?;
    if metadata.file_type().is_symlink() {
        return Err(CoreError::InvalidPath(
            "CopyFallback target contains a symlink".to_owned(),
        ));
    }
    if metadata.is_file() {
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(CoreError::InvalidPath(
            "CopyFallback target is not a regular file or directory".to_owned(),
        ));
    }
    for entry in fs::read_dir(path.as_std_path())? {
        let entry = entry?;
        let name = entry.file_name();
        let child = path.join(name.to_string_lossy().as_ref());
        ensure_path_has_no_symlinks(&child)?;
    }
    Ok(())
}

/// Remove a known copied path without ever following a symlink. If an expected digest is given,
/// the final target must still match it; this makes rollback fail closed under concurrent edits.
fn remove_copied_path(path: &Utf8Path, expected_digest: Option<&str>) -> Result<(), CoreError> {
    let fingerprint = path_fingerprint(path)?;
    if let Some(expected_digest) = expected_digest {
        if fingerprint.digest.as_deref() != Some(expected_digest) {
            return Err(CoreError::InvalidPath(
                "copied target changed before safe removal".to_owned(),
            ));
        }
    }
    ensure_path_has_no_symlinks(path)?;
    if fingerprint.entry_type == super::model::FingerprintType::File {
        fs::remove_file(path.as_std_path())?;
    } else if fingerprint.entry_type == super::model::FingerprintType::Directory {
        remove_directory_without_symlinks(path)?;
    } else {
        return Err(CoreError::InvalidPath(
            "copied target is not removable as a regular file or directory".to_owned(),
        ));
    }
    Ok(())
}

fn remove_directory_without_symlinks(path: &Utf8Path) -> Result<(), CoreError> {
    for entry in fs::read_dir(path.as_std_path())? {
        let entry = entry?;
        let name = entry.file_name();
        let child = path.join(name.to_string_lossy().as_ref());
        let metadata = fs::symlink_metadata(child.as_std_path())?;
        if metadata.file_type().is_symlink() {
            return Err(CoreError::InvalidPath(
                "CopyFallback refuses to remove a tree containing symlinks".to_owned(),
            ));
        }
        if metadata.is_dir() {
            remove_directory_without_symlinks(&child)?;
        } else if metadata.is_file() {
            fs::remove_file(child.as_std_path())?;
        } else {
            return Err(CoreError::InvalidPath(
                "CopyFallback target tree contains a non-regular entry".to_owned(),
            ));
        }
    }
    fs::remove_dir(path.as_std_path())?;
    Ok(())
}

fn allocate_backup_path(
    backup_root: &Utf8Path,
    target: &Utf8Path,
) -> Result<Utf8PathBuf, CoreError> {
    ensure_safe_backup_root(backup_root, target)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(backup_root.as_std_path(), fs::Permissions::from_mode(0o700))?;
    }
    let filename = target
        .file_name()
        .ok_or_else(|| CoreError::InvalidPath("projection target has no file name".to_owned()))?;
    for sequence in 0..1_024 {
        let slot = backup_root.join(format!("{sequence}-{filename}"));
        let manifest = slot.with_extension("manifest.json");
        let temporary = slot.with_extension("manifest.tmp");
        if path_exists_without_following(&manifest)? || path_exists_without_following(&temporary)? {
            continue;
        }
        match fs::create_dir(slot.as_std_path()) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;

                    fs::set_permissions(slot.as_std_path(), fs::Permissions::from_mode(0o700))?;
                }
                let nonce = backup_nonce()?;
                return Ok(slot.join(format!("payload-{nonce:016x}")));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(CoreError::Io(error)),
        }
    }
    Err(CoreError::InvalidPath(
        "could not atomically reserve a private backup slot".to_owned(),
    ))
}

fn write_backup_manifest(
    backup: &Utf8Path,
    target: &Utf8Path,
    fingerprint: &super::model::PathFingerprint,
) -> Result<(), CoreError> {
    let slot = backup.parent().ok_or_else(|| {
        CoreError::InvalidPath("backup path has no private reservation slot".to_owned())
    })?;
    let metadata = fs::symlink_metadata(slot.as_std_path())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CoreError::InvalidPath(
            "backup reservation slot is not a regular directory".to_owned(),
        ));
    }
    let manifest = slot.with_extension("manifest.json");
    let temporary = slot.with_extension("manifest.tmp");
    if path_exists_without_following(&manifest)? || path_exists_without_following(&temporary)? {
        return Err(CoreError::InvalidPath(
            "backup manifest or its temporary path already exists".to_owned(),
        ));
    }
    let content = serde_json::to_vec_pretty(&BackupManifest {
        target_path: target.to_owned(),
        backup_path: backup.to_owned(),
        digest: fingerprint.digest.clone(),
        mode: fingerprint.mode,
    })?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary.as_std_path())?;
    if let Err(error) = file.write_all(&content).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(temporary.as_std_path());
        return Err(CoreError::Io(error));
    }
    if let Err(error) = set_private_file_permissions(&temporary) {
        let _ = fs::remove_file(temporary.as_std_path());
        return Err(error);
    }
    if let Err(error) = fs::hard_link(temporary.as_std_path(), manifest.as_std_path()) {
        let _ = fs::remove_file(temporary.as_std_path());
        return Err(CoreError::Io(error));
    }
    fs::remove_file(temporary.as_std_path())?;
    Ok(())
}

/// Build the backup root one component at a time. `create_dir_all` follows links, so it cannot
/// be used at this trust boundary: every pre-existing component below the caller's target-root
/// anchor must prove itself via `lstat`. The anchor deliberately permits platform-provided path
/// aliases such as macOS's `/var -> /private/var`; all caller-controlled descendants fail closed.
fn ensure_safe_backup_root(backup_root: &Utf8Path, target: &Utf8Path) -> Result<(), CoreError> {
    if !backup_root.is_absolute() {
        return Err(CoreError::InvalidPath(
            "backup root must be an absolute path".to_owned(),
        ));
    }
    let anchor = target
        .ancestors()
        .find(|candidate| backup_root.starts_with(candidate))
        .ok_or_else(|| {
            CoreError::InvalidPath(
                "backup root has no common absolute ancestor with its target".to_owned(),
            )
        })?;
    let mut ancestors = backup_root
        .ancestors()
        .take_while(|path| path.starts_with(anchor))
        .collect::<Vec<_>>();
    ancestors.reverse();
    for path in ancestors {
        if path.as_str().is_empty() {
            continue;
        }
        match fs::symlink_metadata(path.as_std_path()) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(CoreError::InvalidPath(
                    "backup root contains a symlink and is not safe to traverse".to_owned(),
                ));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(CoreError::InvalidPath(
                    "backup root component is not a directory".to_owned(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match fs::create_dir(path.as_std_path()) {
                    Ok(()) => {}
                    Err(create_error)
                        if create_error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(create_error) => return Err(CoreError::Io(create_error)),
                }
                let metadata = fs::symlink_metadata(path.as_std_path())?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(CoreError::InvalidPath(
                        "created backup root component is not a regular directory".to_owned(),
                    ));
                }
            }
            Err(error) => return Err(CoreError::Io(error)),
        }
    }
    Ok(())
}

fn path_exists_without_following(path: &Utf8Path) -> Result<bool, CoreError> {
    match fs::symlink_metadata(path.as_std_path()) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(CoreError::Io(error)),
    }
}

fn backup_nonce() -> Result<u64, CoreError> {
    #[cfg(unix)]
    {
        let mut bytes = [0_u8; 8];
        fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        Ok(u64::from_ne_bytes(bytes))
    }
    #[cfg(not(unix))]
    {
        Ok(TEMP_LINK_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ^ Utc::now().timestamp_nanos_opt().unwrap_or_default() as u64)
    }
}

fn direct_member_id(action: &ProjectionAction) -> Result<super::model::ProjectionId, CoreError> {
    action
        .members
        .first()
        .map(|member| member.id.clone())
        .ok_or_else(|| CoreError::InvalidPath("direct-link action has no source member".to_owned()))
}

fn record_mutation_for_action(action: &ProjectionAction) -> Result<LedgerMutation, CoreError> {
    let mode = if action.state.as_deref() == Some("copied") {
        ProjectionMode::CopyFallback
    } else {
        ProjectionMode::DirectLink
    };
    record_mutation(action, mode)
}

fn direct_record_mutation(action: &ProjectionAction) -> Result<LedgerMutation, CoreError> {
    record_mutation(action, ProjectionMode::DirectLink)
}

fn copy_record_mutation(action: &ProjectionAction) -> Result<LedgerMutation, CoreError> {
    record_mutation(action, ProjectionMode::CopyFallback)
}

fn record_mutation(
    action: &ProjectionAction,
    mode: ProjectionMode,
) -> Result<LedgerMutation, CoreError> {
    let target = action
        .target
        .as_ref()
        .ok_or_else(|| CoreError::InvalidPath("direct-link action has no target".to_owned()))?;
    let member = action.members.first().ok_or_else(|| {
        CoreError::InvalidPath("direct-link action has no source member".to_owned())
    })?;
    let fingerprint = path_fingerprint(&target.path)?;
    let target_fingerprint = fingerprint.digest.ok_or_else(|| {
        CoreError::InvalidPath("direct-link target is missing after apply".to_owned())
    })?;
    Ok(LedgerMutation::Upsert(ProjectionRecord {
        id: member.id.clone(),
        mode,
        source_path: member.source.absolute_path.clone(),
        target_path: target.path.clone(),
        entry_key: None,
        source_fingerprint: member.source.fingerprint.clone(),
        entry_fingerprint: None,
        target_fingerprint,
        applied_at: Utc::now(),
    }))
}

fn rollback(undo: &[(usize, FileUndo)], context: &ExecutorContext<'_>) -> Vec<(usize, bool)> {
    let mut outcomes = Vec::with_capacity(undo.len());
    for (index, operation) in undo.iter().rev() {
        let restored = match operation {
            FileUndo::RemoveCreated {
                target,
                source,
                created_parents,
            } => {
                let removed = path_fingerprint(target)
                    .map(|fingerprint| {
                        fingerprint.entry_type == super::model::FingerprintType::Symlink
                            && fingerprint.link_target.as_ref() == Some(source)
                            && fs::remove_file(target.as_std_path()).is_ok()
                    })
                    .unwrap_or(false);
                if removed {
                    for parent in created_parents.iter().rev() {
                        let is_empty = fs::read_dir(parent.as_std_path())
                            .ok()
                            .and_then(|mut entries| entries.next())
                            .is_none();
                        if is_empty {
                            let _ = fs::remove_dir(parent.as_std_path());
                        }
                    }
                }
                removed
            }
            FileUndo::RestoreRemoved {
                target,
                raw_link_target,
            } => {
                let target_is_missing = path_fingerprint(target)
                    .map(|fingerprint| {
                        fingerprint.entry_type == super::model::FingerprintType::Missing
                    })
                    .unwrap_or(false);
                if target_is_missing {
                    if let Ok(parent) = ensure_safe_target_parent(target, &context.deploy_base) {
                        create_sibling_symlink(raw_link_target, target, &parent.path).is_ok()
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            FileUndo::RestoreBackup {
                target,
                source,
                backup,
            } => {
                if let Ok(fingerprint) = path_fingerprint(target) {
                    if fingerprint.entry_type == super::model::FingerprintType::Symlink
                        && fingerprint.link_target.as_ref() == Some(source)
                    {
                        if fs::remove_file(target.as_std_path()).is_err() {
                            false
                        } else if path_fingerprint(target)
                            .map(|fingerprint| {
                                fingerprint.entry_type == super::model::FingerprintType::Missing
                            })
                            .unwrap_or(false)
                        {
                            fs::rename(backup.as_std_path(), target.as_std_path()).is_ok()
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            FileUndo::RemoveCopied {
                target,
                digest,
                created_parents,
            } => {
                let removed = remove_copied_path(target, Some(digest)).is_ok();
                if removed {
                    for parent in created_parents.iter().rev() {
                        let is_empty = fs::read_dir(parent.as_std_path())
                            .ok()
                            .and_then(|mut entries| entries.next())
                            .is_none();
                        if is_empty {
                            let _ = fs::remove_dir(parent.as_std_path());
                        }
                    }
                }
                removed
            }
            FileUndo::RestoreCopiedBackup {
                target,
                digest,
                backup,
            } => {
                if remove_copied_path(target, Some(digest)).is_ok()
                    && path_fingerprint(target)
                        .map(|fingerprint| {
                            fingerprint.entry_type == super::model::FingerprintType::Missing
                        })
                        .unwrap_or(false)
                {
                    fs::rename(backup.as_std_path(), target.as_std_path()).is_ok()
                } else {
                    false
                }
            }
            FileUndo::RestoreGenerated {
                target,
                rendered_digest,
                backup,
                created_parents,
            } => {
                let removed = path_fingerprint(target)
                    .map(|fingerprint| {
                        fingerprint.entry_type == super::model::FingerprintType::File
                            && fingerprint.digest.as_deref() == Some(rendered_digest)
                            && fs::remove_file(target.as_std_path()).is_ok()
                    })
                    .unwrap_or(false);
                if !removed {
                    false
                } else if let Some(backup) = backup {
                    fs::rename(backup.as_std_path(), target.as_std_path()).is_ok()
                } else {
                    for parent in created_parents.iter().rev() {
                        let is_empty = fs::read_dir(parent.as_std_path())
                            .ok()
                            .and_then(|mut entries| entries.next())
                            .is_none();
                        if is_empty {
                            let _ = fs::remove_dir(parent.as_std_path());
                        }
                    }
                    true
                }
            }
        };
        outcomes.push((*index, restored));
    }
    outcomes
}

fn ensure_target_is_allowed(target: &Utf8Path, deploy_base: &Utf8Path) -> Result<(), CoreError> {
    if !target.starts_with(deploy_base) {
        return Err(CoreError::InvalidPath(
            "projection target is outside the allowlisted deploy root".to_owned(),
        ));
    }
    let relative = target.strip_prefix(deploy_base).map_err(|_| {
        CoreError::InvalidPath(
            "projection target is outside the allowlisted deploy root".to_owned(),
        )
    })?;
    if relative.components().any(|component| {
        matches!(
            component,
            Utf8Component::ParentDir | Utf8Component::Prefix(_)
        )
    }) {
        return Err(CoreError::InvalidPath(
            "projection target escapes the allowlisted deploy root".to_owned(),
        ));
    }
    Ok(())
}

fn ensure_safe_target_path(
    target: &Utf8Path,
    deploy_base: &Utf8Path,
) -> Result<Utf8PathBuf, CoreError> {
    ensure_target_is_allowed(target, deploy_base)?;
    let base = fs::canonicalize(deploy_base.as_std_path())?;
    let base = Utf8PathBuf::from_path_buf(base)
        .map_err(|path| CoreError::InvalidPath(path.to_string_lossy().into_owned()))?;
    let parent = target.parent().ok_or_else(|| {
        CoreError::InvalidPath("projection target has no parent directory".to_owned())
    })?;
    let relative = parent.strip_prefix(deploy_base).map_err(|_| {
        CoreError::InvalidPath(
            "projection parent is outside the allowlisted deploy root".to_owned(),
        )
    })?;
    let mut cursor = base;
    for component in relative.components() {
        match component {
            Utf8Component::Normal(name) => {
                cursor.push(name);
                let metadata = fs::symlink_metadata(cursor.as_std_path())?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(CoreError::InvalidPath(
                        "projection parent is not a safe existing directory".to_owned(),
                    ));
                }
            }
            Utf8Component::CurDir => {}
            Utf8Component::ParentDir | Utf8Component::Prefix(_) | Utf8Component::RootDir => {
                return Err(CoreError::InvalidPath(
                    "projection parent has an unsafe path component".to_owned(),
                ));
            }
        }
    }
    let filename = target
        .file_name()
        .ok_or_else(|| CoreError::InvalidPath("projection target has no file name".to_owned()))?;
    Ok(cursor.join(filename))
}

fn ensure_safe_durable_backup(
    action_backup_root: &Utf8Path,
    backup: &Utf8Path,
) -> Result<(), CoreError> {
    let root_metadata = fs::symlink_metadata(action_backup_root.as_std_path())?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(CoreError::InvalidPath(
            "durable adoption backup root is unsafe".to_owned(),
        ));
    }
    let relative = backup.strip_prefix(action_backup_root).map_err(|_| {
        CoreError::InvalidPath("durable adoption backup escapes its transaction root".to_owned())
    })?;
    if relative.as_str().is_empty() {
        return Err(CoreError::InvalidPath(
            "durable adoption backup cannot be its transaction root".to_owned(),
        ));
    }
    let root = fs::canonicalize(action_backup_root.as_std_path())?;
    let mut cursor = Utf8PathBuf::from_path_buf(root)
        .map_err(|path| CoreError::InvalidPath(path.to_string_lossy().into_owned()))?;
    let component_count = relative.components().count();
    for (index, component) in relative.components().enumerate() {
        match component {
            Utf8Component::Normal(name) => {
                cursor.push(name);
                let metadata = fs::symlink_metadata(cursor.as_std_path())?;
                if metadata.file_type().is_symlink() {
                    return Err(CoreError::InvalidPath(
                        "durable adoption backup contains a symlink".to_owned(),
                    ));
                }
                let is_final = index + 1 == component_count;
                if (!is_final && !metadata.is_dir())
                    || (is_final && !metadata.is_dir() && !metadata.is_file())
                {
                    return Err(CoreError::InvalidPath(
                        "durable adoption backup has an unsafe entry type".to_owned(),
                    ));
                }
            }
            Utf8Component::CurDir => {}
            Utf8Component::ParentDir | Utf8Component::Prefix(_) | Utf8Component::RootDir => {
                return Err(CoreError::InvalidPath(
                    "durable adoption backup has an unsafe path component".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn find_durable_backup(
    action_backup_root: &Utf8Path,
    action: &DurableAdoptionAction,
) -> Result<Option<Utf8PathBuf>, CoreError> {
    let root_metadata = fs::symlink_metadata(action_backup_root.as_std_path())?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(CoreError::InvalidPath(
            "durable adoption backup root is unsafe".to_owned(),
        ));
    }
    let mut matched = None;
    for entry in fs::read_dir(action_backup_root.as_std_path())? {
        let entry = entry?;
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|path| CoreError::InvalidPath(path.to_string_lossy().into_owned()))?;
        if path.extension() != Some("json") || !path.as_str().ends_with(".manifest.json") {
            continue;
        }
        let metadata = fs::symlink_metadata(path.as_std_path())?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(CoreError::InvalidPath(
                "durable adoption backup manifest is unsafe".to_owned(),
            ));
        }
        let manifest: BackupManifest = serde_json::from_slice(&fs::read(path.as_std_path())?)?;
        if manifest.target_path != action.target_path {
            continue;
        }
        if manifest.digest != action.before.digest || manifest.mode != action.before.mode {
            return Err(CoreError::InvalidPath(
                "durable adoption backup manifest does not match its action".to_owned(),
            ));
        }
        let candidate = manifest.backup_path;
        let relative = candidate.strip_prefix(action_backup_root).map_err(|_| {
            CoreError::InvalidPath(
                "durable adoption backup manifest escapes its transaction root".to_owned(),
            )
        })?;
        if relative.as_str().is_empty()
            || relative.components().any(|component| {
                matches!(
                    component,
                    Utf8Component::ParentDir | Utf8Component::Prefix(_)
                )
            })
        {
            return Err(CoreError::InvalidPath(
                "durable adoption backup manifest has an unsafe path".to_owned(),
            ));
        }
        if matched.replace(candidate).is_some() {
            return Err(CoreError::InvalidPath(
                "durable adoption found duplicate backups for one target".to_owned(),
            ));
        }
    }
    Ok(matched)
}

fn durable_record_matches(
    record: &ProjectionRecord,
    action: &DurableAdoptionAction,
    target: &super::model::PathFingerprint,
) -> bool {
    record.id == action.projection_id
        && record.mode == ProjectionMode::DirectLink
        && record.source_path == action.source_path
        && record.target_path == action.target_path
        && record.source_fingerprint == action.source_fingerprint
        && record.entry_key.is_none()
        && record.entry_fingerprint.is_none()
        && record.target_fingerprint == target.digest.as_deref().unwrap_or_default()
}

fn ensure_safe_target_parent(
    target: &Utf8Path,
    deploy_base: &Utf8Path,
) -> Result<SafeParent, CoreError> {
    let base = fs::canonicalize(deploy_base.as_std_path())?;
    let base = Utf8PathBuf::from_path_buf(base)
        .map_err(|path| CoreError::InvalidPath(path.to_string_lossy().into_owned()))?;
    let parent = target.parent().ok_or_else(|| {
        CoreError::InvalidPath("projection target has no parent directory".to_owned())
    })?;
    let relative = parent.strip_prefix(deploy_base).map_err(|_| {
        CoreError::InvalidPath(
            "projection parent is outside the allowlisted deploy root".to_owned(),
        )
    })?;
    let mut cursor = base;
    let mut created_parents = Vec::new();
    for component in relative.components() {
        match component {
            Utf8Component::Normal(name) => {
                cursor.push(name);
                match fs::symlink_metadata(cursor.as_std_path()) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        return Err(CoreError::InvalidPath(
                            "projection parent contains a symlink and is not safe to traverse"
                                .to_owned(),
                        ));
                    }
                    Ok(metadata) if !metadata.is_dir() => {
                        return Err(CoreError::InvalidPath(
                            "projection parent component is not a directory".to_owned(),
                        ));
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        fs::create_dir(&cursor)?;
                        created_parents.push(cursor.clone());
                    }
                    Err(error) => return Err(CoreError::Io(error)),
                }
            }
            Utf8Component::CurDir => {}
            Utf8Component::ParentDir | Utf8Component::Prefix(_) | Utf8Component::RootDir => {
                return Err(CoreError::InvalidPath(
                    "projection parent has an unsafe path component".to_owned(),
                ));
            }
        }
    }
    Ok(SafeParent {
        path: cursor,
        created_parents,
    })
}

fn create_sibling_symlink(
    source: &Utf8Path,
    target: &Utf8Path,
    parent: &Utf8Path,
) -> Result<(), CoreError> {
    let filename = target
        .file_name()
        .ok_or_else(|| CoreError::InvalidPath("projection target has no file name".to_owned()))?;
    let sequence = TEMP_LINK_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(".{filename}.ai-config-{sequence}.tmp"));
    #[cfg(unix)]
    std::os::unix::fs::symlink(source.as_std_path(), temporary.as_std_path())?;
    #[cfg(windows)]
    {
        let result = if source.is_dir() {
            std::os::windows::fs::symlink_dir(source.as_std_path(), temporary.as_std_path())
        } else {
            std::os::windows::fs::symlink_file(source.as_std_path(), temporary.as_std_path())
        };
        if let Err(error) = result {
            // A policy-authorized CopyFallback is a separate action. A failed direct link must
            // never silently materialize a hard copy, otherwise later retract could delete user
            // data that was not ledger-proven.
            return Err(CoreError::LinkFailed {
                src: source.to_string(),
                dest: target.to_string(),
                reason: format!("Windows direct-link creation failed: {error}"),
                hint: "启用平台显式 CopyFallback 策略后重新生成计划；不会自动拷贝".to_owned(),
            });
        }
    }
    fs::rename(temporary.as_std_path(), target.as_std_path())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_failure_is_not_overwritten_by_a_later_success_for_the_same_action() {
        let mut report = ApplyReport {
            transaction_id: None,
            changed: 1,
            unchanged: 0,
            skipped: 0,
            conflict: 0,
            failed: 0,
            rolled_back: 0,
            rollback_failed: 0,
            not_applied: 0,
            actions: vec![ApplyActionReport {
                action_id: "generated-container".to_owned(),
                status: ApplyActionStatus::Applied,
            }],
            mcp_skipped_members: Vec::new(),
            failure: None,
        };

        report.mark_rollback_result(0, false);
        report.mark_rollback_result(0, true);

        assert_eq!(report.actions[0].status, ApplyActionStatus::RollbackFailed);
        assert_eq!(report.rollback_failed, 1);
        assert_eq!(report.rolled_back, 0);
    }
}
