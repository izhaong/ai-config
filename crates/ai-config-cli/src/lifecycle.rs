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

use std::collections::HashMap;
use std::process::ExitCode;

use camino::Utf8Path;
use serde::Serialize;

use ai_config_core::error::{exit_code, CoreError};
use ai_config_core::link::{self, LinkHealth, LinkKind};
use ai_config_core::mcp_json;
use ai_config_core::model::{AssetKind, PlatformId, SyncAction};
use ai_config_core::platform;
use ai_config_core::secrets as core_secrets;
use ai_config_core::source;
use ai_config_core::sync;
use ai_config_core::template::McpSyncState;

use crate::output::{emit_error_envelope, emit_json, emit_line, OutputMode};

// ── 共享:扫描 + 装载 ──────────────────────────────────────────────

/// 一次同步的"工作上下文":扫 default_root + 装 secrets + 列所有平台适配器。
struct SyncContext {
    scan: source::ScanResult,
    /// secrets 已加载(key→value)。**绝不**回显 value 到日志 / stdout / JSON。
    secrets_pairs: Vec<(String, String)>,
    secrets_map: HashMap<String, String>,
    actions: Vec<SyncAction>,
}

fn load_context(default_root: &Utf8Path) -> Result<SyncContext, CoreError> {
    let scan = source::scan_project_root(default_root)?;
    let project = ai_config_core::model::Project::new("default", default_root.to_path_buf());
    let actions = sync::compute_for_project(&project, default_root)?;
    let pairs = core_secrets::load()?;
    let map = pairs_to_map(&pairs);
    Ok(SyncContext {
        scan,
        secrets_pairs: pairs,
        secrets_map: map,
        actions,
    })
}

fn pairs_to_map(pairs: &[(String, String)]) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for (k, v) in pairs {
        m.insert(k.clone(), v.clone());
    }
    m
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
        PlatformId::Cursor => "cursor",
        PlatformId::Codex => "codex",
        PlatformId::Claude => "claude",
        PlatformId::Hermes => "hermes",
    }
}

// ── 共享:执行 SyncAction ──────────────────────────────────────────

/// 一条动作的执行结果。
#[derive(Debug, Clone, Serialize)]
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

/// 反推 src 路径:dest 在 `<home>/.cursor/skills/foo` 之类,src 在 `<root>/skills/foo/SKILL.md`。
fn infer_src_for_create(
    dest: &camino::Utf8Path,
    platform: PlatformId,
    kind: &str,
    default_root: &Utf8Path,
) -> camino::Utf8PathBuf {
    let adapter = match platform::for_id(platform) {
        Ok(a) => a,
        Err(_) => return dest.to_path_buf(),
    };
    let dir = match kind {
        "skill" => adapter.skills_dir(),
        "rule" => adapter.rules_dir(),
        "agent" => adapter.agents_dir(),
        _ => return dest.to_path_buf(),
    };
    if let Ok(rel) = dest.strip_prefix(dir.as_path()) {
        return default_root
            .join(match kind {
                "skill" => "skills",
                "rule" => "rules",
                "agent" => "agents",
                _ => "",
            })
            .join(rel);
    }
    dest.to_path_buf()
}

fn infer_kind_from_dest(dest: &camino::Utf8Path) -> &'static str {
    let s = dest.as_str();
    if s.contains("/skills/") || s.ends_with("/skills") {
        "skill"
    } else if s.contains("/rules/") {
        "rule"
    } else if s.contains("/agents/") || s.contains("/subagents/") {
        "agent"
    } else {
        "unknown"
    }
}

