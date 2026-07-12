//! Phase 1 业务子命令(PRD §5 / §10):
//! `install` / `uninstall` / `sync` / `status` / `list` / `show` / `doctor`。
//!
//! ## 退出码契约(PRD §9.1)
//!
//! | code | 含义                            |
//! |------|---------------------------------|
//! | 0    | 成功                            |
//! | 2    | 参数 / 配置错                   |
//! | 3    | 部分失败                        |
//! | 4    | 缺密钥                          |
//! | 5    | 文件系统 / IO 错                |
//!
//! ## 输出契约
//!
//! - `--json`:`{ ... }` 一行结构化 JSON
//! - 默认:人类可读
//! - `--quiet`:只一行 `ok: <summary>` / `fail: <msg>`

use std::process::ExitCode;

use camino::Utf8Path;
use schemars::JsonSchema;
use serde::Serialize;

use ai_config_core::error::{exit_code, CoreError};
use ai_config_core::hermes_config;
use ai_config_core::hook_adapter;
use ai_config_core::link::{self, LinkHealth};
use ai_config_core::materialize;
use ai_config_core::mcp_json;
use ai_config_core::model::{AssetKind, PlatformId, SyncAction};
use ai_config_core::paths;
use ai_config_core::platform;
use ai_config_core::secrets as core_secrets;
use ai_config_core::source;
use ai_config_core::sync;
use ai_config_core::template::McpSyncState;
use ai_config_core::workspace;

use crate::output::{emit_error_envelope, emit_json, emit_line, OutputMode};

// ── 共享:扫描 + 装载 ──────────────────────────────────────────────

/// 一次同步的"工作上下文":扫 default_root + 装 secrets + 列所有平台适配器。
struct SyncContext {
    scan: source::ScanResult,
    /// secrets 已加载(key→value)。**绝不**回显 value 到日志 / stdout / JSON。
    secrets_pairs: Vec<(String, String)>,
    actions: Vec<SyncAction>,
    deploy_base: camino::Utf8PathBuf,
}

fn load_context(default_root: &Utf8Path) -> Result<SyncContext, CoreError> {
    let roots = paths::resolve_sync_roots(default_root);
    load_context_from_roots(&roots)
}

fn load_context_from_roots(roots: &paths::SyncRoots) -> Result<SyncContext, CoreError> {
    let scan = source::scan_with_override(&roots.asset_root, &roots.global_default)?;
    let name = roots
        .repo_root
        .file_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "default".to_string());
    let project = ai_config_core::model::Project::new(name, roots.repo_root.clone());
    let actions = sync::compute_for_sync_roots(&project, roots)?;
    let pairs = core_secrets::load()?;
    Ok(SyncContext {
        scan,
        secrets_pairs: pairs,
        actions,
        deploy_base: roots.deploy_base.clone(),
    })
}

fn load_context_for_member(
    workspace_root: &Utf8Path,
    member: &Utf8Path,
) -> Result<SyncContext, CoreError> {
    let roots = workspace::resolve_member_sync_roots(member, workspace_root);
    load_context_from_roots(&roots)
}

fn all_platforms() -> [PlatformId; 4] {
    [
        PlatformId::Cursor,
        PlatformId::Codex,
        PlatformId::Claude,
        PlatformId::Hermes,
    ]
}

fn platform_label(p: PlatformId) -> &'static str {
    match p {
        PlatformId::AiConfig => "aiconfig",
        PlatformId::Cursor => "cursor",
        PlatformId::Codex => "codex",
        PlatformId::Claude => "claude",
        PlatformId::Hermes => "hermes",
    }
}

// ── 共享:执行 SyncAction ──────────────────────────────────────────

/// 一条动作的执行结果。
#[derive(Debug, Clone, Serialize, JsonSchema)]
struct Outcome {
    label: String,
    platform: String,
    kind: String,
    result: &'static str, // "ok" | "skipped" | "failed"
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
}

impl Outcome {
    fn ok(label: String, platform: PlatformId, kind: &str) -> Self {
        Self {
            label,
            platform: platform_label(platform).to_string(),
            kind: kind.to_string(),
            result: "ok",
            error: None,
            hint: None,
        }
    }
    fn skipped(label: String, platform: PlatformId, kind: &str, reason: &str) -> Self {
        Self {
            label,
            platform: platform_label(platform).to_string(),
            kind: kind.to_string(),
            result: "skipped",
            error: Some(reason.to_string()),
            hint: None,
        }
    }
    fn failed(label: String, platform: PlatformId, kind: &str, err: &CoreError) -> Self {
        Self {
            label,
            platform: platform_label(platform).to_string(),
            kind: kind.to_string(),
            result: "failed",
            error: Some(err.to_string()),
            hint: err.hint().map(str::to_owned),
        }
    }
}

fn infer_kind_from_dest(dest: &camino::Utf8Path) -> &'static str {
    let s = dest.as_str();
    if s.contains("/hooks/") && !s.ends_with("/hooks") {
        return "hook";
    }
    if s.contains("/skills/") || s.ends_with("/skills") {
        "skill"
    } else if s.contains("/rules/") {
        "rule"
    } else if s.contains("/commands/") {
        "command"
    } else if s.contains("/agents/") || s.contains("/subagents/") {
        "agent"
    } else {
        "unknown"
    }
}

