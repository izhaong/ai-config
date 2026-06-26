//! Agent / MCP 可调用的 JSON API（薄封装 core + lifecycle 报告）。

use camino::{Utf8Path, Utf8PathBuf};
use schemars::JsonSchema;

use ai_config_core::asset_ops::{self, ScopeRoots};
use ai_config_core::doctor;
use ai_config_core::error::CoreError;
use ai_config_core::model::{AssetKind, PlatformId};
use ai_config_core::paths;
use ai_config_core::source;

use crate::lifecycle::{ListReport, StatusReport, SyncReport};

/// 资产根扫描摘要（MCP `ai_config_env`）。
#[derive(Debug, serde::Serialize, JsonSchema)]
pub struct EnvSummary {
    pub root: String,
    pub skills: usize,
    pub rules: usize,
    pub agents: usize,
    pub commands: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_json: Option<String>,
}

/// 解析可选 root；空则使用 `~/.ai-config`。
pub fn resolve_scope_root(root: Option<&str>) -> Utf8PathBuf {
    match root.filter(|s| !s.is_empty()) {
        Some(s) => paths::resolve_asset_root(Utf8Path::new(s)),
        None => paths::discover_global_asset_root(),
    }
}

fn with_scope<R>(root: &Utf8Path, f: impl FnOnce(ScopeRoots<'_>) -> R) -> R {
    let deploy_base = paths::home_dir();
    let scope = ScopeRoots {
        default_root: root,
        asset_root: root,
        deploy_base: &deploy_base,
    };
    f(scope)
}

pub fn parse_kind(s: &str) -> Result<AssetKind, String> {
    match s.to_lowercase().as_str() {
        "skill" | "skills" => Ok(AssetKind::Skill),
        "rule" | "rules" => Ok(AssetKind::Rule),
        "agent" | "agents" => Ok(AssetKind::Agent),
        "command" | "commands" => Ok(AssetKind::Command),
        "mcp" => Ok(AssetKind::Mcp),
        other => Err(format!(
            "未知资产类型 `{other}`；可用: skill, rule, agent, command, mcp"
        )),
    }
}

pub fn parse_platform(s: &str) -> Result<PlatformId, String> {
    match s.to_lowercase().as_str() {
        "cursor" => Ok(PlatformId::Cursor),
        "codex" => Ok(PlatformId::Codex),
        "claude" | "claudecode" | "claude-code" => Ok(PlatformId::Claude),
        "hermes" => Ok(PlatformId::Hermes),
        "aiconfig" | "ai-config" => Ok(PlatformId::AiConfig),
        other => Err(format!(
            "未知平台 `{other}`；可用: cursor, codex, claude, hermes, aiconfig"
        )),
    }
}

pub fn list(root: &Utf8Path) -> Result<ListReport, CoreError> {
    crate::lifecycle::list_report(root)
}

pub fn status(root: &Utf8Path) -> Result<StatusReport, CoreError> {
    crate::lifecycle::status_report(root)
}

pub fn sync(root: &Utf8Path) -> Result<SyncReport, CoreError> {
    crate::lifecycle::sync_report(root, false)
}

pub fn doctor(root: &Utf8Path) -> Result<doctor::DoctorReport, CoreError> {
    doctor::compute_report(root)
}

pub fn show(
    root: &Utf8Path,
    kind: AssetKind,
    name: &str,
) -> Result<asset_ops::AssetFileDetail, CoreError> {
    with_scope(root, |scope| asset_ops::get_detail(&scope, kind, name))
}

pub fn save(
    root: &Utf8Path,
    kind: AssetKind,
    name: &str,
    content: &str,
) -> Result<String, CoreError> {
    with_scope(root, |scope| {
        asset_ops::save_content(&scope, kind, name, content)
    })
}

pub fn deploy(
    root: &Utf8Path,
    kind: AssetKind,
    name: &str,
    platform: PlatformId,
) -> Result<String, CoreError> {
    with_scope(root, |scope| {
        asset_ops::deploy(&scope, kind, name, platform)
    })
}

pub fn retract(
    root: &Utf8Path,
    kind: AssetKind,
    name: &str,
    platform: PlatformId,
) -> Result<String, CoreError> {
    with_scope(root, |scope| {
        asset_ops::retract(&scope, kind, name, platform)
    })
}

pub fn scan_summary(root: &Utf8Path) -> Result<EnvSummary, CoreError> {
    let scan = source::scan_project_root(root)?;
    Ok(EnvSummary {
        root: root.as_str().to_string(),
        skills: scan.skills.len(),
        rules: scan.rules.len(),
        agents: scan.agents.len(),
        commands: scan.commands.len(),
        mcp_json: scan.mcp_json.as_ref().map(|p| p.as_str().to_string()),
    })
}
