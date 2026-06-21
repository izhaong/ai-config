//! `ai-config doctor` 诊断逻辑（GUI / CLI 共用）。

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::error::CoreError;
use crate::link::{self, LinkHealth};
use crate::materialize;
use crate::mcp_json;
use crate::model::{AssetKind, PlatformId};
use crate::path_independence;
use crate::platform::{self, platform_label};
use crate::source;
use crate::sync;
use crate::template::McpSyncState;

/// Doctor 结构化报告（`--json` / GUI）。
#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub broken: usize,
    pub wrong_source: usize,
    pub wrong_type: usize,
    pub missing_secrets: Vec<MissingSecret>,
    pub unregistered_projects: Vec<String>,
    pub platform_capability_issues: Vec<PlatformCapabilityIssue>,
    pub exit_code: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct MissingSecret {
    pub server: String,
    pub key: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlatformCapabilityIssue {
    pub platform: String,
    pub kind: String,
    pub reason: String,
}

/// 对默认资产根做一次完整健康检查。
pub fn compute_report(default_root: &Utf8Path) -> Result<DoctorReport, CoreError> {
    let scan = source::scan_project_root(default_root)?;
    let assets = flat_assets(&scan);

    let mut report = DoctorReport {
        broken: 0,
        wrong_source: 0,
        wrong_type: 0,
        missing_secrets: Vec::new(),
        unregistered_projects: Vec::new(),
        platform_capability_issues: Vec::new(),
        exit_code: 0,
    };

    for (kind, name, src) in &assets {
        for plat in platform::deploy_platform_ids() {
            let state = describe_link_state(*kind, name, src, plat, default_root);
            match state.as_str() {
                "broken" => report.broken += 1,
                "wrong_source" => report.wrong_source += 1,
                "wrong_type" => report.wrong_type += 1,
                _ => {}
            }
        }
    }

    if assets.is_empty() {
        report
            .unregistered_projects
            .push(format!("{default_root} (no assets found)"));
    }

    for issue in platform::collect_capability_issues(&crate::paths::global_deploy_base()) {
        report.platform_capability_issues.push(PlatformCapabilityIssue {
            platform: platform_label(issue.platform).to_string(),
            kind: issue.kind,
            reason: issue.reason,
        });
    }

    report.exit_code = 0;
    Ok(report)
}

fn dest_is_symlink(path: &Utf8Path) -> bool {
    std::fs::symlink_metadata(path.as_std_path())
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// 将各 IDE 平台目录中仍指向 ai-config 源的 **symlink / 同 inode** 下发迁移为实体硬拷贝。
pub fn materialize_legacy_symlink_deploys(
    default_root: &Utf8Path,
) -> Result<Vec<String>, CoreError> {
    let scan = source::scan_project_root(default_root)?;
    let assets = flat_assets(&scan);
    let deploy_base = crate::paths::global_deploy_base();
    let mut repaired = Vec::new();

    for (kind, name, src) in assets {
        if kind == AssetKind::Mcp {
            continue;
        }
        for plat in platform::deploy_platform_ids() {
            if !platform::supports_at_scope(plat, kind, &deploy_base) {
                continue;
            }
            let Some(dest) = sync::asset_dest_for_at_base(plat, kind, &name, &src, &deploy_base)
            else {
                continue;
            };
            if try_materialize_legacy_dest(&dest, kind, &src)? {
                repaired.push(format!("{kind:?}/{name} → {}", platform_label(plat)));
            }
        }
    }

    repaired.extend(materialize_orphan_symlinks_under_ai_config(default_root)?);
    Ok(repaired)
}

fn try_materialize_legacy_dest(
    dest: &Utf8Path,
    kind: AssetKind,
    src: &Utf8Path,
) -> Result<bool, CoreError> {
    let link_src = sync::link_src_for_create(kind, src);
    let needs_repair =
        dest_is_symlink(dest) || path_independence::paths_alias(dest, &link_src);
    if !needs_repair {
        return Ok(false);
    }
    materialize::deploy(&link_src, dest)?;
    Ok(true)
}

/// 扫描各平台目录：凡 symlink 指向 `default_root` 下资产的，一律迁移为实体硬拷贝
/// （含 Hermes agents 等 capability 表未列出的历史下发）。
fn materialize_orphan_symlinks_under_ai_config(
    default_root: &Utf8Path,
) -> Result<Vec<String>, CoreError> {
    let mut repaired = Vec::new();
    let ai_canon = std::fs::canonicalize(default_root.as_std_path())
        .map(|p| Utf8PathBuf::from(p.to_string_lossy().into_owned()))
        .unwrap_or_else(|_| default_root.to_path_buf());

    for plat in platform::deploy_platform_ids() {
        let adapter = match platform::for_id(plat) {
            Ok(a) => a,
            Err(_) => continue,
        };
        let dirs = [
            adapter.skills_dir(),
            adapter.rules_dir(),
            adapter.agents_dir(),
            adapter.commands_dir(),
        ];
        for dir in dirs {
            if !dir.is_dir() {
                continue;
            }
            for entry in std::fs::read_dir(dir.as_std_path()).into_iter().flatten().flatten() {
                let path = Utf8PathBuf::from(entry.path().to_string_lossy().into_owned());
                if !dest_is_symlink(&path) {
                    continue;
                }
                let Ok(target) = std::fs::read_link(entry.path()) else {
                    continue;
                };
                let mut resolved = Utf8PathBuf::from(target.to_string_lossy().into_owned());
                if resolved.is_relative() {
                    if let Some(parent) = path.parent() {
                        resolved = parent.join(resolved);
                    }
                }
                let under_ai = std::fs::canonicalize(resolved.as_std_path())
                    .ok()
                    .map(|p| {
                        let u = Utf8PathBuf::from(p.to_string_lossy().into_owned());
                        u.starts_with(&ai_canon)
                    })
                    .unwrap_or(false);
                if !under_ai {
                    continue;
                }
                let copy_src = materialize::resolve_copy_source(&resolved);
                materialize::deploy(&copy_src, &path)?;
                repaired.push(format!(
                    "orphan {} → {}",
                    path.file_name().unwrap_or("?"),
                    platform_label(plat)
                ));
            }
        }
    }
    Ok(repaired)
}

fn flat_assets(scan: &source::ScanResult) -> Vec<(AssetKind, String, Utf8PathBuf)> {
    let mut out: Vec<(AssetKind, String, Utf8PathBuf)> = Vec::new();
    for p in &scan.skills {
        if let Some(name) = p
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string())
        {
            out.push((AssetKind::Skill, name, p.clone()));
        }
    }
    for p in &scan.rules {
        if let Some(name) = p.file_stem().map(|n| n.to_string()) {
            out.push((AssetKind::Rule, name, p.clone()));
        }
    }
    for p in &scan.commands {
        if let Some(name) = p.file_stem().map(|n| n.to_string()) {
            out.push((AssetKind::Command, name, p.clone()));
        }
    }
    if let Some(ref path) = scan.mcp_json {
        out.push((
            AssetKind::Mcp,
            mcp_json::MCP_ASSET_NAME.to_string(),
            path.clone(),
        ));
    }
    for p in &scan.agents {
        let name = if p.is_dir() {
            p.file_name().map(|n| n.to_string())
        } else {
            p.file_stem().map(|n| n.to_string())
        };
        if let Some(name) = name {
            out.push((AssetKind::Agent, name, p.clone()));
        }
    }
    out
}

fn describe_link_state(
    kind: AssetKind,
    name: &str,
    src: &Utf8Path,
    platform: PlatformId,
    default_root: &Utf8Path,
) -> String {
    let adapter = match platform::for_id(platform) {
        Ok(a) => a,
        Err(_) => return "unmanaged".to_string(),
    };
    let (dest, expected_src) = match kind {
        AssetKind::Skill => {
            let dest = adapter.skills_dir().join(name);
            let expected = default_root.join("skills").join(name);
            (dest, expected)
        }
        AssetKind::Rule => {
            let dest = adapter.rules_dir().join(format!("{name}.mdc"));
            let expected = default_root.join("rules").join(format!("{name}.mdc"));
            (dest, expected)
        }
        AssetKind::Command => {
            let dest = adapter.commands_dir().join(format!("{name}.md"));
            let expected = default_root.join("commands").join(format!("{name}.md"));
            (dest, expected)
        }
        AssetKind::Agent => {
            let dest = sync::asset_dest_for(platform, kind, name, src)
                .unwrap_or_else(|| adapter.agents_dir().join(name));
            let expected = sync::agent_link_src(src);
            (dest, expected)
        }
        AssetKind::Mcp => {
            let dest = adapter.mcp_deploy_path();
            return match mcp_json::mcp_json_file_sync_state(src, &dest, platform) {
                McpSyncState::Linked => "linked".to_string(),
                McpSyncState::Unlinked => "missing".to_string(),
                McpSyncState::WrongValue => "wrong_source".to_string(),
                McpSyncState::Broken => "broken".to_string(),
            };
        }
    };
    if !dest.exists() && dest.as_std_path().symlink_metadata().is_err() {
        return "missing".to_string();
    }
    match materialize::check(&dest, &expected_src) {
        materialize::DeployHealth::Linked { .. } => "linked".to_string(),
        materialize::DeployHealth::Broken => "broken".to_string(),
        materialize::DeployHealth::Unlinked => match link::check(&dest, &expected_src) {
            LinkHealth::Linked { .. } => "linked".to_string(),
            LinkHealth::Broken { .. } => "broken".to_string(),
            LinkHealth::WrongSource { .. } => "wrong_source".to_string(),
            LinkHealth::WrongType { .. } => "wrong_type".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_report_empty_root_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        crate::paths::ensure_user_asset_layout(&root).unwrap();
        let r = compute_report(&root).unwrap();
        assert_eq!(r.exit_code, 0);
    }
}
