//! Source-first migration CLI wiring.

use std::collections::BTreeSet;
use std::fs;
use std::process::ExitCode;

use agents_manager_core::error::exit_code;
use agents_manager_core::error::CoreError;
use agents_manager_core::model::{AssetKind, PlatformId};
use agents_manager_core::paths;
use agents_manager_core::platform;
use agents_manager_core::projection::migration::{
    inventory, CanonicalLayerRoot, InventoryRequest, InventoryScope,
};
use agents_manager_core::projection::migration_action::{
    apply_import_plan, build_import_plan, rollback_import_transaction, ImportApplyOptions,
    ImportApplyReport, ImportPlan, ImportRequest, RollbackStatus,
};
use agents_manager_core::projection::model::SourceLayer;
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::output::{emit_error_envelope, emit_json, emit_line, OutputMode};
use crate::projection;

#[derive(Debug, Deserialize)]
struct ReviewedMigrationPlan {
    plan_digest: String,
}

#[derive(Debug, Serialize)]
struct ImportExecutionReport {
    #[serde(flatten)]
    import: ImportApplyReport,
    projection_plan: projection::PublicPlan,
}

/// Plan or apply one explicitly requested platform-to-source import. The core owns all
/// preconditions, secret checks, durable manifests, and writes; this layer only resolves the
/// published CLI contract to one approved source path.
#[allow(clippy::too_many_arguments)]
pub fn run_import(
    mode: OutputMode,
    default_root: &Utf8Path,
    kind: &str,
    name: &str,
    from: &str,
    to: &str,
    replace: bool,
    plan_path: Option<&str>,
    apply: bool,
) -> ExitCode {
    let request = match import_request(default_root, kind, name, from, to, replace) {
        Ok(request) => request,
        Err(error) => return migration_error(mode, error),
    };
    let current = match build_import_plan(&[request]) {
        Ok(plan) => plan,
        Err(error) => return migration_error(mode, error),
    };

    if !apply {
        if plan_path.is_some() {
            return migration_error(
                mode,
                CoreError::InvalidPath("--plan is only valid together with --apply".to_owned()),
            );
        }
        if mode.is_json() {
            emit_json(mode, &current);
        } else if mode.is_human() {
            emit_line(
                mode,
                format!(
                    "import plan: {} action(s), digest {}",
                    current.actions.len(),
                    current.plan_digest
                ),
            );
        }
        return ExitCode::SUCCESS;
    }

    let reviewed = match plan_path {
        Some(path) => match read_import_plan(path) {
            Ok(plan) => plan,
            Err(error) => return migration_error(mode, error),
        },
        None => {
            return migration_error(
                mode,
                CoreError::InvalidPath(
                    "import --apply requires a reviewed --plan <file>".to_owned(),
                ),
            )
        }
    };
    if reviewed.schema_version != current.schema_version
        || reviewed.plan_digest != current.plan_digest
        || reviewed.actions != current.actions
    {
        return migration_error(
            mode,
            CoreError::InvalidPath(
                "reviewed import plan no longer exactly matches the current action".to_owned(),
            ),
        );
    }
    let Some(action) = current.actions.first() else {
        return migration_error(
            mode,
            CoreError::InvalidPath("import plan has no applicable action".to_owned()),
        );
    };
    let transaction_root = transaction_root(default_root);
    let options = ImportApplyOptions::new(
        reviewed.plan_digest,
        std::iter::once(action.action_id.clone()),
    );
    match apply_import_plan(&current, &options, &transaction_root) {
        Ok(report) => {
            let projection_plan = match projection::build_migration_plan(default_root) {
                Ok(plan) => plan,
                Err(error) => {
                    let rollback_result = report.transaction_id.as_deref().map(|transaction_id| {
                        rollback_import_transaction(
                            &transaction_root,
                            transaction_id,
                            &approved_destination_roots(default_root),
                        )
                    });
                    if !matches!(
                        rollback_result,
                        Some(Ok(ref rollback)) if rollback.status == RollbackStatus::RolledBack
                    ) {
                        return migration_error(
                            mode,
                            CoreError::ProjectionLedger(
                                "import succeeded but the follow-up projection plan failed and rollback could not prove restoration"
                                    .to_owned(),
                            ),
                        );
                    }
                    return migration_error(mode, error);
                }
            };
            let execution = ImportExecutionReport {
                import: report,
                projection_plan,
            };
            if mode.is_json() {
                emit_json(mode, &execution);
            } else if mode.is_human() {
                emit_line(
                    mode,
                    format!(
                        "import applied: {} action(s); next projection plan {} has {} action(s)",
                        execution.import.applied,
                        execution.projection_plan.plan_digest,
                        execution.projection_plan.actions.len()
                    ),
                );
            }
            ExitCode::SUCCESS
        }
        Err(error) => migration_error(mode, error),
    }
}