/// 跑一次完整 sync(per-item × per-platform 动作展开),返回 outcomes。
fn execute_all_actions(ctx: &SyncContext) -> Vec<Outcome> {
    let mut out: Vec<Outcome> = Vec::new();
    let mut mcp_renders_done = std::collections::HashSet::new();

    for action in &ctx.actions {
        match action {
            SyncAction::DeployHook {
                platform,
                name,
                asset_root,
                deploy_base,
                ..
            } => {
                let label = format!("DeployHook {name} → {}", platform_label(*platform));
                match hook_adapter::deploy(asset_root, deploy_base, name, *platform) {
                    Ok(msg) => out.push(Outcome::ok(msg, *platform, "hook")),
                    Err(e) => out.push(Outcome::failed(label, *platform, "hook", &e)),
                }
            }
            SyncAction::Create {
                platform,
                dest,
                src,
                item_id: _,
            } => {
                let kind = infer_kind_from_dest(dest);
                let link_src = match kind {
                    "skill" => ai_config_core::sync::link_src_for_create(AssetKind::Skill, src),
                    "rule" => ai_config_core::sync::link_src_for_create(AssetKind::Rule, src),
                    "command" => ai_config_core::sync::link_src_for_create(AssetKind::Command, src),
                    "agent" => ai_config_core::sync::link_src_for_create(AssetKind::Agent, src),
                    _ => src.clone(),
                };
                let label = format!("Create {} → {}", kind, dest);
                if paths::is_project_deploy_base(&ctx.deploy_base)
                    && dest.exists()
                    && !materialize::is_managed_deploy(dest)
                    && kind != "hook"
                {
                    out.push(Outcome::skipped(
                        label,
                        *platform,
                        kind,
                        "项目已有非托管文件，跳过覆盖",
                    ));
                    continue;
                }
                // 确保父目录存在
                if let Some(parent) = dest.parent() {
                    if !parent.as_str().is_empty() && !parent.exists() {
                        if let Err(e) = std::fs::create_dir_all(parent.as_std_path()) {
                            out.push(Outcome {
                                label: label.clone(),
                                platform: platform_label(*platform).to_string(),
                                kind: kind.to_string(),
                                result: "failed",
                                error: Some(format!("create_dir_all {} 失败: {e}", parent)),
                                hint: Some("检查父目录权限".to_string()),
                            });
                            continue;
                        }
                    }
                }
                match materialize::deploy(&link_src, dest) {
                    Ok(()) => {
                        if *platform == PlatformId::Hermes && kind == "skill" {
                            if let Some(skills_root) = dest.parent() {
                                if let Err(e) = hermes_config::after_skill_deploy(skills_root) {
                                    out.push(Outcome::failed(
                                        format!("Hermes skills external_dirs ({skills_root})"),
                                        *platform,
                                        kind,
                                        &e,
                                    ));
                                    continue;
                                }
                            }
                        }
                        out.push(Outcome::ok(label, *platform, kind));
                    }
                    Err(e) => out.push(Outcome::failed(label, *platform, kind, &e)),
                }
            }
            SyncAction::RenderMcp { platform, .. } => {
                if mcp_renders_done.insert(*platform) {
                    let Some(ref src) = ctx.scan.mcp_json else {
                        continue;
                    };
                    let label = format!("RenderMcp {}", platform_label(*platform));
                    let adapter = match platform::for_scope(*platform, &ctx.deploy_base) {
                        Ok(a) => a,
                        Err(e) => {
                            out.push(Outcome::failed(label, *platform, "mcp", &e));
                            continue;
                        }
                    };
                    let dest = adapter.mcp_deploy_path();
                    let asset_root = src.parent().unwrap_or(src);
                    let server_names = match mcp_json::list_server_names(asset_root) {
                        Ok(n) => n,
                        Err(e) => {
                            out.push(Outcome::failed(label, *platform, "mcp", &e));
                            continue;
                        }
                    };
                    let mut ok = true;
                    for server_name in &server_names {
                        let config = match mcp_json::get_server_config(asset_root, server_name) {
                            Ok(Some(c)) => c,
                            Ok(None) => continue,
                            Err(e) => {
                                out.push(Outcome::failed(
                                    format!("{label} `{server_name}`"),
                                    *platform,
                                    "mcp",
                                    &e,
                                ));
                                ok = false;
                                break;
                            }
                        };
                        if let Err(e) = mcp_json::upsert_server_on_platform(
                            *platform,
                            &dest,
                            server_name,
                            &config,
                            Some(src),
                        ) {
                            out.push(Outcome::failed(
                                format!("{label} `{server_name}`"),
                                *platform,
                                "mcp",
                                &e,
                            ));
                            ok = false;
                            break;
                        }
                    }
                    if ok {
                        out.push(Outcome::ok(label, *platform, "mcp"));
                    }
                }
            }
            SyncAction::Linked { .. } | SyncAction::Unlink { .. } | SyncAction::Retract { .. } => {
                // install / sync 路径不处理
            }
        }
    }
    out
}

// ── 1. install ──────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct InstallReport {
    ok: bool,
    config_dir: String,
    config_dir_created: bool,
    assets_synced: usize,
    platforms: usize,
    outcomes: Vec<Outcome>,
    secrets: SecretsSummary,
    exit_code: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace: Option<WorkspaceInstallReport>,
}

#[derive(Debug, Serialize)]
struct WorkspaceInstallReport {
    workspace: String,
    members: Vec<WorkspaceMemberReport>,
}

#[derive(Debug, Serialize)]
struct WorkspaceMemberReport {
    path: String,
    assets_synced: usize,
    failed: usize,
    outcomes: Vec<Outcome>,
}

#[derive(Debug, Serialize)]
struct SecretsSummary {
    file_exists: bool,
    key_count: usize,
    missing: Vec<MissingSecret>,
}

#[derive(Debug, Clone, Serialize)]
struct MissingSecret {
    server: String,
    key: String,
}

