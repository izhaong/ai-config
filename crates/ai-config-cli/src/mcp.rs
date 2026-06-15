//! `ai-config mcp ...` 子命令(PRD §4.2 / §5.1 / §10 A-7 / A-8 / A-15)。
//!
//! 关键约束:
//! - **per-item × per-platform**(§4.2):每条 server × 4 平台 — `add` / `remove` /
//!   `enable` / `disable` 影响 `mcp/servers/<name>.json`;`deploy <name> <to>` /
//!   `retract <name> <from>` 只渲染/移除**该** server 到/从**该**平台 mcp.json
//!   (不触其它条目,保留用户手写)。
//! - **A-7** 加 server → 4 份 mcp.json 一致(在 4 平台都 deploy 的话)
//! - **A-8** 删 server → 4 份 mcp.json 不残留(retract 全部 → 4 份都清干净)
//! - **A-15** 单条 deploy/retract 不影响其它条目:这是本模块最核心的不变量,见
//!   `deploy_one_internal` / `retract_one_internal` 的实现。

use std::io::{self, IsTerminal, Read, Write};
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;
use serde_json::{Map, Value};

use ai_config_core::error::{exit_code, CoreError};
use ai_config_core::hermes_config::HermesMigrateReport;
use ai_config_core::mcp_json;
use ai_config_core::model::{McpServer, McpTransport, PlatformId};
use ai_config_core::paths;
use ai_config_core::platform;
use ai_config_core::source;

use crate::output::{emit_error_envelope, emit_json, emit_line, OutputMode};

#[derive(Debug, Clone)]
pub enum McpCmd {
    List,
    Show {
        name: String,
    },
    Add,
    Remove {
        name: String,
    },
    Enable {
        name: String,
    },
    Disable {
        name: String,
    },
    /// 单条 × 单平台下发(PRD §4.2 + §10 A-15)
    Deploy {
        name: String,
        to: String,
    },
    /// 单条 × 单平台收回(PRD §4.2 + §10 A-15)
    Retract {
        name: String,
        from: String,
    },
    /// 整份 mcp.json 模板 → 逐项 `mcp/servers/<name>.json`(W5 残留,半天里程碑)
    ///
    /// `source` 默认 `mcp/cursor.mcp.template.json`(相对 `default_root`)。
    /// 默认 **skip 已存在**(不覆盖,避免丢用户手动加的 `enabled: false` 等);
    /// `--dry-run` 跑完不写盘,只输出报告。
    ///
    /// 注:`dry_run` 字段名是 `snake_case`(外层 clap 在 `main.rs` 那边解析,
    /// 本枚举是内部执行态,字段名是 rust 习惯 snake_case)。
    Migrate {
        source: Option<String>,
        dry_run: bool,
    },
    /// 遗留 `~/.hermes/mcp.json` → `~/.hermes/config.yaml` 的 `mcp_servers`
    MigrateHermes {
        dry_run: bool,
    },
}

