//! Tauri command 与 `ai-config-core::asset_ops` 之间的薄桥。

use std::collections::{BTreeMap, BTreeSet};

use ai_config_core::asset_ops::{self, AssetFileDetail, ScopeRoots};
use ai_config_core::error::CoreError;
use ai_config_core::model::{AssetKind, PlatformId};
use ai_config_core::paths::SyncRoots;
use ai_config_core::projection::executor::{
    apply_projection_plans_transactionally, ApplyOptions, ExecutorContext, McpSecretProvider,
};
use ai_config_core::projection::ledger::{MemoryProjectionLedger, ProjectionLedger};
use ai_config_core::projection::lifecycle::build_projection_review;
use ai_config_core::projection::migration_action::{
    apply_import_plan, build_import_plan, import_request_for_sync_roots, ImportApplyOptions,
    ImportApplyReport, ImportPlan,
};
use ai_config_core::projection::planner::{
    McpSecretAvailability, ProjectionActionKind, ProjectionOperation, ProjectionPlan,
};
use ai_config_core::secrets;
use ai_config_core::skills_add::{self, SkillAddBatchItem, SkillAddBatchOutcome, SkillsAddParams};
use ai_config_store::Store;
use camino::Utf8PathBuf;
use serde::Serialize;

pub async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, CoreError> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("spawn_blocking join: {e}"))?
        .map_err(|e| e.to_string())
}

pub fn scope_roots<'a>(
    default_root: &'a Utf8PathBuf,
    asset_root: &'a Utf8PathBuf,
    deploy_base: &'a Utf8PathBuf,
) -> ScopeRoots<'a> {
    ScopeRoots {
        default_root,
        asset_root,
        deploy_base,
    }
}