/// `ai-config install`(PRD §5 场景 A / §10 A-1)
pub fn run_install(default_root: &Utf8Path, workspace: bool, mode: OutputMode) -> ExitCode {
    if workspace {
        return run_install_workspace(default_root, mode);
    }
    // 1. 初始化 ~/.config/ai-config/(若不在)
    let config_dir = ai_config_home().join(".config").join("ai-config");
    let config_dir_created = !config_dir.exists();
    if config_dir_created {
        if let Err(e) = std::fs::create_dir_all(config_dir.as_std_path()) {
            emit_error_envelope(
                mode,
                exit_code::FS_ERROR,
                &format!("创建 {} 失败: {e}", config_dir),
                Some("检查 $HOME 权限"),
            );
            return ExitCode::from(exit_code::FS_ERROR);
        }
    }

    // 2. 扫资产
    let ctx = match load_context(default_root) {
        Ok(c) => c,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            return ExitCode::from(e.exit_code());
        }
    };

    // 3. secrets(MCP 已改为明文 mcp.json,不再校验 ${VAR})
    let missing: Vec<MissingSecret> = Vec::new();
    let secrets_summary = SecretsSummary {
        file_exists: core_secrets::default_path().exists(),
        key_count: ctx.secrets_pairs.len(),
        missing: missing.clone(),
    };

    // 4. 执行 SyncAction
    let outcomes = execute_all_actions(&ctx);
    let ok_count = outcomes
        .iter()
        .filter(|o| o.result == "ok" || o.result == "skipped")
        .count();
    let failed_count = outcomes.iter().filter(|o| o.result == "failed").count();

    // 5. 退出码
    let code = if !missing.is_empty() {
        exit_code::SECRETS_MISSING
    } else if failed_count > 0 {
        exit_code::PARTIAL_FAILURE
    } else {
        exit_code::SUCCESS
    };

    let report = InstallReport {
        ok: code == exit_code::SUCCESS,
        config_dir: config_dir.as_str().to_string(),
        config_dir_created,
        assets_synced: ok_count,
        platforms: all_platforms().len(),
        outcomes,
        secrets: secrets_summary,
        exit_code: code,
        workspace: None,
    };

    if mode.is_json() {
        emit_json(mode, &report);
    } else if !mode.is_quiet() {
        if config_dir_created {
            emit_line(mode, format!("+ 创建配置目录: {}", config_dir));
        }
        if !missing.is_empty() {
            emit_line(mode, format!("! secrets 缺 {} 个 key:", missing.len()));
            for m in &missing {
                emit_line(mode, format!("    - server `{}` 缺 `{}`", m.server, m.key));
            }
        }
        if code == exit_code::SUCCESS {
            emit_line(
                mode,
                format!(
                    "{} 个资产已下发到 {} 个平台",
                    report.assets_synced, report.platforms
                ),
            );
        } else {
            emit_line(
                mode,
                format!(
                    "{} 个资产已下发到 {} 个平台(失败 {})",
                    report.assets_synced, report.platforms, failed_count
                ),
            );
        }
    }

    ExitCode::from(code)
}

fn run_install_workspace(workspace_root: &Utf8Path, mode: OutputMode) -> ExitCode {
    let config_dir = ai_config_home().join(".config").join("ai-config");
    let config_dir_created = !config_dir.exists();
    if config_dir_created {
        if let Err(e) = std::fs::create_dir_all(config_dir.as_std_path()) {
            emit_error_envelope(
                mode,
                exit_code::FS_ERROR,
                &format!("创建 {} 失败: {e}", config_dir),
                Some("检查 $HOME 权限"),
            );
            return ExitCode::from(exit_code::FS_ERROR);
        }
    }

    let members = match workspace::discover_members(workspace_root) {
        Ok(m) => m,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            return ExitCode::from(e.exit_code());
        }
    };

    let mut member_reports = Vec::new();
    let mut all_outcomes = Vec::new();
    let mut total_ok = 0usize;
    let mut total_failed = 0usize;

    for member in &members {
        let ctx = match load_context_for_member(workspace_root, member) {
            Ok(c) => c,
            Err(e) => {
                emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
                return ExitCode::from(e.exit_code());
            }
        };
        let outcomes = execute_all_actions(&ctx);
        let ok = outcomes
            .iter()
            .filter(|o| o.result == "ok" || o.result == "skipped")
            .count();
        let failed = outcomes.iter().filter(|o| o.result == "failed").count();
        total_ok += ok;
        total_failed += failed;
        if !mode.is_quiet() && !mode.is_json() {
            emit_line(
                mode,
                format!("==> {} ({} ok, {} failed)", member, ok, failed),
            );
        }
        member_reports.push(WorkspaceMemberReport {
            path: member.as_str().to_string(),
            assets_synced: ok,
            failed,
            outcomes: outcomes.clone(),
        });
        all_outcomes.extend(outcomes);
    }

    let secrets_summary = SecretsSummary {
        file_exists: core_secrets::default_path().exists(),
        key_count: core_secrets::load().map(|p| p.len()).unwrap_or(0),
        missing: Vec::new(),
    };

    let code = if total_failed > 0 {
        exit_code::PARTIAL_FAILURE
    } else {
        exit_code::SUCCESS
    };

    let report = InstallReport {
        ok: code == exit_code::SUCCESS,
        config_dir: config_dir.as_str().to_string(),
        config_dir_created,
        assets_synced: total_ok,
        platforms: all_platforms().len(),
        outcomes: all_outcomes,
        secrets: secrets_summary,
        exit_code: code,
        workspace: Some(WorkspaceInstallReport {
            workspace: workspace_root.as_str().to_string(),
            members: member_reports,
        }),
    };

    if mode.is_json() {
        emit_json(mode, &report);
    } else if !mode.is_quiet() {
        emit_line(
            mode,
            format!(
                "workspace {}: {} 成员, {} 项下发, {} 失败",
                workspace_root,
                members.len(),
                total_ok,
                total_failed
            ),
        );
    }

    ExitCode::from(code)
}

// ── 2. uninstall ────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct UninstallReport {
    ok: bool,
    retracted: usize,
    mcp_backups: usize,
    platforms: usize,
    outcomes: Vec<Outcome>,
    exit_code: u8,
}

