//! 只读 lifecycle 报告：`status` / `list` / `show` / `doctor`。
//!
//! install / uninstall / sync 统一由 projection plan/apply 执行；此模块不得直接写平台目标。
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

use agents_manager_core::error::{exit_code, CoreError};
use agents_manager_core::hook_adapter;
use agents_manager_core::link::{self, LinkHealth};
use agents_manager_core::materialize;
use agents_manager_core::mcp_json;
use agents_manager_core::model::{AssetKind, PlatformId};
use agents_manager_core::paths;
use agents_manager_core::platform;
use agents_manager_core::source;
use agents_manager_core::sync;
use agents_manager_core::template::McpSyncState;

use crate::output::{emit_error_envelope, emit_json, emit_line, OutputMode};

// ── 共享:只读扫描 ────────────────────────────────────────────────

struct ReadContext {
    scan: source::ScanResult,
}

fn load_context(default_root: &Utf8Path) -> Result<ReadContext, CoreError> {
    let roots = paths::resolve_sync_roots(default_root);
    let scan = source::scan_with_override(&roots.asset_root, &roots.global_default)?;
    Ok(ReadContext { scan })
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
        PlatformId::AgentsManager => "agentsmanager",
        PlatformId::Cursor => "cursor",
        PlatformId::Codex => "codex",
        PlatformId::Claude => "claude",
        PlatformId::Hermes => "hermes",
    }
}

// ── status ──────────────────────────────────────────────────────

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

/// `agents-manager status` — 返回结构化报告（CLI / MCP 共用）。
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

/// `agents-manager status`(PRD §6.1)
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
    if kind == AssetKind::Prompt {
        return ("unsupported".to_string(), camino::Utf8PathBuf::new());
    }
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
        AssetKind::Prompt => unreachable!("Prompt is rejected before path resolution"),
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

/// `agents-manager list` — 返回结构化报告（CLI / MCP 共用）。
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

/// `agents-manager list`
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