impl McpCmd {
    pub fn run(self, mode: OutputMode, default_root: &Utf8Path) -> ExitCode {
        match self {
            McpCmd::List => run_list(mode, default_root),
            McpCmd::Show { name } => run_show(mode, default_root, &name),
            McpCmd::Add => run_add(mode, default_root),
            McpCmd::Remove { name } => run_remove(mode, default_root, &name),
            McpCmd::Enable { name } => run_set_enabled(mode, default_root, &name, true),
            McpCmd::Disable { name } => run_set_enabled(mode, default_root, &name, false),
            McpCmd::Deploy { name, to } => match parse_platform(&to) {
                Ok(plat) => run_deploy(mode, default_root, &name, plat),
                Err(e) => {
                    emit_error_envelope(mode, exit_code::ARG_ERROR, &e.to_string(), None);
                    ExitCode::from(exit_code::ARG_ERROR)
                }
            },
            McpCmd::Retract { name, from } => match parse_platform(&from) {
                Ok(plat) => run_retract(mode, default_root, &name, plat),
                Err(e) => {
                    emit_error_envelope(mode, exit_code::ARG_ERROR, &e.to_string(), None);
                    ExitCode::from(exit_code::ARG_ERROR)
                }
            },
            McpCmd::Migrate { source, dry_run } => {
                run_migrate(mode, default_root, source.as_deref(), dry_run)
            }
            McpCmd::MigrateHermes { dry_run } => run_migrate_hermes(mode, dry_run),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct McpListEntry {
    pub name: String,
    pub transport: String,
    pub enabled: bool,
    pub source_path: String,
    pub secret_keys: Vec<String>,
}

// ── list / show ─────────────────────────────────────────────────

fn run_list(mode: OutputMode, root: &Utf8Path) -> ExitCode {
    let servers = match load_all_servers(root) {
        Ok(s) => s,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code_kind(), &e.to_string(), e.hint_text());
            return ExitCode::from(e.exit_code_kind());
        }
    };
    if mode.is_json() {
        emit_json(mode, &serde_json::json!({ "items": servers }));
    } else {
        for s in &servers {
            let flag = if s.enabled { "on " } else { "off" };
            println!("{flag}\t{}\t{}\t{}", s.name, s.transport, s.source_path);
        }
    }
    ExitCode::SUCCESS
}

fn run_show(mode: OutputMode, root: &Utf8Path, name: &str) -> ExitCode {
    let found = match find_server(root, name) {
        Ok(v) => v,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code_kind(), &e.to_string(), e.hint_text());
            return ExitCode::from(e.exit_code_kind());
        }
    };
    let Some((srv, path)) = found else {
        let msg = format!("mcp server `{name}` 找不到");
        let hint = "跑 `ai-config mcp list` 看全部";
        emit_error_envelope(mode, exit_code::PARTIAL_FAILURE, &msg, Some(hint));
        return ExitCode::from(exit_code::PARTIAL_FAILURE);
    };
    if mode.is_json() {
        emit_json(
            mode,
            &serde_json::json!({
                "name": srv.name,
                "transport": srv.transport,
                "enabled": srv.enabled,
                "source_path": path,
                "secret_keys": srv.secret_keys,
                "config": srv.config,
            }),
        );
    } else {
        println!("name: {}", srv.name);
        println!("transport: {:?}", srv.transport);
        println!("enabled: {}", srv.enabled);
        println!("source: {}", path);
        println!("secret_keys: {:?}", srv.secret_keys);
        println!(
            "config: {}",
            serde_json::to_string_pretty(&srv.config).unwrap_or_default()
        );
    }
    ExitCode::SUCCESS
}

// ── add / remove / enable / disable ─────────────────────────────

fn run_add(mode: OutputMode, root: &Utf8Path) -> ExitCode {
    let stdin = io::stdin();
    let parsed = if stdin.is_terminal() {
        match add_interactive() {
            Ok(p) => p,
            Err(e) => {
                emit_error_envelope(mode, exit_code::ARG_ERROR, &e.to_string(), None);
                return ExitCode::from(exit_code::ARG_ERROR);
            }
        }
    } else {
        match add_from_stdin() {
            Ok(p) => p,
            Err(e) => {
                emit_error_envelope(mode, exit_code::ARG_ERROR, &e.to_string(), None);
                return ExitCode::from(exit_code::ARG_ERROR);
            }
        }
    };
    let (name, transport, _enabled, config) = parsed;
    if let Err(e) = upsert_server(root, &name, &config) {
        emit_error_envelope(mode, exit_code::FS_ERROR, &e.to_string(), None);
        return ExitCode::from(exit_code::FS_ERROR);
    }
    if mode.is_json() {
        emit_json(
            mode,
            &serde_json::json!({
                "ok": true,
                "action": "add",
                "name": name,
                "transport": transport,
            }),
        );
    } else {
        emit_line(mode, format!("mcp `{name}` 已写入 mcp.json"));
    }
    ExitCode::SUCCESS
}