/// `ai-config uninstall`(PRD §5 场景 F / §10 A-2)
pub fn run_uninstall(default_root: &Utf8Path, _force: bool, mode: OutputMode) -> ExitCode {
    let ctx = match load_context(default_root) {
        Ok(c) => c,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            return ExitCode::from(e.exit_code());
        }
    };

    let mut outcomes: Vec<Outcome> = Vec::new();

    // 1. 收回所有本工具创建的 symlink(Create → unlink)
    for action in &ctx.actions {
        if let SyncAction::Create { platform, dest, .. } = action {
            let kind = infer_kind_from_dest(dest);
            let label = format!("Retract {kind} {dest}");
            match materialize::retract(dest) {
                Ok(()) => outcomes.push(Outcome::ok(label, *platform, kind)),
                Err(e) => {
                    let s = e.to_string();
                    if s.contains("不是本工具下发") {
                        outcomes.push(Outcome::skipped(label, *platform, kind, &s));
                    } else {
                        outcomes.push(Outcome::failed(label, *platform, kind, &e));
                    }
                }
            }
        }
    }

    // 2. T001: 在具名 ownership ledger 落地前，不收回任何平台 MCP。
    // 平台聚合配置可能同时包含 ai-config 与用户手工条目，整文件删除不可证明安全。
    for plat in all_platforms() {
        let label = format!("RetractMcp {}", platform_label(plat));
        outcomes.push(Outcome::skipped(
            label,
            plat,
            "mcp",
            "缺少可验证的 MCP 所有权记录；平台聚合配置保持不变",
        ));
    }

    let retracted = outcomes
        .iter()
        .filter(|o| o.label.starts_with("Retract") && o.result == "ok")
        .count();
    let mcp_retracted = outcomes
        .iter()
        .filter(|o| o.label.starts_with("RetractMcp") && o.result == "ok")
        .count();
    let failed = outcomes.iter().filter(|o| o.result == "failed").count();
    let code = if failed > 0 {
        exit_code::PARTIAL_FAILURE
    } else {
        exit_code::SUCCESS
    };

    let report = UninstallReport {
        ok: code == exit_code::SUCCESS,
        retracted,
        mcp_backups: mcp_retracted,
        platforms: all_platforms().len(),
        outcomes,
        exit_code: code,
    };

    if mode.is_json() {
        emit_json(mode, &report);
    } else if !mode.is_quiet() {
        emit_line(
            mode,
            format!("已收回 {retracted} 条链接,{mcp_retracted} 份平台 mcp.json"),
        );
        if failed > 0 {
            emit_line(mode, format!("! {failed} 条失败"));
        }
    }

    ExitCode::from(code)
}

// ── 3. sync ─────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
pub struct SyncReport {
    ok: bool,
    synced: usize,
    failed: usize,
    outcomes: Vec<Outcome>,
    exit_code: u8,
}

/// `ai-config sync` — 返回结构化报告（CLI / MCP 共用）。
pub fn sync_report(default_root: &Utf8Path, workspace: bool) -> Result<SyncReport, CoreError> {
    if workspace {
        return sync_report_workspace(default_root);
    }
    let ctx = load_context(default_root)?;
    let outcomes = execute_all_actions(&ctx);
    let synced = outcomes
        .iter()
        .filter(|o| o.result == "ok" || o.result == "skipped")
        .count();
    let failed = outcomes.iter().filter(|o| o.result == "failed").count();
    let code = if failed > 0 {
        exit_code::PARTIAL_FAILURE
    } else {
        exit_code::SUCCESS
    };
    Ok(SyncReport {
        ok: code == exit_code::SUCCESS,
        synced,
        failed,
        outcomes,
        exit_code: code,
    })
}

fn sync_report_workspace(workspace_root: &Utf8Path) -> Result<SyncReport, CoreError> {
    let members = workspace::discover_members(workspace_root)?;
    let mut outcomes = Vec::new();
    for member in &members {
        let ctx = load_context_for_member(workspace_root, member)?;
        outcomes.extend(execute_all_actions(&ctx));
    }
    let synced = outcomes
        .iter()
        .filter(|o| o.result == "ok" || o.result == "skipped")
        .count();
    let failed = outcomes.iter().filter(|o| o.result == "failed").count();
    let code = if failed > 0 {
        exit_code::PARTIAL_FAILURE
    } else {
        exit_code::SUCCESS
    };
    Ok(SyncReport {
        ok: code == exit_code::SUCCESS,
        synced,
        failed,
        outcomes,
        exit_code: code,
    })
}

/// `ai-config sync`(PRD §10 A-5)
pub fn run_sync(default_root: &Utf8Path, workspace: bool, mode: OutputMode) -> ExitCode {
    let report = match sync_report(default_root, workspace) {
        Ok(r) => r,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            return ExitCode::from(e.exit_code());
        }
    };
    let synced = report.synced;
    let failed = report.failed;
    let code = report.exit_code;

    if mode.is_json() {
        emit_json(mode, &report);
    } else if !mode.is_quiet() {
        emit_line(mode, format!("{synced} 个 ok / {failed} 个 fail"));
    }

    ExitCode::from(code)
}

// ── 4. status ───────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
pub struct StatusReport {
    projects: Vec<ProjectStatus>,
    summary: StatusSummary,
}