#[derive(Debug, Serialize)]
pub struct ProjectionReview {
    pub schema_version: u16,
    pub plan_digest: String,
    pub actions: Vec<serde_json::Value>,
    pub blocking_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ledger_status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProjectionApply {
    pub changed: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub conflict: usize,
    pub failed: usize,
    pub rolled_back: usize,
    pub rollback_failed: usize,
    pub not_applied: usize,
}

struct GuiMcpSecrets {
    values: BTreeMap<String, String>,
}

impl GuiMcpSecrets {
    fn load() -> Result<Self, CoreError> {
        Ok(Self {
            values: secrets::load_from(&secrets::default_path())?
                .into_iter()
                .collect(),
        })
    }
}

impl McpSecretAvailability for GuiMcpSecrets {
    fn missing_secret_keys(&self, keys: &[String]) -> Result<Vec<String>, CoreError> {
        Ok(keys
            .iter()
            .filter(|key| !self.values.contains_key(key.as_str()))
            .cloned()
            .collect())
    }
}

impl McpSecretProvider for GuiMcpSecrets {
    fn resolve(&self, key: &str) -> Result<Option<String>, CoreError> {
        Ok(self.values.get(key).cloned())
    }
}

fn roots(
    default_root: Utf8PathBuf,
    asset_root: Utf8PathBuf,
    deploy_base: Utf8PathBuf,
) -> SyncRoots {
    SyncRoots {
        repo_root: deploy_base.clone(),
        asset_root,
        global_default: default_root,
        deploy_base,
    }
}

fn ledger_path(roots: &SyncRoots) -> Utf8PathBuf {
    roots
        .deploy_base
        .join(".ai-config/projection-ledger.sqlite")
}

fn review_with_ledger(
    roots: &SyncRoots,
    operation: ProjectionOperation,
    ledger: &dyn ProjectionLedger,
    secrets: &GuiMcpSecrets,
    ledger_status: Option<String>,
) -> Result<ProjectionReview, CoreError> {
    let review = build_projection_review(roots, operation, ledger, secrets)?;
    Ok(ProjectionReview {
        schema_version: review.schema_version,
        plan_digest: review.plan_digest,
        actions: review.actions,
        blocking_reason: review.blocking_reason,
        ledger_status,
    })
}

fn reviewed_projection(
    roots: &SyncRoots,
    operation: ProjectionOperation,
    secrets: &GuiMcpSecrets,
) -> Result<ProjectionReview, CoreError> {
    let ledger_path = ledger_path(roots);
    if std::fs::symlink_metadata(ledger_path.as_std_path()).is_ok() {
        match Store::open_read_only_at(ledger_path.as_std_path()) {
            Ok(store) => review_with_ledger(roots, operation, &store.projections(), secrets, None),
            Err(_) => {
                let ledger = MemoryProjectionLedger::default();
                review_with_ledger(
                    roots,
                    operation,
                    &ledger,
                    secrets,
                    Some("ledger_unavailable".to_owned()),
                )
            }
        }
    } else {
        let ledger = MemoryProjectionLedger::default();
        review_with_ledger(roots, operation, &ledger, secrets, None)
    }
}

pub async fn projection_plan(
    default_root: Utf8PathBuf,
    asset_root: Utf8PathBuf,
    deploy_base: Utf8PathBuf,
    retract: bool,
) -> Result<ProjectionReview, String> {
    blocking(move || {
        let roots = roots(default_root, asset_root, deploy_base);
        let secrets = GuiMcpSecrets::load()?;
        reviewed_projection(
            &roots,
            if retract {
                ProjectionOperation::Uninstall
            } else {
                ProjectionOperation::Sync
            },
            &secrets,
        )
    })
    .await
}

pub async fn projection_apply(
    default_root: Utf8PathBuf,
    asset_root: Utf8PathBuf,
    deploy_base: Utf8PathBuf,
    retract: bool,
    expected_plan_digest: String,
) -> Result<ProjectionApply, String> {
    blocking(move || {
        let roots = roots(default_root, asset_root, deploy_base);
        let operation = if retract {
            ProjectionOperation::Uninstall
        } else {
            ProjectionOperation::Sync
        };
        let secrets = GuiMcpSecrets::load()?;
        let reviewed = reviewed_projection(&roots, operation, &secrets)?;
        if reviewed.ledger_status.as_deref() == Some("ledger_unavailable") {
            return Err(CoreError::ProjectionLedger(
                "projection ownership ledger is unavailable".to_owned(),
            ));
        }
        if reviewed.plan_digest != expected_plan_digest {
            return Err(CoreError::InvalidPath(
                "reviewed projection plan digest is stale or does not match current source-first plan"
                    .to_owned(),
            ));
        }
        let has_adoption = reviewed
            .actions
            .iter()
            .any(|action| action["kind"] == "adopt_equivalent");
        if let Some(reason) = reviewed.blocking_reason.filter(|_| !has_adoption) {
            return Err(CoreError::InvalidPath(format!(
                "source-first projection cannot apply while the reviewed plan contains a blocking action: {reason}"
            )));
        }

        let store = Store::open_at(ledger_path(&roots).as_std_path())
            .map_err(|error| CoreError::ProjectionLedger(error.to_string()))?;
        let ledger = store.projections();
        let rebuilt = build_projection_review(&roots, operation, &ledger, &secrets)?;
        let rebuilt_has_adoption = rebuilt.plans.iter().any(|plan| {
            plan.actions
                .iter()
                .any(|action| action.kind == ProjectionActionKind::AdoptEquivalent)
        });
        if rebuilt.plan_digest != expected_plan_digest
            || (rebuilt.blocking_reason.is_some() && !rebuilt_has_adoption)
        {
            return Err(CoreError::InvalidPath(
                "reviewed projection plan changed before apply".to_owned(),
            ));
        }
        let context = ExecutorContext::new(
            &ledger,
            roots.deploy_base.clone(),
            roots.deploy_base.join(".ai-config/projection-backups"),
        )
        .with_mcp_secret_provider(&secrets);
        let executable = projection_apply_slices(&rebuilt.plans);
        let reports = apply_projection_plans_transactionally(
            executable.plans.iter().map(|plan| {
                let selected = plan
                    .actions
                    .iter()
                    .enumerate()
                    .filter(|(_, action)| action.kind == ProjectionActionKind::AdoptEquivalent)
                    .map(|(index, _)| plan.action_ids[index].clone())
                    .collect::<Vec<_>>();
                let options = if selected.is_empty() {
                    ApplyOptions::for_plan(plan)
                } else {
                    // The GUI confirmation modal reviews the exact combined digest and every
                    // visible action. Confirming it explicitly selects equivalent adoption
                    // slices; ordinary actions on other targets remain in the transaction.
                    ApplyOptions::with_selected_action_ids(plan, selected)
                };
                (plan, options)
            }),
            &context,
        )
        .map_err(|error| error.error)?;
        let mut result = reports.into_iter().fold(
            ProjectionApply {
                changed: 0,
                unchanged: 0,
                skipped: 0,
                conflict: 0,
                failed: 0,
                rolled_back: 0,
                rollback_failed: 0,
                not_applied: 0,
            },
            |mut total, report| {
                total.changed += report.changed;
                total.unchanged += report.unchanged;
                total.skipped += report.skipped;
                total.conflict += report.conflict;
                total.failed += report.failed;
                total.rolled_back += report.rolled_back;
                total.rollback_failed += report.rollback_failed;
                total.not_applied += report.not_applied;
                total
            },
        );
        result.skipped += executable.deferred;
        result.conflict += executable.conflicts;
        Ok(result)
    })
    .await
}

struct ProjectionApplySlices {
    plans: Vec<ProjectionPlan>,
    deferred: usize,
    conflicts: usize,
}

fn projection_apply_slices(plans: &[ProjectionPlan]) -> ProjectionApplySlices {
    let has_adoption = plans.iter().any(|plan| {
        plan.actions
            .iter()
            .any(|action| action.kind == ProjectionActionKind::AdoptEquivalent)
    });
    if !has_adoption {
        return ProjectionApplySlices {
            plans: plans
                .iter()
                .filter(|plan| !plan.actions.is_empty())
                .cloned()
                .collect(),
            deferred: 0,
            conflicts: 0,
        };
    }
    let adoption_targets = plans
        .iter()
        .flat_map(|plan| plan.actions.iter())
        .filter(|action| action.kind == ProjectionActionKind::AdoptEquivalent)
        .filter_map(|action| action.target.as_ref().map(|target| target.path.clone()))
        .collect::<BTreeSet<_>>();
    let mut slices = Vec::new();
    let mut deferred = 0;
    let mut conflicts = 0;
    for plan in plans {
        let adoption_indexes = plan
            .actions
            .iter()
            .enumerate()
            .filter_map(|(index, action)| {
                (action.kind == ProjectionActionKind::AdoptEquivalent).then_some(index)
            })
            .collect::<Vec<_>>();
        let ordinary_indexes = plan
            .actions
            .iter()
            .enumerate()
            .filter_map(|(index, action)| {
                if action.kind == ProjectionActionKind::ReportOnly {
                    conflicts += 1;
                    return None;
                }
                if action.kind != ProjectionActionKind::AdoptEquivalent
                    && action
                        .target
                        .as_ref()
                        .is_some_and(|target| adoption_targets.contains(&target.path))
                {
                    deferred += 1;
                    return None;
                }
                (action.kind != ProjectionActionKind::AdoptEquivalent).then_some(index)
            })
            .collect::<Vec<_>>();
        if !ordinary_indexes.is_empty() {
            slices.push(projection_plan_slice(plan, &ordinary_indexes));
        }
        if !adoption_indexes.is_empty() {
            slices.push(projection_plan_slice(plan, &adoption_indexes));
        }
    }
    ProjectionApplySlices {
        plans: slices,
        deferred,
        conflicts,
    }
}

fn projection_plan_slice(plan: &ProjectionPlan, indexes: &[usize]) -> ProjectionPlan {
    ProjectionPlan {
        schema_version: plan.schema_version,
        actions: indexes
            .iter()
            .map(|index| plan.actions[*index].clone())
            .collect(),
        action_ids: indexes
            .iter()
            .map(|index| plan.action_ids[*index].clone())
            .collect(),
        trust_requirements: indexes
            .iter()
            .map(|index| plan.trust_requirements[*index])
            .collect(),
        warnings: plan.warnings.clone(),
        plan_digest: plan.plan_digest.clone(),
    }
}

pub async fn import_to_source_plan(
    default_root: Utf8PathBuf,
    asset_root: Utf8PathBuf,
    deploy_base: Utf8PathBuf,
    kind: AssetKind,
    name: String,
    source_platform: PlatformId,
    replace: bool,
) -> Result<ImportPlan, String> {
    blocking(move || {
        let roots = roots(default_root, asset_root, deploy_base);
        let request = import_request_for_sync_roots(kind, &name, source_platform, &roots, replace)?;
        build_import_plan(&[request])
    })
    .await
}

pub async fn import_to_source_apply(
    default_root: Utf8PathBuf,
    asset_root: Utf8PathBuf,
    deploy_base: Utf8PathBuf,
    kind: AssetKind,
    name: String,
    source_platform: PlatformId,
    replace: bool,
    expected_plan_digest: String,
    selected_action_ids: Vec<String>,
) -> Result<ImportApplyReport, String> {
    blocking(move || {
        let roots = roots(default_root, asset_root, deploy_base);
        let request = import_request_for_sync_roots(kind, &name, source_platform, &roots, replace)?;
        let plan = build_import_plan(&[request])?;
        if plan.plan_digest != expected_plan_digest {
            return Err(CoreError::InvalidPath(
                "reviewed import plan digest is stale or does not match current source-first import"
                    .to_owned(),
            ));
        }
        apply_import_plan(
            &plan,
            &ImportApplyOptions::new(expected_plan_digest, selected_action_ids),
            &roots.deploy_base.join(".ai-config-migrations"),
        )
    })
    .await
}

pub async fn get_detail(
    default_root: Utf8PathBuf,
    asset_root: Utf8PathBuf,
    deploy_base: Utf8PathBuf,
    kind: AssetKind,
    name: String,
) -> Result<AssetFileDetail, String> {
    blocking(move || {
        let scope = scope_roots(&default_root, &asset_root, &deploy_base);
        asset_ops::get_detail(&scope, kind, &name)
    })
    .await
}

pub async fn save_content(
    default_root: Utf8PathBuf,
    asset_root: Utf8PathBuf,
    deploy_base: Utf8PathBuf,
    kind: AssetKind,
    name: String,
    content: String,
) -> Result<String, String> {
    blocking(move || {
        let scope = scope_roots(&default_root, &asset_root, &deploy_base);
        asset_ops::save_content(&scope, kind, &name, &content)
    })
    .await
}

pub async fn retract_source(
    default_root: Utf8PathBuf,
    asset_root: Utf8PathBuf,
    deploy_base: Utf8PathBuf,
    kind: AssetKind,
    name: String,
) -> Result<String, String> {
    blocking(move || {
        let scope = scope_roots(&default_root, &asset_root, &deploy_base);
        asset_ops::retract_source(&scope, kind, &name)
    })
    .await
}

pub async fn delete_source(
    default_root: Utf8PathBuf,
    asset_root: Utf8PathBuf,
    deploy_base: Utf8PathBuf,
    kind: AssetKind,
    name: String,
) -> Result<String, String> {
    blocking(move || {
        let scope = scope_roots(&default_root, &asset_root, &deploy_base);
        asset_ops::delete_source(&scope, kind, &name)
    })
    .await
}

pub async fn read_platform_preview(
    kind: AssetKind,
    path: String,
    name: String,
) -> Result<AssetFileDetail, String> {
    let platform_path = camino::Utf8PathBuf::from(path);
    blocking(move || asset_ops::read_platform_preview(kind, &platform_path, &name)).await
}

pub async fn add_remote_skill(
    default_root: Utf8PathBuf,
    asset_root: Utf8PathBuf,
    deploy_base: Utf8PathBuf,
    source: String,
    skill_name: Option<String>,
    target_platform: PlatformId,
) -> Result<String, String> {
    blocking(move || {
        let scope = scope_roots(&default_root, &asset_root, &deploy_base);
        skills_add::add_remote_skill(
            &scope,
            SkillsAddParams {
                source: &source,
                skill_name: skill_name.as_deref(),
                target_platform,
                deploy_base: &deploy_base,
                asset_root: &asset_root,
            },
        )
    })
    .await
}

pub async fn add_remote_skills_batch(
    default_root: Utf8PathBuf,
    asset_root: Utf8PathBuf,
    deploy_base: Utf8PathBuf,
    items: Vec<SkillAddBatchItem>,
    platforms: Vec<PlatformId>,
) -> Result<SkillAddBatchOutcome, String> {
    blocking(move || {
        let scope = scope_roots(&default_root, &asset_root, &deploy_base);
        skills_add::add_remote_skills_batch(&scope, &items, &platforms, &deploy_base, &asset_root)
    })
    .await
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn reviewed_projection_apply_requires_the_exact_plan_digest_and_creates_no_target_before_apply(
    ) {
        let temp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).unwrap();
        let global = root.join("global");
        let project_assets = root.join("repo/.ai-config");
        let deploy_base = root.join("repo");
        std::fs::create_dir_all(global.join("skills").as_std_path()).unwrap();
        std::fs::create_dir_all(project_assets.join("skills/review").as_std_path()).unwrap();
        std::fs::write(
            project_assets.join("skills/review/SKILL.md").as_std_path(),
            "---\nname: review\n---\ncanonical",
        )
        .unwrap();

        let review = projection_plan(
            global.clone(),
            project_assets.clone(),
            deploy_base.clone(),
            false,
        )
        .await
        .unwrap();
        assert!(!deploy_base.join(".agents/skills/review").exists());

        let apply = projection_apply(
            global,
            project_assets,
            deploy_base.clone(),
            false,
            review.plan_digest,
        )
        .await
        .unwrap();

        assert!(apply.changed > 0);
        assert!(deploy_base.join(".agents/skills/review").is_symlink());
    }