fn add_interactive() -> Result<(String, McpTransport, bool, Value), String> {
    let name = prompt_required("server name")?;
    let transport_raw = prompt_optional("transport (stdio|http|sse)", "stdio")?;
    let command = prompt_optional("command (e.g. uvx)", "")?;
    let args_raw = prompt_optional("args (逗号分隔,留空跳过)", "")?;
    let env_raw = prompt_optional("env (KEY=VAL, 逗号分隔, 留空跳过)", "")?;
    let enabled_raw = prompt_optional("enabled (y/n, 默认 y)", "y")?;
    let enabled = !enabled_raw.trim().eq_ignore_ascii_case("n");

    let transport = match transport_raw.to_ascii_lowercase().as_str() {
        "stdio" => McpTransport::Stdio,
        "http" => McpTransport::Http,
        "sse" => McpTransport::Sse,
        other => return Err(format!("未知 transport: {other}")),
    };

    let args: Vec<String> = if args_raw.trim().is_empty() {
        vec![]
    } else {
        args_raw.split(',').map(|s| s.trim().to_string()).collect()
    };
    let env: Map<String, Value> = if env_raw.trim().is_empty() {
        Map::new()
    } else {
        let mut m = Map::new();
        for kv in env_raw.split(',') {
            let kv = kv.trim();
            if let Some((k, v)) = kv.split_once('=') {
                m.insert(k.trim().to_string(), Value::String(v.trim().to_string()));
            }
        }
        m
    };

    let mut config = Map::new();
    if !command.trim().is_empty() {
        config.insert(
            "command".to_string(),
            Value::String(command.trim().to_string()),
        );
    }
    if !args.is_empty() {
        config.insert(
            "args".to_string(),
            Value::Array(args.into_iter().map(Value::String).collect()),
        );
    }
    if !env.is_empty() {
        config.insert("env".to_string(), Value::Object(env));
    }
    Ok((name, transport, enabled, Value::Object(config)))
}

fn add_from_stdin() -> Result<(String, McpTransport, bool, Value), String> {
    let mut buf = String::new();
    io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&buf).map_err(|e| e.to_string())?;
    let name = v
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "stdin JSON 缺 `name` 字段".to_string())?
        .to_string();
    let transport = match v
        .get("transport")
        .and_then(Value::as_str)
        .unwrap_or("stdio")
    {
        "stdio" => McpTransport::Stdio,
        "http" => McpTransport::Http,
        "sse" => McpTransport::Sse,
        other => return Err(format!("未知 transport: {other}")),
    };
    let enabled = v.get("enabled").and_then(Value::as_bool).unwrap_or(true);
    let config = v
        .get("config")
        .cloned()
        .ok_or_else(|| "stdin JSON 缺 `config` 字段".to_string())?;
    Ok((name, transport, enabled, config))
}

fn prompt_required(label: &str) -> Result<String, String> {
    loop {
        eprint!("{label}> ");
        io::stderr().flush().ok();
        let mut buf = String::new();
        io::stdin().read_line(&mut buf).map_err(|e| e.to_string())?;
        let v = buf.trim().to_string();
        if !v.is_empty() {
            return Ok(v);
        }
    }
}

fn prompt_optional(label: &str, default: &str) -> Result<String, String> {
    eprint!("{label} [{default}]> ");
    io::stderr().flush().ok();
    let mut buf = String::new();
    io::stdin().read_line(&mut buf).map_err(|e| e.to_string())?;
    let v = buf.trim();
    if v.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(v.to_string())
    }
}

fn upsert_server(root: &Utf8Path, name: &str, config: &Value) -> Result<(), String> {
    if name.is_empty() || name.contains('/') {
        return Err("server name 非法: 不可为空,不可含 '/'".to_string());
    }
    mcp_json::ensure_mcp_json(root).map_err(|e| e.to_string())?;
    mcp_json::upsert_server_in_document(root, name, config.clone()).map_err(|e| e.to_string())
}

// ── migrate(legacy mcp/servers 或模板 → mcp.json)────────────────