#[derive(Debug, Serialize, JsonSchema)]
struct ProjectStatus {
    project: String,
    assets: Vec<AssetStatus>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct AssetStatus {
    kind: String,
    name: String,
    source_path: String,
    platforms: Vec<PlatformStatus>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct PlatformStatus {
    platform: String,
    state: String,
    dest: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct StatusSummary {
    total_assets: usize,
    linked: usize,
    broken: usize,
    wrong_source: usize,
    wrong_type: usize,
    missing: usize,
}

/// `ai-config status` — 返回结构化报告（CLI / MCP 共用）。
pub fn status_report(default_root: &Utf8Path) -> Result<StatusReport, CoreError> {
    let ctx = load_context(default_root)?;
    let assets = flat_assets(&ctx.scan);
    let mut asset_statuses: Vec<AssetStatus> = Vec::new();
    let mut summary = StatusSummary {
        total_assets: assets.len(),
        linked: 0,
        broken: 0,
        wrong_source: 0,
        wrong_type: 0,
        missing: 0,
    };

    for (kind, name, src) in &assets {
        let kind_str = kind_to_str(*kind);
        let mut per_platform: Vec<PlatformStatus> = Vec::new();
        for plat in all_platforms() {
            let (state, dest) = describe_for(*kind, name, src, plat, default_root);
            match state.as_str() {
                "linked" => summary.linked += 1,
                "broken" => summary.broken += 1,
                "wrong_source" => summary.wrong_source += 1,
                "wrong_type" => summary.wrong_type += 1,
                "missing" => summary.missing += 1,
                _ => {}
            }
            per_platform.push(PlatformStatus {
                platform: platform_label(plat).to_string(),
                state,
                dest: dest.as_str().to_string(),
            });
        }
        asset_statuses.push(AssetStatus {
            kind: kind_str.to_string(),
            name: name.clone(),
            source_path: src.as_str().to_string(),
            platforms: per_platform,
        });
    }

    Ok(StatusReport {
        projects: vec![ProjectStatus {
            project: "default".to_string(),
            assets: asset_statuses,
        }],
        summary,
    })
}

/// `ai-config status`(PRD §6.1)
pub fn run_status(default_root: &Utf8Path, mode: OutputMode) -> ExitCode {
    let report = match status_report(default_root) {
        Ok(r) => r,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            return ExitCode::from(e.exit_code());
        }
    };

    if mode.is_json() {
        emit_json(mode, &report);
    } else if !mode.is_quiet() {
        emit_line(
            mode,
            format!(
                "项目: default | 平台: {} | 资产: {}",
                all_platforms().len(),
                report.summary.total_assets
            ),
        );
        emit_line(
            mode,
            format!(
                "  linked={} broken={} wrong_source={} wrong_type={} missing={}",
                report.summary.linked,
                report.summary.broken,
                report.summary.wrong_source,
                report.summary.wrong_type,
                report.summary.missing
            ),
        );
        for asset in &report.projects[0].assets {
            emit_line(mode, format!("[{}] {}", asset.kind, asset.name));
            for p in &asset.platforms {
                emit_line(
                    mode,
                    format!("    - {}: {} ({})", p.platform, p.state, p.dest),
                );
            }
        }
    }

    ExitCode::from(exit_code::SUCCESS)
}

fn describe_for(
    kind: AssetKind,
    name: &str,
    _src: &camino::Utf8Path,
    platform: PlatformId,
    default_root: &Utf8Path,
) -> (String, camino::Utf8PathBuf) {
    let adapter = match platform::for_id(platform) {
        Ok(a) => a,
        Err(_) => return ("unmanaged".to_string(), camino::Utf8PathBuf::new()),
    };
    let (dest, expected_src) = match kind {
        AssetKind::Skill => {
            let dest = adapter.skills_dir().join(name);
            let src = default_root.join("skills").join(name);
            (dest, src)
        }
        AssetKind::Rule => {
            let dest = adapter.rules_dir().join(format!("{name}.mdc"));
            let src = default_root.join("rules").join(format!("{name}.mdc"));
            (dest, src)
        }
        AssetKind::Command => {
            let dest = adapter.commands_dir().join(format!("{name}.md"));
            let src = default_root.join("commands").join(format!("{name}.md"));
            (dest, src)
        }
        AssetKind::Agent => {
            let dest = sync::asset_dest_for(platform, kind, name, _src)
                .unwrap_or_else(|| adapter.agents_dir().join(name));
            let expected_src = sync::agent_link_src(_src);
            (dest, expected_src)
        }
        AssetKind::Mcp => {
            let dest = adapter.mcp_deploy_path();
            let state = match mcp_json::mcp_json_file_sync_state(_src, &dest, platform) {
                McpSyncState::Linked => "linked",
                McpSyncState::Unlinked => "missing",
                McpSyncState::WrongValue => "wrong_source",
                McpSyncState::Broken => "broken",
            };
            return (state.to_string(), dest);
        }
        AssetKind::Hook => {
            let deploy_base = paths::global_deploy_base();
            let dest = hook_adapter::platform_scripts_dir(&deploy_base, platform, name);
            let state = if hook_adapter::is_deployed(&deploy_base, platform, name) {
                "linked"
            } else {
                "missing"
            };
            return (state.to_string(), dest);
        }
    };
    if !dest.exists() && dest.as_std_path().symlink_metadata().is_err() {
        return ("missing".to_string(), dest);
    }
    let health = materialize::check(&dest, &expected_src);
    let state = match health {
        materialize::DeployHealth::Linked { .. } => "linked",
        materialize::DeployHealth::Broken => "broken",
        materialize::DeployHealth::Unlinked => {
            // 兼容历史 symlink 状态展示
            match link::check(&dest, &expected_src) {
                LinkHealth::Linked { .. } => "linked",
                LinkHealth::Broken { .. } => "broken",
                LinkHealth::WrongSource { .. } => "wrong_source",
                LinkHealth::WrongType { .. } => "wrong_type",
            }
        }
    };
    (state.to_string(), dest)
}

// ── 5. list ─────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListReport {
    pub count: usize,
    pub assets: Vec<AssetEntry>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AssetEntry {
    pub kind: String,
    pub name: String,
    pub source_path: String,
}

/// `ai-config list` — 返回结构化报告（CLI / MCP 共用）。
pub fn list_report(default_root: &Utf8Path) -> Result<ListReport, CoreError> {
    let ctx = load_context(default_root)?;
    let assets = flat_assets(&ctx.scan);
    let entries: Vec<AssetEntry> = assets
        .iter()
        .map(|(k, n, p)| AssetEntry {
            kind: kind_to_str(*k).to_string(),
            name: n.clone(),
            source_path: p.as_str().to_string(),
        })
        .collect();
    Ok(ListReport {
        count: entries.len(),
        assets: entries,
    })
}

/// `ai-config list`
pub fn run_list(default_root: &Utf8Path, mode: OutputMode) -> ExitCode {
    let report = match list_report(default_root) {
        Ok(r) => r,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            return ExitCode::from(e.exit_code());
        }
    };

    if mode.is_json() {
        emit_json(mode, &report);
    } else if !mode.is_quiet() {
        // 按 kind 分组
        let mut grouped: std::collections::BTreeMap<&str, Vec<&AssetEntry>> =
            std::collections::BTreeMap::new();
        for a in &report.assets {
            grouped.entry(a.kind.as_str()).or_default().push(a);
        }
        emit_line(mode, format!("已纳管资产 ({} 条):", report.count));
        for (kind, items) in &grouped {
            emit_line(mode, format!("  [{kind}]"));
            for a in items {
                emit_line(mode, format!("    - {} ({})", a.name, a.source_path));
            }
        }
    }

    ExitCode::from(exit_code::SUCCESS)
}

// ── 6. show <name> ──────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct ShowReport {
    name: String,
    kind: String,
    source_path: String,
    platforms: Vec<PlatformStatus>,
}

