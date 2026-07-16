//! `ai-config mcp ...` 子命令(PRD §4.2 / §5.1 / §10 A-7 / A-8 / A-15)。
//!
//! T007 安全边界：source-first per-server CRUD 与 generated executor 就绪前，
//! `add` / `remove` / `enable` / `disable` / `deploy` / `retract` 一律 fail-closed。
//! 仅 `list` / `show` 和 migration 的 `--dry-run` 可读取现有状态。

use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;
use serde_json::Value;

use ai_config_core::error::{exit_code, CoreError};
use ai_config_core::model::McpServer;
use ai_config_core::paths;
use ai_config_core::projection::mcp::source::load_mcp_definitions;

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
    /// 兼容保留的旧写入口；T007 期间一律拒绝。
    Deploy {
        name: String,
        to: String,
    },
    /// 兼容保留的旧写入口；T007 期间一律拒绝。
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
            McpCmd::Add => refuse_legacy_write(mode, "add"),
            McpCmd::Remove { name } => {
                let _ = name;
                refuse_legacy_write(mode, "remove")
            }
            McpCmd::Enable { name } => {
                let _ = name;
                refuse_legacy_write(mode, "enable")
            }
            McpCmd::Disable { name } => {
                let _ = name;
                refuse_legacy_write(mode, "disable")
            }
            McpCmd::Deploy { name, to } => {
                let _ = (name, to);
                refuse_legacy_write(mode, "deploy")
            }
            McpCmd::Retract { name, from } => {
                let _ = (name, from);
                refuse_legacy_write(mode, "retract")
            }
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
    let servers = match load_canonical_servers(root) {
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
    let found = match find_canonical_server(root, name) {
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
            }),
        );
    } else {
        println!("name: {}", srv.name);
        println!("transport: {:?}", srv.transport);
        println!("enabled: {}", srv.enabled);
        println!("source: {}", path);
        println!("secret_keys: {:?}", srv.secret_keys);
    }
    ExitCode::SUCCESS
}

// ── legacy writes: fail closed until source-first CRUD + executor exists ──

fn refuse_legacy_write(mode: OutputMode, action: &str) -> ExitCode {
    emit_error_envelope(
        mode,
        exit_code::ARG_ERROR,
        &format!(
            "legacy MCP `{action}` 已禁用：source-first per-server CRUD 与 generated executor 尚未就绪，拒绝写入旧 MCP 资产"
        ),
        Some("可使用 `ai-config mcp list` / `show` 或 migration 的 `--dry-run` 盘点；待 source-first apply 就绪后再执行单项变更"),
    );
    ExitCode::from(exit_code::ARG_ERROR)
}

// ── migrate(legacy inputs → source-first plan; writes are deliberately disabled) ──

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

    emit_error_envelope(
        mode,
        exit_code::ARG_ERROR,
        "legacy MCP migrate 只读：source-first apply 尚未就绪，拒绝写入或删除旧 MCP 资产",
        Some("使用 `ai-config mcp migrate --dry-run` 盘点；待生成 source-first 计划后再执行单项 apply"),
    );
    ExitCode::from(exit_code::ARG_ERROR)
}

fn count_template_servers(path: &Utf8Path) -> Result<usize, String> {
    let raw = std::fs::read_to_string(path.as_std_path()).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    Ok(v.get("mcpServers")
        .and_then(|x| x.as_object())
        .map(|m| m.len())
        .unwrap_or(0))
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
    emit_error_envelope(
        mode,
        exit_code::ARG_ERROR,
        "legacy Hermes MCP migrate 只读：source-first apply 尚未就绪，拒绝写入或重命名旧 MCP 资产",
        Some("使用 `ai-config mcp migrate-hermes --dry-run` 盘点；待生成 source-first 计划后再执行单项 apply"),
    );
    ExitCode::from(exit_code::ARG_ERROR)
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

fn load_canonical_servers(root: &Utf8Path) -> anyhow::Result<Vec<McpListEntry>> {
    let mut out = load_mcp_definitions(root)?
        .into_iter()
        .map(|definition| McpListEntry {
            name: definition.server.name,
            transport: format!("{:?}", definition.server.transport).to_ascii_lowercase(),
            enabled: definition.server.enabled,
            source_path: definition.source_path.to_string(),
            secret_keys: definition.server.secret_keys,
        })
        .collect::<Vec<_>>();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn find_canonical_server(
    root: &Utf8Path,
    name: &str,
) -> anyhow::Result<Option<(McpServer, Utf8PathBuf)>> {
    Ok(load_mcp_definitions(root)?
        .into_iter()
        .find(|definition| definition.server.name == name)
        .map(|definition| (definition.server, definition.source_path)))
}