fn run_migrate(mode: OutputMode, root: &Utf8Path, source: Option<&str>, dry_run: bool) -> ExitCode {
    let rel_source = source.unwrap_or("mcp/cursor.mcp.template.json");
    let template_path = root.join(rel_source);
    let legacy_servers = root.join("mcp/servers");

    if dry_run {
        let count = if legacy_servers.is_dir() {
            std::fs::read_dir(legacy_servers.as_std_path())
                .map(|rd| {
                    rd.flatten()
                        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
                        .count()
                })
                .unwrap_or(0)
        } else if template_path.is_file() {
            count_template_servers(&template_path).unwrap_or(0)
        } else {
            0
        };
        if mode.is_json() {
            emit_json(
                mode,
                &serde_json::json!({
                    "written": [],
                    "skipped": [],
                    "errors": [],
                    "dry_run": true,
                    "would_migrate": count,
                }),
            );
        } else {
            println!("dry-run 已迁移 {count} 个 / 跳过 0 个 / 失败 0 个");
        }
        return ExitCode::SUCCESS;
    }

    if !legacy_servers.is_dir() && !template_path.is_file() && !root.join("mcp.json").is_file() {
        let msg = format!("migrate 源不存在: 无 `mcp/servers/` 且无 `{template_path}`");
        emit_error_envelope(
            mode,
            exit_code::FS_ERROR,
            &msg,
            Some("放置模板或 legacy servers"),
        );
        return ExitCode::from(exit_code::FS_ERROR);
    }

    if let Err(e) = mcp_json::migrate_legacy_mcp_layout(root) {
        emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
        return ExitCode::from(e.exit_code());
    }

    let written = mcp_json::list_server_names(root).unwrap_or_default();
    if mode.is_json() {
        emit_json(
            mode,
            &serde_json::json!({
                "written": written,
                "skipped": [],
                "errors": [],
                "dry_run": false,
            }),
        );
    } else {
        println!("已迁移 {} 个 / 跳过 0 个 / 失败 0 个", written.len());
    }
    ExitCode::SUCCESS
}

fn count_template_servers(path: &Utf8Path) -> Result<usize, String> {
    let raw = std::fs::read_to_string(path.as_std_path()).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    Ok(v.get("mcpServers")
        .and_then(|x| x.as_object())
        .map(|m| m.len())
        .unwrap_or(0))
}

fn run_remove(mode: OutputMode, root: &Utf8Path, name: &str) -> ExitCode {
    let found = match find_server(root, name) {
        Ok(v) => v,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code_kind(), &e.to_string(), e.hint_text());
            return ExitCode::from(e.exit_code_kind());
        }
    };
    let Some((server, _path)) = found else {
        let msg = format!("mcp server `{name}` 找不到");
        let hint = "跑 `ai-config mcp list` 看全部";
        emit_error_envelope(mode, exit_code::PARTIAL_FAILURE, &msg, Some(hint));
        return ExitCode::from(exit_code::PARTIAL_FAILURE);
    };
    if let Err(e) = mcp_json::remove_server_from_document(root, &server.name) {
        emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
        return ExitCode::from(e.exit_code());
    }
    if mode.is_json() {
        emit_json(
            mode,
            &serde_json::json!({
                "ok": true,
                "action": "remove",
                "name": name,
            }),
        );
    } else {
        emit_line(mode, format!("mcp `{name}` 已从 mcp.json 移除"));
    }
    ExitCode::SUCCESS
}

fn run_set_enabled(mode: OutputMode, root: &Utf8Path, name: &str, enabled: bool) -> ExitCode {
    if enabled {
        let msg = "单文件 mcp.json 模式不支持 enable;请用 `mcp add` 或编辑 mcp.json";
        emit_error_envelope(mode, exit_code::ARG_ERROR, msg, None);
        return ExitCode::from(exit_code::ARG_ERROR);
    }
    run_remove(mode, root, name)
}

// ── deploy / retract(per-item × per-platform) ─────────────────