    #[tokio::test]
    async fn reviewed_import_applies_only_its_selected_action_id() {
        let temp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).unwrap();
        let global = root.join("global");
        let project_assets = root.join("repo/.ai-config");
        let deploy_base = root.join("repo");
        std::fs::create_dir_all(deploy_base.join(".cursor/skills/review").as_std_path()).unwrap();
        std::fs::write(
            deploy_base
                .join(".cursor/skills/review/SKILL.md")
                .as_std_path(),
            "platform skill",
        )
        .unwrap();

        let plan = import_to_source_plan(
            global.clone(),
            project_assets.clone(),
            deploy_base.clone(),
            AssetKind::Skill,
            "review".to_owned(),
            PlatformId::Cursor,
            false,
        )
        .await
        .unwrap();
        assert!(!project_assets.join("skills/review/SKILL.md").exists());

        let report = import_to_source_apply(
            global,
            project_assets.clone(),
            deploy_base,
            AssetKind::Skill,
            "review".to_owned(),
            PlatformId::Cursor,
            false,
            plan.plan_digest,
            vec![plan.actions[0].action_id.clone()],
        )
        .await
        .unwrap();
        assert_eq!(report.applied, 1);
        assert_eq!(
            std::fs::read_to_string(project_assets.join("skills/review/SKILL.md").as_std_path())
                .unwrap(),
            "platform skill"
        );
    }

    #[tokio::test]
    async fn codex_mcp_import_and_equivalent_adoption_preserve_target_bytes() {
        let temp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).unwrap();
        let global = root.join("global");
        let project_assets = root.join("repo/.ai-config");
        let deploy_base = root.join("repo");
        let target = deploy_base.join(".codex/config.toml");
        std::fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
        std::fs::write(
            target.as_std_path(),
            r#"model = "gpt-test"

[mcp_servers.gitea]
url = "https://mcp.example.test/mcp"
bearer_token_env_var = "PROJECT_GITEA_TOKEN"
"#,
        )
        .unwrap();
        let before = std::fs::read(target.as_std_path()).unwrap();

        let import = import_to_source_plan(
            global.clone(),
            project_assets.clone(),
            deploy_base.clone(),
            AssetKind::Mcp,
            "gitea".to_owned(),
            PlatformId::Codex,
            false,
        )
        .await
        .unwrap();
        import_to_source_apply(
            global.clone(),
            project_assets.clone(),
            deploy_base.clone(),
            AssetKind::Mcp,
            "gitea".to_owned(),
            PlatformId::Codex,
            false,
            import.plan_digest,
            vec![import.actions[0].action_id.clone()],
        )
        .await
        .unwrap();
        let canonical = project_assets.join("mcp/servers/gitea.json");
        let mut definition: serde_json::Value =
            serde_json::from_slice(&std::fs::read(canonical.as_std_path()).unwrap()).unwrap();
        definition["targets"] = serde_json::json!(["codex", "cursor"]);
        std::fs::write(
            canonical.as_std_path(),
            serde_json::to_vec_pretty(&definition).unwrap(),
        )
        .unwrap();

        let review = projection_plan(
            global.clone(),
            project_assets.clone(),
            deploy_base.clone(),
            false,
        )
        .await
        .unwrap();
        assert!(review.actions.iter().any(|action| {
            action["kind"] == "adopt_equivalent"
                && action["reason_code"] == "equivalent_mcp_entries_unmanaged"
        }));
        let applied = projection_apply(
            global.clone(),
            project_assets.clone(),
            deploy_base.clone(),
            false,
            review.plan_digest,
        )
        .await
        .unwrap();
        assert_eq!(applied.changed, 2);
        assert_eq!(std::fs::read(target.as_std_path()).unwrap(), before);
        assert!(deploy_base.join(".cursor/mcp.json").is_file());

        let managed = projection_plan(global, project_assets, deploy_base, false)
            .await
            .unwrap();
        assert!(managed.actions.iter().any(|action| {
            action["kind"] == "noop" && action["reason_code"] == "managed_generated_noop"
        }));
    }

    #[tokio::test]
    async fn codex_same_target_adoption_defers_the_ordinary_write_and_reports_it() {
        let temp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).unwrap();
        let global = root.join("global");
        let project_assets = root.join("repo/.ai-config");
        let deploy_base = root.join("repo");
        for (name, command) in [("catalog", "catalog-mcp"), ("missing", "missing-mcp")] {
            let source = project_assets.join(format!("mcp/servers/{name}.json"));
            std::fs::create_dir_all(source.parent().unwrap().as_std_path()).unwrap();
            std::fs::write(
                source.as_std_path(),
                format!(r#"{{"targets":["codex"],"config":{{"command":"{command}"}}}}"#),
            )
            .unwrap();
        }
        let target = deploy_base.join(".codex/config.toml");
        std::fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
        std::fs::write(
            target.as_std_path(),
            "[mcp_servers.catalog]\ncommand = \"catalog-mcp\"\n",
        )
        .unwrap();
        let before = std::fs::read(target.as_std_path()).unwrap();

        let first = projection_plan(
            global.clone(),
            project_assets.clone(),
            deploy_base.clone(),
            false,
        )
        .await
        .unwrap();
        assert!(first
            .actions
            .iter()
            .any(|action| action["kind"] == "adopt_equivalent"));
        assert!(first
            .actions
            .iter()
            .any(|action| action["kind"] == "upsert_generated_batch"));
        let adopted = projection_apply(
            global.clone(),
            project_assets.clone(),
            deploy_base.clone(),
            false,
            first.plan_digest,
        )
        .await
        .unwrap();
        assert_eq!(adopted.changed, 1);
        assert_eq!(adopted.skipped, 1);
        assert_eq!(std::fs::read(target.as_std_path()).unwrap(), before);

        let second = projection_plan(
            global.clone(),
            project_assets.clone(),
            deploy_base.clone(),
            false,
        )
        .await
        .unwrap();
        let synced = projection_apply(
            global,
            project_assets,
            deploy_base,
            false,
            second.plan_digest,
        )
        .await
        .unwrap();
        assert_eq!(synced.changed, 1);
        assert!(std::fs::read_to_string(target.as_std_path())
            .unwrap()
            .contains("mcp_servers.missing"));
    }

    #[tokio::test]
    async fn codex_foreign_sibling_is_reported_but_does_not_block_equivalent_adoption() {
        let temp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).unwrap();
        let global = root.join("global");
        let project_assets = root.join("repo/.ai-config");
        let deploy_base = root.join("repo");
        for (name, command) in [("catalog", "catalog-mcp"), ("foreign", "canonical-mcp")] {
            let source = project_assets.join(format!("mcp/servers/{name}.json"));
            std::fs::create_dir_all(source.parent().unwrap().as_std_path()).unwrap();
            std::fs::write(
                source.as_std_path(),
                format!(r#"{{"targets":["codex"],"config":{{"command":"{command}"}}}}"#),
            )
            .unwrap();
        }
        let target = deploy_base.join(".codex/config.toml");
        std::fs::create_dir_all(target.parent().unwrap().as_std_path()).unwrap();
        std::fs::write(
            target.as_std_path(),
            r#"[mcp_servers.catalog]
command = "catalog-mcp"

[mcp_servers.foreign]
command = "external-mcp"
"#,
        )
        .unwrap();
        let before = std::fs::read(target.as_std_path()).unwrap();
        let review = projection_plan(
            global.clone(),
            project_assets.clone(),
            deploy_base.clone(),
            false,
        )
        .await
        .unwrap();
        assert!(review.blocking_reason.is_some());
        assert!(review
            .actions
            .iter()
            .any(|action| action["kind"] == "adopt_equivalent"));

        let applied = projection_apply(
            global,
            project_assets,
            deploy_base,
            false,
            review.plan_digest,
        )
        .await
        .unwrap();
        assert_eq!(applied.changed, 1);
        assert_eq!(applied.conflict, 1);
        assert_eq!(std::fs::read(target.as_std_path()).unwrap(), before);
    }
}