/// `agents-manager show <name>`
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
            Some("跑 `agents-manager list` 查所有资产名;name 区分大小写"),
        );
        return ExitCode::from(exit_code::PARTIAL_FAILURE);
    }
    if matched.len() > 1 {
        emit_error_envelope(
            mode,
            exit_code::PARTIAL_FAILURE,
            &format!("name `{name}` 跨多类资产存在,需 kind 前缀"),
            Some("用 `agents-manager skill show {name}` / `rule show` / `mcp show` / `agent show`"),
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

/// `agents-manager doctor`(PRD §6.2 / §10 A-10)
pub fn run_doctor(default_root: &Utf8Path, mode: OutputMode, materialize: bool) -> ExitCode {
    if materialize {
        let error = CoreError::InvalidPath(
            "doctor 是只读命令；--materialize 已禁用，请使用显式迁移计划".to_owned(),
        );
        emit_error_envelope(mode, error.exit_code(), &error.to_string(), error.hint());
        return ExitCode::from(error.exit_code());
    }

    let report = match agents_manager_core::doctor::compute_report(default_root) {
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
        AssetKind::Prompt => "prompt",
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

// ── 单元测试 ────────────────────────────────────────────────────

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

    /// 在测试期间,把 HOME 重定向到 tempdir,避免污染真实 ~/.config/agents-manager
    /// 与 ~/.cursor/... 等。`HomeGuard` 析构时恢复。
    static HOME_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct HomeGuard {
        prev: Option<String>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl HomeGuard {
        fn set_to(p: &Path) -> Self {
            // A prior assertion can unwind after its guard restored HOME. Keep later tests
            // diagnostic instead of hiding their own contract failures behind lock poisoning.
            let lock = HOME_TEST_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let prev = std::env::var("HOME").ok();
            std::env::set_var("HOME", p);
            // 同时清掉 AGENTS_MANAGER_HOME(防止旧 env 干扰)
            std::env::remove_var("AGENTS_MANAGER_HOME");
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

    /// Install / uninstall 默认只返回投影计划；不得沿用旧 lifecycle 写入或报告字段。
    #[test]
    fn install_and_uninstall_default_to_plan_only_without_targets() {
        let (root_tmp, root) = make_project();
        let home_tmp = tempfile::tempdir().expect("home tempdir");
        let _home = HomeGuard::set_to(home_tmp.path());

        // bin path:`assert_cmd::cargo_bin` 找本 crate 的 bin
        let assert = |args: &[&str]| -> assert_cmd::Command {
            let mut c = assert_cmd::Command::cargo_bin("agents-manager").expect("cargo_bin agents-manager");
            c.args(args);
            c.env("HOME", home_tmp.path());
            c
        };

        for (command, label) in [("install", "install"), ("uninstall", "uninstall")] {
            let out = assert(&["--root", root.as_str(), "--json", command])
                .output()
                .expect(label);
            assert!(
                out.status.success(),
                "{label} plan should succeed; stderr={}",
                String::from_utf8_lossy(&out.stderr)
            );
            let report: serde_json::Value =
                serde_json::from_slice(&out.stdout).expect("projection lifecycle JSON");
            assert!(report["plan"]["schema_version"].is_u64(), "got {report:?}");
            assert!(report["plan"]["plan_digest"].is_string(), "got {report:?}");
            assert!(
                report.get("apply").is_none(),
                "{label} without --apply must never execute a plan: {report:?}"
            );
        }
        for target in [
            home_tmp.path().join(".agents/skills/foo"),
            home_tmp.path().join(".cursor/mcp.json"),
            home_tmp.path().join(".claude/skills/foo"),
        ] {
            assert!(
                !target.exists(),
                "default lifecycle command must not create {}",
                target.display()
            );
        }

        // 保留 root_tmp 防止 drop
        drop(root_tmp);
    }

    // ── 共享测试:--json 输出是合法 JSON ─────────────────────────

    #[test]
    fn list_json_output_is_valid_json_with_count() {
        let (root_tmp, root) = make_project();
        let home_tmp = tempfile::tempdir().expect("home tempdir");
        let _home = HomeGuard::set_to(home_tmp.path());

        let out = assert_cmd::Command::cargo_bin("agents-manager")
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

        let out = assert_cmd::Command::cargo_bin("agents-manager")
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

        let out = assert_cmd::Command::cargo_bin("agents-manager")
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
    fn projection_sync_defaults_to_plan_only_report() {
        let (root_tmp, root) = make_project();
        let home_tmp = tempfile::tempdir().expect("home");
        let _home = HomeGuard::set_to(home_tmp.path());

        let execution =
            crate::projection::execute(&root, false, false, false).expect("source-first sync plan");
        assert_eq!(
            execution.exit_code,
            agents_manager_core::error::exit_code::SUCCESS
        );
        assert!(execution.report.apply.is_none());
        assert!(!execution.report.plan.actions.is_empty());
        assert!(
            !home_tmp.path().join(".agents/skills/foo").exists(),
            "default projection sync must remain zero-write"
        );
        drop(root_tmp);
    }

    #[test]
    fn workspace_install_discovers_parent_and_submodule() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let ws = camino::Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let home_tmp = tempfile::tempdir().expect("home");
        let _home = HomeGuard::set_to(home_tmp.path());

        std::fs::create_dir_all(ws.join(".agents/skills/foo")).unwrap();
        std::fs::write(ws.join(".agents/skills/foo/SKILL.md"), "SKILL").unwrap();
        std::fs::create_dir_all(ws.join("child")).unwrap();
        std::fs::write(
            ws.join(".gitmodules"),
            "[submodule \"child\"]\n\tpath = child\n",
        )
        .unwrap();

        let members = agents_manager_core::workspace::discover_members(&ws).unwrap();
        assert_eq!(members.len(), 2);

        let result = crate::projection::execute(&ws, true, false, false);
        let error = match result {
            Ok(_) => panic!("workspace projection must fail closed until its adapter exists"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            agents_manager_core::error::CoreError::NotImplemented(_)
        ));
        assert_eq!(
            error.exit_code(),
            agents_manager_core::error::exit_code::ARG_ERROR
        );
        assert!(
            !home_tmp.path().join(".agents/skills/foo").exists(),
            "unsupported workspace mode must not write platform targets"
        );
    }

    // ── 共享测试:show 不存在的 name 退出码 3(部分失败) ────────

    #[test]
    fn show_missing_name_exits_3() {
        let (root_tmp, root) = make_project();
        let home_tmp = tempfile::tempdir().expect("home tempdir");
        let _home = HomeGuard::set_to(home_tmp.path());

        let out = assert_cmd::Command::cargo_bin("agents-manager")
            .expect("cargo_bin")
            .args(["--root", root.as_str(), "--json", "show", "does-not-exist"])
            .env("HOME", home_tmp.path())
            .output()
            .expect("show");
        assert_eq!(
            out.status.code(),
            Some(agents_manager_core::error::exit_code::PARTIAL_FAILURE as i32),
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
            let out = assert_cmd::Command::cargo_bin("agents-manager")
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