fn run_deploy(mode: OutputMode, root: &Utf8Path, name: &str, plat: PlatformId) -> ExitCode {
    let adapter = match platform::for_id(plat) {
        Ok(a) => a,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            return ExitCode::from(e.exit_code());
        }
    };
    let dest = adapter.mcp_deploy_path();
    let result = if plat == PlatformId::Hermes {
        let config = match mcp_json::get_server_config(root, name) {
            Ok(Some(c)) => c,
            Ok(None) => {
                let msg = format!("mcp server `{name}` 找不到");
                emit_error_envelope(
                    mode,
                    exit_code::PARTIAL_FAILURE,
                    &msg,
                    Some("跑 `ai-config mcp list` 看全部"),
                );
                return ExitCode::from(exit_code::PARTIAL_FAILURE);
            }
            Err(e) => {
                emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
                return ExitCode::from(e.exit_code());
            }
        };
        mcp_json::upsert_server_on_platform(plat, &dest, name, &config)
    } else {
        let src = match mcp_json_path_for_root(root) {
            Ok(p) => p,
            Err(e) => {
                emit_error_envelope(mode, exit_code::FS_ERROR, &e, None);
                return ExitCode::from(exit_code::FS_ERROR);
            }
        };
        let _ = name;
        mcp_json::deploy_mcp_json_file(&src, &dest, plat)
    };
    match result {
        Ok(()) => {
            if mode.is_json() {
                emit_json(
                    mode,
                    &serde_json::json!({
                        "ok": true,
                        "action": "deploy",
                        "name": name,
                        "to": plat,
                        "dest": dest.to_string(),
                    }),
                );
            } else if plat == PlatformId::Hermes {
                emit_line(
                    mode,
                    format!("mcp `{name}` 已写入 Hermes config.yaml ({dest})"),
                );
            } else {
                emit_line(mode, format!("mcp.json 已 deploy 到 {plat:?}"));
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            ExitCode::from(e.exit_code())
        }
    }
}

fn run_retract(mode: OutputMode, root: &Utf8Path, name: &str, plat: PlatformId) -> ExitCode {
    let _ = root;
    let adapter = match platform::for_id(plat) {
        Ok(a) => a,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            return ExitCode::from(e.exit_code());
        }
    };
    let dest = adapter.mcp_deploy_path();
    let result = if plat == PlatformId::Hermes {
        mcp_json::remove_server_on_platform(plat, &dest, name)
    } else {
        let _ = name;
        mcp_json::retract_platform_mcp_json(&dest, plat)
    };
    match result {
        Ok(()) => {
            if mode.is_json() {
                emit_json(
                    mode,
                    &serde_json::json!({
                        "ok": true,
                        "action": "retract",
                        "name": name,
                        "from": plat,
                        "dest": dest.to_string(),
                    }),
                );
            } else if plat == PlatformId::Hermes {
                emit_line(
                    mode,
                    format!("mcp `{name}` 已从 Hermes config.yaml 移除 ({dest})"),
                );
            } else {
                emit_line(mode, format!("mcp.json 已 retract 从 {plat:?}"));
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            ExitCode::from(e.exit_code())
        }
    }
}

fn run_migrate_hermes(mode: OutputMode, dry_run: bool) -> ExitCode {
    let home = paths::home_dir();
    let legacy = home.join(".hermes/mcp.json");
    if dry_run {
        let exists = legacy.is_file();
        if mode.is_json() {
            emit_json(
                mode,
                &serde_json::json!({
                    "dry_run": true,
                    "legacy_exists": exists,
                    "legacy_path": legacy.to_string(),
                    "target": paths::home_dir().join(".hermes/config.yaml").to_string(),
                }),
            );
        } else if exists {
            emit_line(
                mode,
                format!("dry-run: 将把 {legacy} 合并进 ~/.hermes/config.yaml"),
            );
        } else {
            emit_line(mode, "dry-run: 无遗留 ~/.hermes/mcp.json");
        }
        return ExitCode::SUCCESS;
    }
    match mcp_json::migrate_legacy_hermes_mcp_json(&home) {
        Ok(report) => emit_migrate_hermes_report(mode, &report),
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            ExitCode::from(e.exit_code())
        }
    }
}

