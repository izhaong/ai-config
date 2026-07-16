//! Transactional source-first projection executor.
//!
//! The initial direct-link slice deliberately accepts only proven `CreateLink` and `Noop`
//! actions. Generated rendering, adoption, copy fallback and cleanup are added by later T006
//! tests; they must never fall back to the legacy materialize path.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use chrono::Utc;
use serde::Serialize;
use serde_json::Value;

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
use super::model::{LedgerMutation, ProjectionMode, ProjectionRecord};
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

/// Transaction-level failure classification. It deliberately carries a stable code rather than
/// the underlying error text, so reports remain safe to serialize without exposing source data.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ApplyFailure {
    pub code: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ApplyReport {
    pub changed: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub conflict: usize,
    pub failed: usize,
    pub rolled_back: usize,
    pub rollback_failed: usize,
    pub not_applied: usize,
    pub actions: Vec<ApplyActionReport>,
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
            changed: 0,
            unchanged: 0,
            skipped: 0,
            conflict: 0,
            failed: 0,
            rolled_back: 0,
            rollback_failed: 0,
            not_applied: 0,
            actions,
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
#[derive(Serialize)]
struct BackupManifest<'a> {
    target_path: &'a Utf8Path,
    backup_path: &'a Utf8Path,
    digest: Option<&'a str>,
    mode: Option<u32>,
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
    if plan
        .actions
        .iter()
        .any(|action| matches!(action.kind, ProjectionActionKind::ReportOnly))
    {
        for (index, action) in plan.actions.iter().enumerate() {
            if action.kind == ProjectionActionKind::ReportOnly {
                report.set_status(index, ApplyActionStatus::Conflict);
            }
        }
        return Err(preflight_failure(
            report,
            "blocking_conflict",
            CoreError::InvalidPath(
                "projection plan contains blocking report-only actions".to_owned(),
            ),
        ));
    }
    let _lock = match ApplyLock::acquire(&context.deploy_base) {
        Ok(lock) => lock,
        Err(error) => return Err(preflight_failure(report, "apply_lock_unavailable", error)),
    };

    let mut mutations = Vec::new();
    let mut undo = Vec::new();
    for (index, action) in plan.actions.iter().enumerate() {
        let outcome = match action.kind {
            ProjectionActionKind::CreateLink => {
                apply_create_link(action, context).and_then(|created| {
                    mutations.push(direct_record_mutation(action)?);
                    undo.push((
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
                    mutations.push(LedgerMutation::Remove(direct_member_id(action)?));
                    undo.push((
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
                    mutations.push(copy_record_mutation(action)?);
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
                    undo.push((index, undo_operation));
                    report.set_status(index, ApplyActionStatus::Applied);
                    Ok(())
                })
            }
            ProjectionActionKind::RemoveManagedCopy => apply_remove_managed_copy(action, context)
                .and_then(|removed| {
                    mutations.push(LedgerMutation::Remove(direct_member_id(action)?));
                    undo.push((
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
            ProjectionActionKind::Noop => record_mutation_for_action(action).map(|mutation| {
                mutations.push(mutation);
                report.set_status(index, ApplyActionStatus::Unchanged);
            }),
            ProjectionActionKind::AdoptEquivalent => {
                if options
                    .selected_action_ids
                    .contains(&plan.action_ids[index])
                {
                    apply_adopt_equivalent(action, context).and_then(|adopted| {
                        mutations.push(direct_record_mutation(action)?);
                        undo.push((
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
            ProjectionActionKind::UpsertGeneratedBatch => {
                apply_mcp_generated_upsert(action, context).map(|applied| {
                    mutations.extend(applied.mutations);
                    undo.push((index, applied.undo));
                    report.set_status(index, ApplyActionStatus::Applied);
                })
            }
            ProjectionActionKind::RemoveGeneratedEntries | ProjectionActionKind::CleanupOrphan => {
                Err(CoreError::NotImplemented(
                    "projection action requires its dedicated transactional executor slice",
                ))
            }
            ProjectionActionKind::ReportOnly => unreachable!("checked before writes"),
        };
        if let Err(error) = outcome {
            return Err(transaction_failure(
                report,
                undo,
                context,
                Some(index),
                "action_apply_failed",
                error,
            ));
        }
    }
    if let Err(error) = context.ledger.apply_batch(&mutations) {
        return Err(transaction_failure(
            report,
            undo,
            context,
            None,
            "ledger_apply_failed",
            error,
        ));
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
    undo: FileUndo,
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
    let intents = action
        .mcp_members
        .iter()
        .map(|member| load_and_hydrate_mcp_member(member, context))
        .collect::<Result<Vec<_>, _>>()?;
    let existing = match actual.entry_type {
        super::model::FingerprintType::Missing => empty_mcp_container(renderer),
        super::model::FingerprintType::File => fs::read_to_string(target.path.as_std_path())?,
        _ => unreachable!("validated above"),
    };
    let rendered = render_mcp_container(renderer, &existing, &intents)?;
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
        undo: FileUndo::RestoreGenerated {
            target: target.path.clone(),
            rendered_digest,
            backup,
            created_parents: parent.created_parents,
        },
    })
}

#[derive(Clone)]
struct HydratedMcpIntent {
    name: String,
    config: Value,
}

fn load_and_hydrate_mcp_member(
    member: &McpProjectionMember,
    context: &ExecutorContext<'_>,
) -> Result<HydratedMcpIntent, CoreError> {
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
    let config = hydrate_mcp_config(definition.server.config, &expected_keys, context)?;
    Ok(HydratedMcpIntent {
        name: member.name.clone(),
        config,
    })
}

fn hydrate_mcp_config(
    mut config: Value,
    secret_keys: &[String],
    context: &ExecutorContext<'_>,
) -> Result<Value, CoreError> {
    if secret_keys.is_empty() {
        return Ok(config);
    }
    let provider = context.mcp_secret_provider.ok_or_else(|| {
        CoreError::InvalidPath("MCP plan needs a caller-provided secret resolver".to_owned())
    })?;
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
            let secret = provider.resolve(key).map_err(|_| {
                CoreError::InvalidPath(format!("MCP secret lookup failed for declared key {key}"))
            })?;
            let secret = secret.ok_or_else(|| {
                CoreError::InvalidPath(format!(
                    "MCP secret resolver has no value for declared key {key}"
                ))
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
) -> Result<String, CoreError> {
    match renderer {
        GeneratedContainerRenderer::McpJson => render_cursor_mcp_json(
            existing,
            &intents
                .iter()
                .map(|intent| JsonServerIntent::new(&intent.name, intent.config.clone()))
                .collect::<Vec<_>>(),
            &[],
        ),
        GeneratedContainerRenderer::McpToml => render_codex_mcp_toml(
            existing,
            &intents
                .iter()
                .map(|intent| TomlServerIntent::new(&intent.name, intent.config.clone()))
                .collect::<Vec<_>>(),
            &[],
        ),
        GeneratedContainerRenderer::McpYaml => render_hermes_mcp_yaml(
            existing,
            &intents
                .iter()
                .map(|intent| YamlServerIntent::new(&intent.name, intent.config.clone()))
                .collect::<Vec<_>>(),
            &[],
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
    fs::create_dir_all(backup_root.as_std_path())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(backup_root.as_std_path(), fs::Permissions::from_mode(0o700))?;
    }
    let filename = target
        .file_name()
        .ok_or_else(|| CoreError::InvalidPath("projection target has no file name".to_owned()))?;
    let sequence = TEMP_LINK_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(backup_root.join(format!("{sequence}-{filename}")))
}

fn write_backup_manifest(
    backup: &Utf8Path,
    target: &Utf8Path,
    fingerprint: &super::model::PathFingerprint,
) -> Result<(), CoreError> {
    let manifest = backup.with_extension("manifest.json");
    let temporary = backup.with_extension("manifest.tmp");
    let content = serde_json::to_vec_pretty(&BackupManifest {
        target_path: target,
        backup_path: backup,
        digest: fingerprint.digest.as_deref(),
        mode: fingerprint.mode,
    })?;
    fs::write(temporary.as_std_path(), content)?;
    fs::rename(temporary.as_std_path(), manifest.as_std_path())?;
    Ok(())
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