/// `ai-config show <name>`
pub fn run_show(default_root: &Utf8Path, name: &str, mode: OutputMode) -> ExitCode {
    let ctx = match load_context(default_root) {
        Ok(c) => c,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            return ExitCode::from(e.exit_code());
        }
    };
    let assets = flat_assets(&ctx.scan);
    let matched: Vec<(AssetKind, String, camino::Utf8PathBuf)> =
        assets.into_iter().filter(|(_, n, _)| n == name).collect();

    if matched.is_empty() {
        emit_error_envelope(
            mode,
            exit_code::PARTIAL_FAILURE,
            &format!("资产 `{name}` 找不到"),
            Some("跑 `ai-config list` 查所有资产名;name 区分大小写"),
        );
        return ExitCode::from(exit_code::PARTIAL_FAILURE);
    }
    if matched.len() > 1 {
        emit_error_envelope(
            mode,
            exit_code::PARTIAL_FAILURE,
            &format!("name `{name}` 跨多类资产存在,需 kind 前缀"),
            Some("用 `ai-config skill show {name}` / `rule show` / `mcp show` / `agent show`"),
        );
        return ExitCode::from(exit_code::PARTIAL_FAILURE);
    }
    let (kind, _name, src) = matched.into_iter().next().unwrap();
    let kind_str = kind_to_str(kind);
    let mut platforms_out: Vec<PlatformStatus> = Vec::new();
    for plat in all_platforms() {
        let (state, dest) = describe_for(kind, name, &src, plat, default_root);
        platforms_out.push(PlatformStatus {
            platform: platform_label(plat).to_string(),
            state,
            dest: dest.as_str().to_string(),
        });
    }
    let report = ShowReport {
        name: name.to_string(),
        kind: kind_str.to_string(),
        source_path: src.as_str().to_string(),
        platforms: platforms_out,
    };

    if mode.is_json() {
        emit_json(mode, &report);
    } else if !mode.is_quiet() {
        emit_line(mode, format!("[{}] {}", report.kind, report.name));
        emit_line(mode, format!("  源: {}", report.source_path));
        emit_line(mode, "  平台状态:");
        for p in &report.platforms {
            emit_line(
                mode,
                format!("    - {}: {} ({})", p.platform, p.state, p.dest),
            );
        }
    }

    ExitCode::from(exit_code::SUCCESS)
}

// ── 7. doctor ───────────────────────────────────────────────────

/// `ai-config doctor`(PRD §6.2 / §10 A-10)
pub fn run_doctor(default_root: &Utf8Path, mode: OutputMode, materialize: bool) -> ExitCode {
    if materialize {
        let error = CoreError::InvalidPath(
            "doctor 是只读命令；--materialize 已禁用，请使用显式迁移计划".to_owned(),
        );
        emit_error_envelope(mode, error.exit_code(), &error.to_string(), error.hint());
        return ExitCode::from(error.exit_code());
    }

    let report = match ai_config_core::doctor::compute_report(default_root) {
        Ok(r) => r,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            return ExitCode::from(e.exit_code());
        }
    };

    if mode.is_json() {
        emit_json(mode, &report);
    } else if !mode.is_quiet() {
        emit_line(
            mode,
            format!(
                "doctor: broken={} wrong_source={} wrong_type={} missing_secrets={} unregistered_projects={} platform_capability_issues={}",
                report.broken,
                report.wrong_source,
                report.wrong_type,
                report.missing_secrets.len(),
                report.unregistered_projects.len(),
                report.platform_capability_issues.len()
            ),
        );
        if !report.missing_secrets.is_empty() {
            emit_line(mode, "  missing_secrets:");
            for m in &report.missing_secrets {
                emit_line(mode, format!("    - server `{}` 缺 `{}`", m.server, m.key));
            }
        }
        if !report.unregistered_projects.is_empty() {
            emit_line(mode, "  unregistered_projects:");
            for p in &report.unregistered_projects {
                emit_line(mode, format!("    - {p}"));
            }
        }
        if !report.platform_capability_issues.is_empty() {
            emit_line(mode, "  platform_capability_issues:");
            for c in &report.platform_capability_issues {
                emit_line(mode, format!("    - {}", c.reason));
            }
        }
    }

    ExitCode::from(report.exit_code)
}

// ── 工具 ───────────────────────────────────────────────────────

fn kind_to_str(k: AssetKind) -> &'static str {
    match k {
        AssetKind::Skill => "skill",
        AssetKind::Rule => "rule",
        AssetKind::Mcp => "mcp",
        AssetKind::Agent => "agent",
        AssetKind::Command => "command",
        AssetKind::Hook => "hook",
    }
}