fn emit_migrate_hermes_report(mode: OutputMode, report: &HermesMigrateReport) -> ExitCode {
    if mode.is_json() {
        emit_json(
            mode,
            &serde_json::json!({
                "merged": report.merged,
                "skipped_conflict": report.skipped_conflict,
                "legacy_renamed": report.legacy_renamed,
            }),
        );
    } else {
        if report.merged.is_empty() && report.skipped_conflict.is_empty() {
            emit_line(mode, "无遗留 ~/.hermes/mcp.json 需要迁移");
        } else {
            emit_line(
                mode,
                format!(
                    "已合并 {} 条, 跳过 {} 条",
                    report.merged.len(),
                    report.skipped_conflict.len()
                ),
            );
            for name in &report.merged {
                emit_line(mode, format!("  merged: {name}"));
            }
            for msg in &report.skipped_conflict {
                emit_line(mode, format!("  skip: {msg}"));
            }
        }
        if let Some(bak) = &report.legacy_renamed {
            emit_line(mode, format!("遗留文件已重命名为 {bak}"));
        }
    }
    ExitCode::SUCCESS
}

// ── 平台 ID 解析 ────────────────────────────────────────────────

fn parse_platform(s: &str) -> Result<PlatformId, String> {
    match s.to_ascii_lowercase().as_str() {
        "cursor" => Ok(PlatformId::Cursor),
        "codex" => Ok(PlatformId::Codex),
        "claude" | "claudecode" | "claude-code" => Ok(PlatformId::Claude),
        "hermes" => Ok(PlatformId::Hermes),
        other => Err(format!("未知平台: {other};可选 cursor/codex/claude/hermes")),
    }
}

// ── 工具:从 mcp.json 读 servers ─────────────────────────────────

trait CliErrorExt {
    fn exit_code_kind(&self) -> u8;
    fn hint_text(&self) -> Option<&str>;
}
impl CliErrorExt for anyhow::Error {
    fn exit_code_kind(&self) -> u8 {
        self.downcast_ref::<CoreError>()
            .map(|e| e.exit_code())
            .unwrap_or(exit_code::FS_ERROR)
    }
    fn hint_text(&self) -> Option<&str> {
        self.downcast_ref::<CoreError>().and_then(|e| e.hint())
    }
}

fn mcp_json_path_for_root(root: &Utf8Path) -> Result<Utf8PathBuf, String> {
    let scan = source::scan_project_root(root).map_err(|e| e.to_string())?;
    scan.mcp_json
        .ok_or_else(|| format!("`{}` 不存在", mcp_json::MCP_ASSET_NAME))
}

fn load_all_servers(root: &Utf8Path) -> anyhow::Result<Vec<McpListEntry>> {
    let path = mcp_json_path_for_root(root).map_err(|e| anyhow::anyhow!(e))?;
    let doc = mcp_json::load_mcp_document(root)?
        .unwrap_or_else(|| serde_json::json!({ "mcpServers": {} }));
    let mut out = Vec::new();
    if let Some(servers) = doc.get("mcpServers").and_then(|v| v.as_object()) {
        for (name, config) in servers {
            let srv = server_from_config(name, config);
            out.push(McpListEntry {
                name: srv.name,
                transport: format!("{:?}", srv.transport).to_ascii_lowercase(),
                enabled: true,
                source_path: path.as_str().to_string(),
                secret_keys: srv.secret_keys,
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn find_server(root: &Utf8Path, name: &str) -> anyhow::Result<Option<(McpServer, Utf8PathBuf)>> {
    let path = mcp_json_path_for_root(root).map_err(|e| anyhow::anyhow!(e))?;
    let doc = mcp_json::load_mcp_document(root)?;
    let Some(doc) = doc else {
        return Ok(None);
    };
    let Some(config) = doc
        .get("mcpServers")
        .and_then(|v| v.as_object())
        .and_then(|m| m.get(name))
    else {
        return Ok(None);
    };
    Ok(Some((server_from_config(name, config), path)))
}

fn server_from_config(name: &str, config: &Value) -> McpServer {
    let transport = if config.get("url").is_some()
        || config
            .get("type")
            .and_then(|v| v.as_str())
            .is_some_and(|t| t.eq_ignore_ascii_case("http") || t.eq_ignore_ascii_case("sse"))
    {
        McpTransport::Http
    } else {
        McpTransport::Stdio
    };
    McpServer::new(0, name, transport, config.clone())
}
