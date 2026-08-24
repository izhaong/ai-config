//! 通过 [vercel-labs/skills](https://github.com/vercel-labs/skills) CLI (`npx skills add`) 安装远程 skill。
//!
//! 路径映射见 `docs/reference/vercel-skills-agent-paths.md`。

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::Command;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;
use tempfile::TempDir;

use crate::asset_ops::{self, ScopeRoots};
use crate::error::CoreError;
use crate::materialize;
use crate::model::{AssetKind, PlatformId};

/// vercel `skills add --agent` 键（与 `src/agents.ts` 一致）。
pub fn vercel_agent_key(plat: PlatformId) -> Option<&'static str> {
    match plat {
        PlatformId::Cursor => Some("cursor"),
        PlatformId::Codex => Some("codex"),
        PlatformId::Claude => Some("claude-code"),
        PlatformId::Hermes => Some("hermes-agent"),
        PlatformId::AgentsManager => None,
    }
}

pub struct SkillsAddParams<'a> {
    pub source: &'a str,
    pub skill_name: Option<&'a str>,
    pub target_platform: PlatformId,
    pub deploy_base: &'a Utf8Path,
    pub asset_root: &'a Utf8Path,
}

/// 安装远程 skill 到目标平台或 agents-manager 平台目录。
pub fn add_remote_skill(
    scope: &ScopeRoots<'_>,
    params: SkillsAddParams<'_>,
) -> Result<String, CoreError> {
    let source = params.source.trim();
    if source.is_empty() {
        return Err(CoreError::InvalidPath(
            "skill 来源不能为空（如 owner/repo 或 GitHub URL）".into(),
        ));
    }

    if params.target_platform == PlatformId::AgentsManager {
        let dir_name = add_to_agentsmanager_platform(scope.asset_root, source, params.skill_name)?;
        return Ok(format!(
            "已添加 skill `{dir_name}` 到 agents-manager 平台 ({})",
            scope.asset_root.join("skills").join(&dir_name)
        ));
    }

    let skill_name = ensure_skill_in_agentsmanager(scope.asset_root, source, params.skill_name)?;
    asset_ops::deploy(scope, AssetKind::Skill, &skill_name, params.target_platform)
}

fn add_to_agentsmanager_platform(
    asset_root: &Utf8Path,
    source: &str,
    skill_name: Option<&str>,
) -> Result<String, CoreError> {
    let temp = TempDir::new().map_err(CoreError::Io)?;
    let temp_utf8 = Utf8PathBuf::from_path_buf(temp.path().to_path_buf())
        .map_err(|_| CoreError::InvalidPath("临时目录路径非 UTF-8".into()))?;

    let args = build_agentsmanager_add_args(source, skill_name);
    run_npx(&temp_utf8, &args)?;

    let installed = find_skill_dirs_under(&temp_utf8)?;
    let picked = pick_skill_dir(&installed, skill_name)?;
    let dir_name = picked
        .file_name()
        .ok_or_else(|| CoreError::InvalidPath("无法解析 skill 目录名".into()))?
        .to_string();

    let dest = asset_root.join("skills").join(&dir_name);
    if dest.exists() {
        return Err(CoreError::InvalidPath(format!(
            "agents-manager 平台已存在 skill `{dir_name}`，请先删除或更换名称"
        )));
    }

    materialize::copy_tree(&picked, &dest)?;
    Ok(dir_name)
}

/// 确保 skill 已存在于 agents-manager 源；返回 skill 目录名（用于 deploy）。
fn ensure_skill_in_agentsmanager(
    asset_root: &Utf8Path,
    source: &str,
    skill_name: Option<&str>,
) -> Result<String, CoreError> {
    if let Some(name) = skill_name.filter(|s| !s.trim().is_empty()) {
        let name = name.trim();
        let dest = asset_root.join("skills").join(name);
        if dest.is_dir() {
            return Ok(name.to_string());
        }
        add_to_agentsmanager_platform(asset_root, source, Some(name))?;
        return Ok(name.to_string());
    }
    add_to_agentsmanager_platform(asset_root, source, None)
}

fn build_agentsmanager_add_args(source: &str, skill_name: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "skills".into(),
        "add".into(),
        source.into(),
        "--copy".into(),
        "-y".into(),
    ];
    if let Some(name) = skill_name.filter(|s| !s.trim().is_empty()) {
        args.push("--skill".into());
        args.push(name.trim().into());
    }
    args
}