/// Rollback routes the distinct import and adoption transaction namespaces to their core-owned
/// recovery logic. Both paths preflight the complete transaction before changing any target.
pub fn run_rollback(mode: OutputMode, default_root: &Utf8Path, transaction_id: &str) -> ExitCode {
    if transaction_id.starts_with("adopt-") {
        return match projection::rollback_reviewed_migration(default_root, transaction_id) {
            Ok(report) => {
                if mode.is_json() {
                    emit_json(mode, &report);
                } else if mode.is_human() {
                    emit_line(
                        mode,
                        format!(
                            "migration rollback {}: {} restored",
                            report.transaction_id, report.restored
                        ),
                    );
                }
                ExitCode::SUCCESS
            }
            Err(error) => migration_error(mode, error),
        };
    }
    match rollback_import_transaction(
        &transaction_root(default_root),
        transaction_id,
        &approved_destination_roots(default_root),
    ) {
        Ok(report) => {
            if mode.is_json() {
                emit_json(mode, &report);
            } else if mode.is_human() {
                emit_line(
                    mode,
                    format!(
                        "migration rollback {}: {:?} ({} restored)",
                        report.transaction_id, report.status, report.restored
                    ),
                );
            }
            if report.status == RollbackStatus::Drifted {
                ExitCode::from(exit_code::PARTIAL_FAILURE)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => migration_error(mode, error),
    }
}

fn approved_destination_roots(default_root: &Utf8Path) -> Vec<Utf8PathBuf> {
    let roots = paths::resolve_sync_roots(default_root);
    let mut approved = vec![roots.global_default, roots.asset_root];
    approved.sort();
    approved.dedup();
    approved
}

pub fn run_inventory(mode: OutputMode, default_root: &Utf8Path, workspace: bool) -> ExitCode {
    let request = match inventory_request(default_root, workspace) {
        Ok(request) => request,
        Err(error) => {
            let code = error.exit_code();
            emit_error_envelope(mode, code, &error.to_string(), error.hint());
            return ExitCode::from(code);
        }
    };

    match inventory(&request) {
        Ok(report) => {
            if mode.is_json() {
                emit_json(mode, &report);
            } else if mode.is_human() {
                let blocking = report.entries.iter().filter(|entry| entry.blocking).count();
                emit_line(
                    mode,
                    format!(
                        "migration inventory: {} entries, {blocking} blocking, digest {}",
                        report.entries.len(),
                        report.plan_digest
                    ),
                );
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            let code = error.exit_code();
            emit_error_envelope(mode, code, &error.to_string(), error.hint());
            ExitCode::from(code)
        }
    }
}

/// Render the same planner-owned source-first actions that a later adoption will validate.
/// This remains read-only: it opens an existing ledger only for planning and never creates one.
pub fn run_plan(mode: OutputMode, default_root: &Utf8Path, workspace: bool) -> ExitCode {
    if workspace {
        return migration_error(
            mode,
            CoreError::NotImplemented(
                "workspace source-first migration is not implemented; refusing to infer a HOME target",
            ),
        );
    }
    match projection::build_migration_plan(default_root) {
        Ok(plan) => {
            if mode.is_json() {
                emit_json(mode, &plan);
            } else if mode.is_human() {
                emit_line(
                    mode,
                    format!(
                        "migration plan: {} actions, digest {}",
                        plan.actions.len(),
                        plan.plan_digest
                    ),
                );
            }
            ExitCode::SUCCESS
        }
        Err(error) => migration_error(mode, error),
    }
}

/// Rebuilds the plan from current source/targets, verifies the reviewed public digest, and then
/// lets the shared projection executor transact only selected `AdoptEquivalent` actions.
pub fn run_source_first(
    mode: OutputMode,
    default_root: &Utf8Path,
    workspace: bool,
    plan_path: &str,
    select: Vec<String>,
    apply: bool,
) -> ExitCode {
    if workspace {
        return migration_error(
            mode,
            CoreError::NotImplemented(
                "workspace source-first migration is not implemented; refusing to infer a HOME target",
            ),
        );
    }
    let reviewed = match read_reviewed_plan(plan_path) {
        Ok(reviewed) => reviewed,
        Err(error) => return migration_error(mode, error),
    };
    let selected = select.into_iter().collect::<BTreeSet<_>>();
    if selected.is_empty() {
        return migration_error(
            mode,
            CoreError::InvalidPath(
                "source-first migration requires at least one --select <action-id>".to_owned(),
            ),
        );
    }

    if !apply {
        match projection::verify_migration_review(default_root, &reviewed.plan_digest, &selected) {
            Ok(plan) => {
                if mode.is_json() {
                    emit_json(mode, &plan);
                } else if mode.is_human() {
                    emit_line(
                        mode,
                        "migration source-first: reviewed plan verified (dry-run)",
                    );
                }
                return ExitCode::SUCCESS;
            }
            Err(error) => return migration_error(mode, error),
        }
    }

    match projection::apply_reviewed_migration(default_root, &reviewed.plan_digest, &selected) {
        Ok(summary) => {
            if mode.is_json() {
                emit_json(mode, &summary);
            } else if mode.is_human() {
                emit_line(
                    mode,
                    format!(
                        "migration source-first: {} selected actions adopted; transaction {}",
                        summary.apply.changed, summary.transaction_id
                    ),
                );
            }
            ExitCode::SUCCESS
        }
        Err(error) => migration_error(mode, error),
    }
}

fn read_reviewed_plan(path: &str) -> Result<ReviewedMigrationPlan, CoreError> {
    let content = fs::read_to_string(path)?;
    let reviewed: ReviewedMigrationPlan = serde_json::from_str(&content)?;
    if reviewed.plan_digest.is_empty() {
        return Err(CoreError::InvalidPath(
            "reviewed migration plan has no plan_digest".to_owned(),
        ));
    }
    Ok(reviewed)
}

fn migration_error(mode: OutputMode, error: CoreError) -> ExitCode {
    let code = error.exit_code();
    emit_error_envelope(mode, code, &error.to_string(), error.hint());
    ExitCode::from(code)
}

fn read_import_plan(path: &str) -> Result<ImportPlan, CoreError> {
    let content = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&content)?)
}

fn import_request(
    default_root: &Utf8Path,
    kind: &str,
    name: &str,
    from: &str,
    to: &str,
    replace: bool,
) -> Result<ImportRequest, CoreError> {
    let kind = parse_import_kind(kind)?;
    let source_platform = platform::parse_deploy_platform_str(from)?;
    let roots = paths::resolve_sync_roots(default_root);
    let (destination_layer, destination_asset_root) = match to {
        "global" => (SourceLayer::Global, roots.global_default.clone()),
        "project" if paths::is_project_deploy_base(&roots.deploy_base) => {
            (SourceLayer::Project, roots.asset_root.clone())
        }
        "project" => {
            return Err(CoreError::InvalidPath(
                "--to project requires --root <project>/.agents-manager".to_owned(),
            ))
        }
        _ => {
            return Err(CoreError::InvalidPath(
                "import --to must be global or project".to_owned(),
            ))
        }
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

fn parse_import_kind(value: &str) -> Result<AssetKind, CoreError> {
    match value {
        "skill" => Ok(AssetKind::Skill),
        "rule" => Ok(AssetKind::Rule),
        "mcp" => Ok(AssetKind::Mcp),
        "agent" => Ok(AssetKind::Agent),
        "command" => Ok(AssetKind::Command),
        "prompt" => Ok(AssetKind::Prompt),
        "hook" => Err(CoreError::NotImplemented(
            "hook import requires compound binding conversion and is fail-closed",
        )),
        _ => Err(CoreError::InvalidPath(
            "import kind must be skill, rule, mcp, agent, command, or prompt".to_owned(),
        )),
    }
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
            "MCP import currently supports only Cursor JSON and Codex TOML containers",
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
        let approved_root = platform_path
            .parent()
            .ok_or_else(|| {
                CoreError::InvalidPath("platform MCP path has no approved parent".to_owned())
            })?
            .to_path_buf();
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

fn transaction_root(default_root: &Utf8Path) -> Utf8PathBuf {
    // The deploy base always exists for a valid global HOME or project root. Keeping the private
    // manifests beside platform state means first-time project import does not require the
    // destination `.agents-manager` directory (or any of its parents) to exist yet.
    paths::resolve_sync_roots(default_root)
        .deploy_base
        .join(".agents-manager-migrations")
}

fn inventory_request(
    default_root: &Utf8Path,
    workspace: bool,
) -> Result<InventoryRequest, CoreError> {
    let roots = paths::resolve_sync_roots(default_root);
    let project_scope = paths::is_project_deploy_base(&roots.deploy_base);
    let (canonical_layers, scope) = if workspace {
        if !project_scope || roots.global_default == roots.asset_root {
            return Err(CoreError::InvalidPath(
                "--workspace migrate inventory requires --root <workspace-root>".to_owned(),
            ));
        }
        (
            vec![
                CanonicalLayerRoot {
                    layer: SourceLayer::Global,
                    asset_root: roots.global_default,
                },
                CanonicalLayerRoot {
                    layer: SourceLayer::Workspace,
                    asset_root: roots.asset_root,
                },
            ],
            InventoryScope::Workspace,
        )
    } else if project_scope {
        let canonical_layers = if roots.global_default == roots.asset_root {
            vec![CanonicalLayerRoot {
                layer: SourceLayer::Project,
                asset_root: roots.asset_root,
            }]
        } else {
            vec![
                CanonicalLayerRoot {
                    layer: SourceLayer::Global,
                    asset_root: roots.global_default,
                },
                CanonicalLayerRoot {
                    layer: SourceLayer::Project,
                    asset_root: roots.asset_root,
                },
            ]
        };
        (canonical_layers, InventoryScope::Project)
    } else {
        (
            vec![CanonicalLayerRoot {
                layer: SourceLayer::Global,
                asset_root: roots.asset_root,
            }],
            InventoryScope::Global,
        )
    };
    Ok(InventoryRequest {
        canonical_layers,
        deploy_base: roots.deploy_base,
        scope,
    })
}