/// 跑一次完整 sync(per-item × per-platform 动作展开),返回 outcomes。
fn execute_all_actions(ctx: &SyncContext, default_root: &Utf8Path) -> Vec<Outcome> {
    let mut out: Vec<Outcome> = Vec::new();
    let mut mcp_renders_done = std::collections::HashSet::new();

    for action in &ctx.actions {
        match action {
            SyncAction::Create {
                platform,
                dest,
                item_id: _,
            } => {
                let kind = infer_kind_from_dest(dest);
                let src = infer_src_for_create(dest, *platform, kind, default_root);
                let label = format!("Create {} → {}", kind, dest);
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
                match link::link(&src, dest, LinkKind::auto()) {
                    Ok(()) => out.push(Outcome::ok(label, *platform, kind)),
                    Err(e) => out.push(Outcome::failed(label, *platform, kind, &e)),
                }
            }
            SyncAction::RenderMcp { platform, .. } => {
                if mcp_renders_done.insert(*platform) {
                    let Some(ref src) = ctx.scan.mcp_json else {
                        continue;
                    };
                    let label = format!("RenderMcp {}", platform_label(*platform));
                    let adapter = match platform::for_id(*platform) {
                        Ok(a) => a,
                        Err(e) => {
                            out.push(Outcome::failed(label, *platform, "mcp", &e));
                            continue;
                        }
                    };
                    let dest = adapter.mcp_json_path();
                    match mcp_json::deploy_mcp_json_file(src, &dest) {
                        Ok(_) => out.push(Outcome::ok(label, *platform, "mcp")),
                        Err(e) => out.push(Outcome::failed(label, *platform, "mcp", &e)),
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
pub fn run_install(default_root: &Utf8Path, mode: OutputMode) -> ExitCode {
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
    let outcomes = execute_all_actions(&ctx, default_root);
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
            match link::unlink(dest) {
                Ok(()) => outcomes.push(Outcome::ok(label, *platform, kind)),
                Err(e) => {
                    // 目标不是链接(可能用户手写):**不**擅改,记为 skipped
                    let s = e.to_string();
                    if s.contains("不是链接") {
                        outcomes.push(Outcome::skipped(label, *platform, kind, &s));
                    } else {
                        outcomes.push(Outcome::failed(label, *platform, kind, &e));
                    }
                }
            }
        }
    }

    // 2. 收回各平台 mcp.json(整文件删除)
    for plat in all_platforms() {
        let label = format!("RetractMcp {}", platform_label(plat));
        let adapter = match platform::for_id(plat) {
            Ok(a) => a,
            Err(e) => {
                outcomes.push(Outcome::failed(label, plat, "mcp", &e));
                continue;
            }
        };
        let mcp_path = adapter.mcp_json_path();
        if !mcp_path.exists() {
            outcomes.push(Outcome::skipped(
                label,
                plat,
                "mcp",
                "目标平台没有 mcp.json,无需收回",
            ));
            continue;
        }
        match mcp_json::retract_platform_mcp_json(&mcp_path) {
            Ok(()) => outcomes.push(Outcome::ok(label, plat, "mcp")),
            Err(e) => outcomes.push(Outcome::failed(label, plat, "mcp", &e)),
        }
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

#[derive(Debug, Serialize)]
struct SyncReport {
    ok: bool,
    synced: usize,
    failed: usize,
    outcomes: Vec<Outcome>,
    exit_code: u8,
}

/// `ai-config sync`(PRD §10 A-5)
pub fn run_sync(default_root: &Utf8Path, mode: OutputMode) -> ExitCode {
    let ctx = match load_context(default_root) {
        Ok(c) => c,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            return ExitCode::from(e.exit_code());
        }
    };
    let outcomes = execute_all_actions(&ctx, default_root);
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

    let report = SyncReport {
        ok: code == exit_code::SUCCESS,
        synced,
        failed,
        outcomes,
        exit_code: code,
    };

    if mode.is_json() {
        emit_json(mode, &report);
    } else if !mode.is_quiet() {
        emit_line(mode, format!("{synced} 个 ok / {failed} 个 fail"));
    }

    ExitCode::from(code)
}

// ── 4. status ───────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct StatusReport {
    projects: Vec<ProjectStatus>,
    summary: StatusSummary,
}

#[derive(Debug, Serialize)]
struct ProjectStatus {
    project: String,
    assets: Vec<AssetStatus>,
}

#[derive(Debug, Serialize)]
struct AssetStatus {
    kind: String,
    name: String,
    source_path: String,
    platforms: Vec<PlatformStatus>,
}

#[derive(Debug, Serialize)]
struct PlatformStatus {
    platform: String,
    state: String,
    dest: String,
}

#[derive(Debug, Serialize)]
struct StatusSummary {
    total_assets: usize,
    linked: usize,
    broken: usize,
    wrong_source: usize,
    wrong_type: usize,
    missing: usize,
}

/// `ai-config status`(PRD §6.1)
pub fn run_status(default_root: &Utf8Path, mode: OutputMode) -> ExitCode {
    let ctx = match load_context(default_root) {
        Ok(c) => c,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            return ExitCode::from(e.exit_code());
        }
    };

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

    let report = StatusReport {
        projects: vec![ProjectStatus {
            project: "default".to_string(),
            assets: asset_statuses,
        }],
        summary,
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
        AssetKind::Agent => {
            let dest = sync::asset_dest_for(platform, kind, name, _src)
                .unwrap_or_else(|| adapter.agents_dir().join(name));
            let expected_src = sync::agent_link_src(_src);
            (dest, expected_src)
        }
        AssetKind::Mcp => {
            let state = match mcp_json::mcp_json_file_sync_state(_src, &adapter.mcp_json_path()) {
                McpSyncState::Linked => "linked",
                McpSyncState::Unlinked => "missing",
                McpSyncState::WrongValue => "wrong_source",
                McpSyncState::Broken => "broken",
            };
            return (state.to_string(), adapter.mcp_json_path());
        }
    };
    if !dest.exists() && dest.as_std_path().symlink_metadata().is_err() {
        return ("missing".to_string(), dest);
    }
    let health = link::check(&dest, &expected_src);
    let state = match health {
        LinkHealth::Linked { .. } => "linked",
        LinkHealth::Broken { .. } => "broken",
        LinkHealth::WrongSource { .. } => "wrong_source",
        LinkHealth::WrongType { .. } => "wrong_type",
    };
    (state.to_string(), dest)
}

// ── 5. list ─────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct ListReport {
    count: usize,
    assets: Vec<AssetEntry>,
}

#[derive(Debug, Serialize)]
struct AssetEntry {
    kind: String,
    name: String,
    source_path: String,
}

/// `ai-config list`
pub fn run_list(default_root: &Utf8Path, mode: OutputMode) -> ExitCode {
    let ctx = match load_context(default_root) {
        Ok(c) => c,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            return ExitCode::from(e.exit_code());
        }
    };
    let assets = flat_assets(&ctx.scan);
    let entries: Vec<AssetEntry> = assets
        .iter()
        .map(|(k, n, p)| AssetEntry {
            kind: kind_to_str(*k).to_string(),
            name: n.clone(),
            source_path: p.as_str().to_string(),
        })
        .collect();

    let report = ListReport {
        count: entries.len(),
        assets: entries,
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

#[derive(Debug, Serialize)]
struct DoctorReport {
    broken: usize,
    wrong_source: usize,
    wrong_type: usize,
    missing_secrets: Vec<MissingSecret>,
    unregistered_projects: Vec<String>,
    platform_capability_issues: Vec<PlatformCapabilityIssue>,
    exit_code: u8,
}

#[derive(Debug, Serialize)]
struct PlatformCapabilityIssue {
    platform: String,
    kind: String,
    reason: String,
}

/// `ai-config doctor`(PRD §6.2 / §10 A-10)
pub fn run_doctor(default_root: &Utf8Path, mode: OutputMode) -> ExitCode {
    let ctx = match load_context(default_root) {
        Ok(c) => c,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            return ExitCode::from(e.exit_code());
        }
    };

    let mut report = DoctorReport {
        broken: 0,
        wrong_source: 0,
        wrong_type: 0,
        missing_secrets: Vec::new(),
        unregistered_projects: Vec::new(),
        platform_capability_issues: Vec::new(),
        exit_code: exit_code::SUCCESS,
    };

    // 1. 链接健康
    let assets = flat_assets(&ctx.scan);
    for (kind, name, src) in &assets {
        for plat in all_platforms() {
            let (state, _dest) = describe_for(*kind, name, src, plat, default_root);
            match state.as_str() {
                "broken" => report.broken += 1,
                "wrong_source" => report.wrong_source += 1,
                "wrong_type" => report.wrong_type += 1,
                _ => {}
            }
        }
    }

    // 2. secrets(MCP 明文模式,不再扫描缺 key)
    let _ = &ctx.secrets_map;

    // 3. 项目未注册(Phase 1:若 root 没有任何资产,记为 "无项目")
    if assets.is_empty() {
        report
            .unregistered_projects
            .push(format!("{default_root} (no assets found)"));
    }

    // 4. 平台能力(Codex 不支持 rule / agent)
    for plat in all_platforms() {
        for k in [
            AssetKind::Skill,
            AssetKind::Rule,
            AssetKind::Mcp,
            AssetKind::Agent,
        ] {
            let supports = platform::for_id(plat)
                .map(|a| a.supports(k))
                .unwrap_or(false);
            if !supports {
                report
                    .platform_capability_issues
                    .push(PlatformCapabilityIssue {
                        platform: platform_label(plat).to_string(),
                        kind: kind_to_str(k).to_string(),
                        reason: format!(
                            "platform `{}` 不支持 asset kind `{}`",
                            platform_label(plat),
                            kind_to_str(k)
                        ),
                    });
            }
        }
    }

    let _any_issue = report.broken > 0
        || report.wrong_source > 0
        || report.wrong_type > 0
        || !report.missing_secrets.is_empty()
        || !report.unregistered_projects.is_empty()
        || !report.platform_capability_issues.is_empty();
    // doctor 是诊断输出,即便发现问题也退出 0(已成功报告,非部分失败)
    // agent 拿 --json 解析后用字段值判断健康;退出码 0 表示"诊断成功完成"
    report.exit_code = exit_code::SUCCESS;

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
    struct HomeGuard {
        prev: Option<String>,
    }
    impl HomeGuard {
        fn set_to(p: &Path) -> Self {
            let prev = std::env::var("HOME").ok();
            std::env::set_var("HOME", p);
            // 同时清掉 AI_CONFIG_HOME(防止旧 env 干扰)
            std::env::remove_var("AI_CONFIG_HOME");
            Self { prev }
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

    /// PRD §10 A-3:跑 N 次 == 跑 1 次。
    /// 这里验证 install → uninstall → install 三步都成功(exit 0),且第 2 次 install
    /// 不会因为已存在的旧链接而崩溃。
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
        // 确认 mcp.json 至少被备份了 1 次(看 stdout JSON 里的 mcp_backups)
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json out");
        assert!(
            v["mcp_backups"].as_u64().unwrap_or(0) > 0,
            "uninstall 应至少备份 1 份 mcp.json(本工具产物);got {v:?}"
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