fn run_npx(cwd: &Utf8Path, args: &[String]) -> Result<String, CoreError> {
    let arg_refs: Vec<&OsStr> = args.iter().map(|s| OsStr::new(s.as_str())).collect();
    let output = Command::new("npx")
        .args(&arg_refs)
        .current_dir(cwd.as_std_path())
        .output()
        .map_err(|e| {
            CoreError::InvalidPath(format!(
                "无法执行 npx skills add（请确认已安装 Node.js/npx）: {e}"
            ))
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        let detail = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        return Err(CoreError::InvalidPath(format!(
            "npx skills add 失败: {detail}"
        )));
    }

    Ok(if stdout.trim().is_empty() {
        stderr
    } else {
        stdout
    })
}

fn find_skill_dirs_under(root: &Utf8Path) -> Result<Vec<Utf8PathBuf>, CoreError> {
    let mut out = Vec::new();
    walk_skill_dirs(root.as_std_path(), &mut out)?;
    out.sort();
    Ok(out
        .into_iter()
        .map(|p| Utf8PathBuf::from(p.to_string_lossy().into_owned()))
        .collect())
}

fn walk_skill_dirs(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<(), CoreError> {
    if !dir.is_dir() {
        return Ok(());
    }
    let skill_md = dir.join("SKILL.md");
    if skill_md.is_file() {
        out.push(dir.to_path_buf());
        return Ok(());
    }
    for entry in fs::read_dir(dir).map_err(CoreError::Io)? {
        let entry = entry.map_err(CoreError::Io)?;
        let path = entry.path();
        if path.is_dir() {
            walk_skill_dirs(&path, out)?;
        }
    }
    Ok(())
}

fn pick_skill_dir(
    dirs: &[Utf8PathBuf],
    skill_name: Option<&str>,
) -> Result<Utf8PathBuf, CoreError> {
    if dirs.is_empty() {
        return Err(CoreError::InvalidPath(
            "npx skills add 未产生可识别的 skill 目录（缺少 SKILL.md）".into(),
        ));
    }
    if let Some(name) = skill_name.filter(|s| !s.trim().is_empty()) {
        let name = name.trim();
        if let Some(found) = dirs
            .iter()
            .find(|d| d.file_name().map(|n| n == name).unwrap_or(false))
        {
            return Ok(found.clone());
        }
        return Err(CoreError::InvalidPath(format!(
            "未在安装结果中找到 skill `{name}`"
        )));
    }
    if dirs.len() == 1 {
        return Ok(dirs[0].clone());
    }
    Err(CoreError::InvalidPath(format!(
        "安装结果包含多个 skill（{}），请指定 skill 名称",
        dirs.iter()
            .filter_map(|d| d.file_name())
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

/// 批量安装：先写入 agents-manager 源，再 deploy 到各 IDE 平台。
#[derive(Debug, Clone)]
pub struct SkillAddBatchItem {
    pub source: String,
    pub skill_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SkillAddBatchOutcome {
    pub ok: usize,
    pub failed: usize,
    pub errors: Vec<String>,
}

pub fn add_remote_skills_batch(
    scope: &ScopeRoots<'_>,
    items: &[SkillAddBatchItem],
    platforms: &[PlatformId],
    _deploy_base: &Utf8Path,
    _asset_root: &Utf8Path,
) -> Result<SkillAddBatchOutcome, CoreError> {
    if items.is_empty() {
        return Err(CoreError::InvalidPath("未选择任何 skill".into()));
    }
    if platforms.is_empty() {
        return Err(CoreError::InvalidPath("未选择任何目标平台".into()));
    }

    let mut ok = 0usize;
    let mut failed = 0usize;
    let mut errors = Vec::new();

    for item in items {
        let skill_name = match ensure_skill_in_agentsmanager(
            scope.asset_root,
            item.source.trim(),
            item.skill_name.as_deref(),
        ) {
            Ok(name) => name,
            Err(e) => {
                failed += platforms.len();
                let label = item.skill_name.as_deref().unwrap_or(item.source.as_str());
                errors.push(format!("{label}（写入 agents-manager 源失败）: {e}"));
                continue;
            }
        };

        for plat in platforms {
            if *plat == PlatformId::AgentsManager {
                ok += 1;
                continue;
            }
            match asset_ops::deploy(scope, AssetKind::Skill, &skill_name, *plat) {
                Ok(_) => ok += 1,
                Err(e) => {
                    failed += 1;
                    errors.push(format!(
                        "{skill_name} → {}: {e}",
                        crate::platform::platform_label(*plat)
                    ));
                }
            }
        }
    }

    Ok(SkillAddBatchOutcome { ok, failed, errors })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_ide_add_args(
        source: &str,
        skill_name: Option<&str>,
        agent: &str,
        global: bool,
    ) -> Vec<String> {
        let mut args = vec![
            "skills".into(),
            "add".into(),
            source.into(),
            "--copy".into(),
            "-y".into(),
            "-a".into(),
            agent.into(),
        ];
        if global {
            args.push("-g".into());
        }
        if let Some(name) = skill_name.filter(|s| !s.trim().is_empty()) {
            args.push("--skill".into());
            args.push(name.trim().into());
        }
        args
    }

    #[test]
    fn vercel_agent_keys_match_upstream() {
        assert_eq!(vercel_agent_key(PlatformId::Claude), Some("claude-code"));
        assert_eq!(vercel_agent_key(PlatformId::Hermes), Some("hermes-agent"));
        assert_eq!(vercel_agent_key(PlatformId::AgentsManager), None);
    }

    #[test]
    fn build_ide_add_args_includes_copy_and_agent() {
        let args = build_ide_add_args(
            "vercel-labs/agent-skills",
            Some("web-design"),
            "cursor",
            true,
        );
        assert!(args.contains(&"skills".to_string()));
        assert!(args.contains(&"--copy".to_string()));
        assert!(args.contains(&"-a".to_string()));
        assert!(args.contains(&"cursor".to_string()));
        assert!(args.contains(&"-g".to_string()));
        assert!(args.contains(&"--skill".to_string()));
        assert!(args.contains(&"web-design".to_string()));
    }

    #[test]
    fn pick_skill_dir_requires_name_when_ambiguous() {
        let a = Utf8PathBuf::from("/tmp/a");
        let b = Utf8PathBuf::from("/tmp/b");
        assert!(pick_skill_dir(&[a.clone(), b], None).is_err());
        assert_eq!(
            pick_skill_dir(&[a], None).unwrap(),
            Utf8PathBuf::from("/tmp/a")
        );
    }
}
