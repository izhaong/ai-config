//! Tauri command 与 `ai-config-core::asset_ops` 之间的薄桥。

use std::collections::BTreeMap;

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
use ai_config_core::projection::planner::{McpSecretAvailability, ProjectionOperation};
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
        if let Some(reason) = reviewed.blocking_reason {
            return Err(CoreError::InvalidPath(format!(
                "source-first projection cannot apply while the reviewed plan contains a blocking action: {reason}"
            )));
        }

        let store = Store::open_at(ledger_path(&roots).as_std_path())
            .map_err(|error| CoreError::ProjectionLedger(error.to_string()))?;
        let ledger = store.projections();
        let rebuilt = build_projection_review(&roots, operation, &ledger, &secrets)?;
        if rebuilt.plan_digest != expected_plan_digest || rebuilt.blocking_reason.is_some() {
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
        let reports = apply_projection_plans_transactionally(
            rebuilt
                .plans
                .iter()
                .filter(|plan| !plan.actions.is_empty())
                .map(|plan| (plan, ApplyOptions::for_plan(plan))),
            &context,
        )
        .map_err(|error| error.error)?;
        Ok(reports.into_iter().fold(
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
        ))
    })
    .await
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
}