fn flat_assets(scan: &source::ScanResult) -> Vec<(AssetKind, String, camino::Utf8PathBuf)> {
    let mut out: Vec<(AssetKind, String, camino::Utf8PathBuf)> = Vec::new();
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
    for item in &scan.hooks {
        out.push((
            AssetKind::Hook,
            item.script_filename.clone(),
            item.script_path.clone(),
        ));
    }
    out
}

fn ai_config_home() -> camino::Utf8PathBuf {
    if let Ok(h) = std::env::var("AI_CONFIG_HOME") {
        return camino::Utf8PathBuf::from(h);
    }
    if let Ok(h) = std::env::var("HOME") {
        return camino::Utf8PathBuf::from(h);
    }
    if let Ok(h) = std::env::var("USERPROFILE") {
        return camino::Utf8PathBuf::from(h);
    }
    camino::Utf8PathBuf::from(".")
}

// ── 单元测试(PRD §10 A-3:install → uninstall → install 幂等) ─────────

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;
    use std::fs;
    use std::path::Path;

    /// 建一个最小项目根(有 1 skill + 1 rule + 1 mcp server),返回 tempdir + 根路径。
    /// tempdir 在 caller 函数结束前都活着。
    fn make_project() -> (tempfile::TempDir, Utf8PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).expect("utf8 path");

        // skills/foo/SKILL.md
        fs::create_dir_all(root.join("skills/foo")).unwrap();
        fs::write(root.join("skills/foo/SKILL.md"), "# SKILL foo\n").unwrap();
        // rules/r1.mdc
        fs::create_dir_all(root.join("rules")).unwrap();
        fs::write(root.join("rules/r1.mdc"), "# RULE r1\n").unwrap();
        // mcp.json
        fs::write(
            root.join("mcp.json"),
            r#"{"mcpServers":{"echo":{"command":"echo","args":["hi"]}}}"#,
        )
        .unwrap();

        (tmp, root)
    }

    /// 在测试期间,把 HOME 重定向到 tempdir,避免污染真实 ~/.config/ai-config
    /// 与 ~/.cursor/... 等。`HomeGuard` 析构时恢复。
    static HOME_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct HomeGuard {
        prev: Option<String>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl HomeGuard {
        fn set_to(p: &Path) -> Self {
            let lock = HOME_TEST_LOCK.lock().expect("HOME test lock");
            let prev = std::env::var("HOME").ok();
            std::env::set_var("HOME", p);
            // 同时清掉 AI_CONFIG_HOME(防止旧 env 干扰)
            std::env::remove_var("AI_CONFIG_HOME");
            Self { prev, _lock: lock }
        }
    }
    impl Drop for HomeGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
    }

    // ── 共享测试:install → uninstall → install 幂等(PRD §10 A-3) ─

    /// 安全收回后，未带 marker 的 legacy copy 只能跳过；重复 install 仍须幂等。
    #[test]
    fn install_uninstall_install_is_idempotent() {
        let (root_tmp, root) = make_project();
        let home_tmp = tempfile::tempdir().expect("home tempdir");
        let _home = HomeGuard::set_to(home_tmp.path());

        // bin path:`assert_cmd::cargo_bin` 找本 crate 的 bin
        let assert = |args: &[&str]| -> assert_cmd::Command {
            let mut c = assert_cmd::Command::cargo_bin("ai-config").expect("cargo_bin ai-config");
            c.args(args);
            c.env("HOME", home_tmp.path());
            c
        };

        // 1st install
        let out = assert(&["--root", root.as_str(), "--quiet", "install"])
            .output()
            .expect("1st install");
        assert!(
            out.status.success(),
            "1st install should succeed; stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );

        // uninstall
        let out = assert(&["--root", root.as_str(), "--json", "uninstall"])
            .output()
            .expect("uninstall");
        assert!(
            out.status.success(),
            "uninstall should succeed; stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
        // T001 不再删除整份平台 MCP，未证明 ownership 的历史副本也不得删除。
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json out");
        assert_eq!(v["mcp_backups"].as_u64(), Some(0), "got {v:?}");
        assert!(
            home_tmp
                .path()
                .join(".cursor/skills/foo/SKILL.md")
                .is_file(),
            "没有 marker 的旧平台副本必须保留"
        );

        // 2nd install — 应当幂等(链接已撤回,从头开始)
        let out = assert(&["--root", root.as_str(), "--quiet", "install"])
            .output()
            .expect("2nd install");
        assert!(
            out.status.success(),
            "2nd install should also succeed (idempotent); stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );

        // 保留 root_tmp 防止 drop
        drop(root_tmp);
    }

    // ── 共享测试:--json 输出是合法 JSON ─────────────────────────

    #[test]
    fn list_json_output_is_valid_json_with_count() {
        let (root_tmp, root) = make_project();
        let home_tmp = tempfile::tempdir().expect("home tempdir");
        let _home = HomeGuard::set_to(home_tmp.path());

        let out = assert_cmd::Command::cargo_bin("ai-config")
            .expect("cargo_bin")
            .args(["--root", root.as_str(), "--json", "list"])
            .env("HOME", home_tmp.path())
            .output()
            .expect("list");
        assert!(out.status.success());

        let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json out");
        // list 至少 1 条 + count 字段
        assert!(v["count"].is_u64(), "list 应当有 count 字段: {v:?}");
        assert!(
            v["count"].as_u64().unwrap() >= 1,
            "list 至少 1 条(本测试 fixture 3 条:foo/r1/echo)"
        );
        assert!(v["assets"].is_array(), "list 应当有 assets 数组");

        drop(root_tmp);
    }

    // ── 共享测试:--quiet 不输出人类行 ───────────────────────────

    #[test]
    fn quiet_mode_silent_for_humans_but_exits_zero() {
        let (root_tmp, root) = make_project();
        let home_tmp = tempfile::tempdir().expect("home tempdir");
        let _home = HomeGuard::set_to(home_tmp.path());

        let out = assert_cmd::Command::cargo_bin("ai-config")
            .expect("cargo_bin")
            .args(["--root", root.as_str(), "--quiet", "list"])
            .env("HOME", home_tmp.path())
            .output()
            .expect("list quiet");
        assert!(out.status.success());
        // --quiet 时 stdout 应当是空(没人话)
        assert!(
            out.stdout.is_empty(),
            "--quiet 模式 list 应只输 stderr;stdout = {}",
            String::from_utf8_lossy(&out.stdout)
        );

        drop(root_tmp);
    }

    // ── 共享测试:doctor JSON 至少含核心字段 ──────────────────────

    #[test]
    fn doctor_json_contains_required_fields() {
        let (root_tmp, root) = make_project();
        let home_tmp = tempfile::tempdir().expect("home tempdir");
        let _home = HomeGuard::set_to(home_tmp.path());

        let out = assert_cmd::Command::cargo_bin("ai-config")
            .expect("cargo_bin")
            .args(["--root", root.as_str(), "--json", "doctor"])
            .env("HOME", home_tmp.path())
            .output()
            .expect("doctor");
        // doctor 是诊断输出,即便发现问题也退出 0(已成功报告,非部分失败)
        assert!(
            out.status.success(),
            "doctor stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json out");
        for key in [
            "broken",
            "wrong_source",
            "wrong_type",
            "missing_secrets",
            "unregistered_projects",
            "platform_capability_issues",
        ] {
            assert!(v.get(key).is_some(), "doctor JSON 缺 {key}: {v:?}");
        }
        // broken / wrong_* 是数字;missing_secrets / unregistered_projects / platform_capability_issues 是数组
        assert!(v["broken"].is_u64());
        assert!(v["missing_secrets"].is_array());
        assert!(v["platform_capability_issues"].is_array());

        drop(root_tmp);
    }

    #[test]
    fn list_report_matches_fixture_assets() {
        let (root_tmp, root) = make_project();
        let report = super::list_report(&root).expect("list_report");
        assert_eq!(report.count, 3);
        let kinds: Vec<_> = report.assets.iter().map(|a| a.kind.as_str()).collect();
        assert!(kinds.contains(&"skill"));
        assert!(kinds.contains(&"rule"));
        assert!(kinds.contains(&"mcp"));
        drop(root_tmp);
    }

    #[test]
    fn status_report_includes_per_platform_states() {
        let (root_tmp, root) = make_project();
        let home_tmp = tempfile::tempdir().expect("home");
        let _home = HomeGuard::set_to(home_tmp.path());

        let report = super::status_report(&root).expect("status_report");
        assert_eq!(report.summary.total_assets, 3);
        let mcp = report.projects[0]
            .assets
            .iter()
            .find(|a| a.kind == "mcp")
            .expect("mcp asset");
        assert_eq!(mcp.platforms.len(), 4);
        drop(root_tmp);
    }

    #[test]
    fn sync_report_produces_outcomes() {
        let (root_tmp, root) = make_project();
        let home_tmp = tempfile::tempdir().expect("home");
        let _home = HomeGuard::set_to(home_tmp.path());

        let report = super::sync_report(&root, false).expect("sync_report");
        assert!(!report.outcomes.is_empty());
        assert!(report.synced > 0 || report.failed > 0);
        drop(root_tmp);
    }

    #[test]
    fn workspace_install_discovers_parent_and_submodule() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let ws = camino::Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let home_tmp = tempfile::tempdir().expect("home");
        let _home = HomeGuard::set_to(home_tmp.path());

        std::fs::create_dir_all(ws.join(".ai-config/skills/foo")).unwrap();
        std::fs::write(ws.join(".ai-config/skills/foo/SKILL.md"), "SKILL").unwrap();
        std::fs::create_dir_all(ws.join("child")).unwrap();
        std::fs::write(
            ws.join(".gitmodules"),
            "[submodule \"child\"]\n\tpath = child\n",
        )
        .unwrap();

        let members = ai_config_core::workspace::discover_members(&ws).unwrap();
        assert_eq!(members.len(), 2);

        let report = super::sync_report(&ws, true).expect("workspace sync");
        assert!(!report.outcomes.is_empty());
    }

    // ── 共享测试:show 不存在的 name 退出码 3(部分失败) ────────

    #[test]
    fn show_missing_name_exits_3() {
        let (root_tmp, root) = make_project();
        let home_tmp = tempfile::tempdir().expect("home tempdir");
        let _home = HomeGuard::set_to(home_tmp.path());

        let out = assert_cmd::Command::cargo_bin("ai-config")
            .expect("cargo_bin")
            .args(["--root", root.as_str(), "--json", "show", "does-not-exist"])
            .env("HOME", home_tmp.path())
            .output()
            .expect("show");
        assert_eq!(
            out.status.code(),
            Some(ai_config_core::error::exit_code::PARTIAL_FAILURE as i32),
            "show 不存在的 name 应退出码 3(部分失败)"
        );

        drop(root_tmp);
    }

    // ── 共享测试:install 二次 install 退出码 0(幂等) ───────────

    #[test]
    fn install_twice_exits_zero_each_time() {
        let (root_tmp, root) = make_project();
        let home_tmp = tempfile::tempdir().expect("home tempdir");
        let _home = HomeGuard::set_to(home_tmp.path());

        for nth in 1..=2 {
            let out = assert_cmd::Command::cargo_bin("ai-config")
                .expect("cargo_bin")
                .args(["--root", root.as_str(), "--quiet", "install"])
                .env("HOME", home_tmp.path())
                .output()
                .expect("install");
            assert!(
                out.status.success(),
                "{nth}th install should succeed; stderr={}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        drop(root_tmp);
    }
}
