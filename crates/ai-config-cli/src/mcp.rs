//! `ai-config mcp ...` 子命令(PRD §4.2 / §5.1 / §10 A-7 / A-8 / A-15)。
//!
//! T007 安全边界：source-first CRUD 仅变更 canonical source；单项 `deploy` / `retract`
//! 必须先构建经审核的 source-first projection plan，再交由 generated executor 事务执行。
//! legacy migration 仅保留只读盘点；历史 secret-extraction 写路径已被 fail-closed。

use std::collections::BTreeMap;
use std::process::ExitCode;

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use serde::Serialize;
use serde_json::Value;

use ai_config_core::error::{exit_code, CoreError};
use ai_config_core::model::{McpServer, PlatformId};
use ai_config_core::paths;
use ai_config_core::projection::executor::{
    apply_projection_plan, ApplyOptions, ExecutorContext, McpSecretProvider,
};
use ai_config_core::projection::ledger::MemoryProjectionLedger;
use ai_config_core::projection::mcp::source::{
    load_mcp_definition_at, load_mcp_definitions, resolve_effective_mcp_definitions,
    EffectiveMcpDefinition,
};
use ai_config_core::projection::model::DeploymentScope;
use ai_config_core::projection::planner::{
    build_mcp_projection_plan, McpSecretAvailability, PlannerContext, ProjectionActionKind,
    ProjectionOperation, ProjectionRequest,
};
use ai_config_core::projection::source::OverlayRoots;
use ai_config_core::secrets;

use crate::output::{emit_error_envelope, emit_json, emit_line, OutputMode};

#[derive(Debug, Clone)]
pub enum McpCmd {
    List,
    Show {
        name: String,
    },
    Add {
        source: String,
    },
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
        extract_secrets: bool,
        apply: bool,
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
            McpCmd::Add { source } => run_add(mode, default_root, &source),
            McpCmd::Remove { name } => run_remove(mode, default_root, &name),
            McpCmd::Enable { name } => run_set_enabled(mode, default_root, &name, true),
            McpCmd::Disable { name } => run_set_enabled(mode, default_root, &name, false),
            McpCmd::Deploy { name, to } => {
                run_projection_command(mode, default_root, &name, &to, ProjectionOperation::Sync)
            }
            McpCmd::Retract { name, from } => run_projection_command(
                mode,
                default_root,
                &name,
                &from,
                ProjectionOperation::Retract,
            ),
            McpCmd::Migrate {
                source,
                dry_run,
                extract_secrets,
                apply,
            } => run_migrate(
                mode,
                default_root,
                source.as_deref(),
                dry_run,
                extract_secrets,
                apply,
            ),
            McpCmd::MigrateHermes { dry_run } => run_migrate_hermes(mode, dry_run),
        }
    }
}

/// CLI-owned secret boundary for a single source-first plan/apply invocation.  It is populated
/// only through `secrets::load_from`, which rejects an existing store unless it is strict 0600.
/// Neither planner nor executor receives the map itself, and no error/report serializes values.
struct CliMcpSecrets {
    values: BTreeMap<String, String>,
}

impl CliMcpSecrets {
    fn load() -> Result<Self, CoreError> {
        let pairs = secrets::load_from(&secrets::default_path())?;
        Ok(Self {
            values: pairs.into_iter().collect(),
        })
    }
}

impl McpSecretAvailability for CliMcpSecrets {
    fn missing_secret_keys(&self, declared_keys: &[String]) -> Result<Vec<String>, CoreError> {
        Ok(declared_keys
            .iter()
            .filter(|key| !self.values.contains_key(key.as_str()))
            .cloned()
            .collect())
    }
}

impl McpSecretProvider for CliMcpSecrets {
    fn resolve(&self, key: &str) -> Result<Option<String>, CoreError> {
        Ok(self.values.get(key).cloned())
    }
}

#[derive(Serialize)]
struct McpProjectionCommandReport<'a> {
    operation: &'a str,
    server: &'a str,
    platform: &'a str,
    report: ai_config_core::projection::executor::ApplyReport,
}

/// The old CLI entrypoint is intentionally only an orchestration layer.  It never renders a
/// platform container directly: source lookup -> read-only plan -> exact plan authorization ->
/// transactional core executor.  `deploy_base` comes from the selected CLI scope, so all target
/// writes remain inside the caller-selected user/project boundary.
fn run_projection_command(
    mode: OutputMode,
    default_root: &Utf8Path,
    name: &str,
    platform_raw: &str,
    operation: ProjectionOperation,
) -> ExitCode {
    let result = (|| -> Result<McpProjectionCommandReport<'_>, CoreError> {
        let platform = parse_mcp_platform(platform_raw)?;
        let roots = paths::resolve_sync_roots(default_root);
        let scope = if paths::is_project_deploy_base(&roots.deploy_base) {
            DeploymentScope::Project
        } else {
            DeploymentScope::User
        };
        let definitions = effective_mcp_definitions(&roots)?;
        let definition = definitions
            .into_iter()
            .find(|definition| definition.definition.server.name == name)
            .ok_or_else(|| missing_canonical_definition(name))?;
        if operation == ProjectionOperation::Sync && !definition.definition.enabled_for(platform) {
            return Err(CoreError::InvalidPath(format!(
                "canonical MCP server `{name}` is disabled or does not target `{platform_raw}`"
            )));
        }

        let secrets = CliMcpSecrets::load()?;
        let request = ProjectionRequest {
            operation,
            scope_key: projection_scope_key(scope, &roots.deploy_base),
            scope,
            deploy_base: roots.deploy_base.clone(),
            assets: Vec::new(),
            platforms: vec![platform],
        };
        // A persistent projection ledger is deliberately not invented in the CLI.  Until the
        // durable ledger slice lands, a later process can never prove ownership for retract and
        // therefore turns it into a zero-write conflict rather than guessing from container data.
        let ledger = MemoryProjectionLedger::default();
        let plan = build_mcp_projection_plan(
            &request,
            &[definition],
            &PlannerContext::new(&ledger).with_mcp_secret_availability(&secrets),
        )?;
        if plan.actions.is_empty() {
            return Err(CoreError::InvalidPath(
                "requested MCP server has no eligible source-first projection action".to_owned(),
            ));
        }
        if let Some(conflict) = plan.actions.iter().find(|action| {
            action.kind == ProjectionActionKind::ReportOnly
                && !(action.state.as_deref() == Some("skipped")
                    && action.reason_code == "mcp_missing_secret_keys")
        }) {
            return Err(CoreError::InvalidPath(format!(
                "mcp_projection_plan_conflict:{}",
                conflict.reason_code
            )));
        }
        let backup_root = roots.deploy_base.join(".ai-config/projection-backups");
        let report = apply_projection_plan(
            &plan,
            &ExecutorContext::new(&ledger, roots.deploy_base.clone(), backup_root)
                .with_mcp_secret_provider(&secrets),
            ApplyOptions::for_plan(&plan),
        )
        .map_err(|error| error.error)?;
        Ok(McpProjectionCommandReport {
            operation: if operation == ProjectionOperation::Sync {
                "deploy"
            } else {
                "retract"
            },
            server: name,
            platform: platform_raw,
            report,
        })
    })();

    match result {
        Ok(report) => {
            if mode.is_json() {
                emit_json(mode, &report);
            } else {
                emit_line(
                    mode,
                    format!(
                        "mcp {} {} -> {}: changed={}, skipped={}, unchanged={}",
                        report.operation,
                        report.server,
                        report.platform,
                        report.report.changed,
                        report.report.skipped,
                        report.report.unchanged,
                    ),
                );
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            emit_error_envelope(mode, error.exit_code(), &error.to_string(), error.hint());
            ExitCode::from(error.exit_code())
        }
    }
}

fn effective_mcp_definitions(
    roots: &paths::SyncRoots,
) -> Result<Vec<EffectiveMcpDefinition>, CoreError> {
    let overlays = if paths::is_project_deploy_base(&roots.deploy_base) {
        OverlayRoots {
            global: roots.global_default.clone(),
            workspace: None,
            project: roots.asset_root.clone(),
        }
    } else {
        // Do not load the same root twice: source provenance remains `Global` for a global CLI
        // invocation and an absent project layer cannot shadow it.
        OverlayRoots {
            global: roots.asset_root.clone(),
            workspace: None,
            project: roots.asset_root.join(".ai-config-no-project-overlay"),
        }
    };
    resolve_effective_mcp_definitions(&overlays)
}

fn missing_canonical_definition(name: &str) -> CoreError {
    CoreError::InvalidPath(format!(
        "source-first canonical MCP server `{name}` was not found; 拒绝写入 legacy MCP asset"
    ))
}

fn projection_scope_key(scope: DeploymentScope, deploy_base: &Utf8Path) -> String {
    match scope {
        DeploymentScope::User => format!("user:{}", deploy_base),
        DeploymentScope::Workspace => format!("workspace:{}", deploy_base),
        DeploymentScope::Project => format!("project:{}", deploy_base),
    }
}

fn parse_mcp_platform(raw: &str) -> Result<PlatformId, CoreError> {
    match raw.to_ascii_lowercase().as_str() {
        "cursor" => Ok(PlatformId::Cursor),
        "codex" => Ok(PlatformId::Codex),
        "claude" | "claudecode" | "claude-code" => Ok(PlatformId::Claude),
        "hermes" => Ok(PlatformId::Hermes),
        _ => Err(CoreError::InvalidPath(format!(
            "unknown MCP deploy platform `{raw}`; use cursor, codex, claude, or hermes"
        ))),
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

// ── source-first CRUD (source only, no platform lifecycle cutover) ──────────

fn run_add(mode: OutputMode, root: &Utf8Path, source: &str) -> ExitCode {
    let source = Utf8Path::new(source);
    let result = (|| -> anyhow::Result<Utf8PathBuf> {
        reject_symlink(source, "MCP input source")?;
        let definition = load_mcp_definition_at(source)?;
        let destination = canonical_server_path(root, &definition.server.name)?;
        ensure_canonical_parent(root)?;
        if destination.exists() {
            anyhow::bail!(
                "canonical MCP source `{}` already exists",
                definition.server.name
            );
        }
        let raw = std::fs::read(source.as_std_path())?;
        write_new_file(&destination, &raw)?;
        // Re-parse the exact persisted bytes so a bad input can never become a source.
        load_mcp_definition_at(&destination)?;
        Ok(destination)
    })();
    match result {
        Ok(destination) => {
            emit_mutation_success(mode, "add", &destination);
            ExitCode::SUCCESS
        }
        Err(error) => emit_crud_error(mode, error),
    }
}

fn run_remove(mode: OutputMode, root: &Utf8Path, name: &str) -> ExitCode {
    let result = (|| -> anyhow::Result<Utf8PathBuf> {
        let path = canonical_server_path(root, name)?;
        reject_symlink(&path, "canonical MCP source")?;
        let definition = load_mcp_definition_at(&path)?;
        if definition.server.name != name {
            anyhow::bail!("canonical MCP source name does not match requested server");
        }
        std::fs::remove_file(path.as_std_path())?;
        Ok(path)
    })();
    match result {
        Ok(path) => {
            emit_mutation_success(mode, "remove", &path);
            ExitCode::SUCCESS
        }
        Err(error) => emit_crud_error(mode, error),
    }
}

fn run_set_enabled(mode: OutputMode, root: &Utf8Path, name: &str, enabled: bool) -> ExitCode {
    let result = (|| -> anyhow::Result<(Utf8PathBuf, bool)> {
        let path = canonical_server_path(root, name)?;
        reject_symlink(&path, "canonical MCP source")?;
        let definition = load_mcp_definition_at(&path)?;
        if definition.server.name != name {
            anyhow::bail!("canonical MCP source name does not match requested server");
        }
        if definition.server.enabled == enabled {
            return Ok((path, false));
        }
        let raw = std::fs::read_to_string(path.as_std_path())?;
        let mut document: Value = serde_json::from_str(&raw)?;
        let object = document
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("canonical MCP source must be a JSON object"))?;
        object.insert("enabled".to_owned(), Value::Bool(enabled));
        let rendered = serde_json::to_vec_pretty(&document)?;
        write_replace_file(&path, &rendered)?;
        load_mcp_definition_at(&path)?;
        Ok((path, true))
    })();
    match result {
        Ok((path, changed)) => {
            if mode.is_json() {
                emit_json(
                    mode,
                    &serde_json::json!({
                        "action": if enabled { "enable" } else { "disable" },
                        "source_path": path,
                        "changed": changed,
                    }),
                );
            } else if changed {
                println!("{}: {}", if enabled { "enabled" } else { "disabled" }, path);
            } else {
                println!("unchanged: {path}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => emit_crud_error(mode, error),
    }
}

fn canonical_server_path(root: &Utf8Path, name: &str) -> anyhow::Result<Utf8PathBuf> {
    if name.is_empty()
        || name.starts_with('.')
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        anyhow::bail!("MCP server name must use only ASCII letters, digits, '.', '-' or '_'");
    }
    Ok(root.join("mcp/servers").join(format!("{name}.json")))
}

fn ensure_canonical_parent(root: &Utf8Path) -> anyhow::Result<()> {
    let mcp = root.join("mcp");
    let servers = mcp.join("servers");
    for path in [&mcp, &servers] {
        if path.exists() {
            reject_symlink(path, "canonical MCP directory")?;
        } else {
            std::fs::create_dir(path.as_std_path())?;
        }
    }
    Ok(())
}

fn reject_symlink(path: &Utf8Path, what: &str) -> anyhow::Result<()> {
    if std::fs::symlink_metadata(path.as_std_path())
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        anyhow::bail!("{what} must not be a symlink");
    }
    Ok(())
}

fn write_new_file(path: &Utf8Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;

    let temp = temporary_path(path);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp.as_std_path())?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    match std::fs::hard_link(temp.as_std_path(), path.as_std_path()) {
        Ok(()) => {
            std::fs::remove_file(temp.as_std_path())?;
            Ok(())
        }
        Err(error) => {
            let _ = std::fs::remove_file(temp.as_std_path());
            Err(error.into())
        }
    }
}

fn write_replace_file(path: &Utf8Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;

    reject_symlink(path, "canonical MCP source")?;
    let temp = temporary_path(path);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp.as_std_path())?;
    file.write_all(bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temp.as_std_path(), path.as_std_path())?;
    Ok(())
}

fn temporary_path(path: &Utf8Path) -> Utf8PathBuf {
    let filename = path.file_name().unwrap_or("mcp.json");
    path.parent()
        .unwrap_or_else(|| Utf8Path::new("."))
        .join(format!(".{filename}.{}.tmp", std::process::id()))
}

fn emit_mutation_success(mode: OutputMode, action: &str, path: &Utf8Path) {
    if mode.is_json() {
        emit_json(
            mode,
            &serde_json::json!({ "action": action, "source_path": path, "changed": true }),
        );
    } else {
        println!("{action}: {path}");
    }
}

fn emit_crud_error(mode: OutputMode, error: anyhow::Error) -> ExitCode {
    emit_error_envelope(
        mode,
        exit_code::ARG_ERROR,
        &error.to_string(),
        Some("MCP source 必须是经过校验的单 server JSON；该操作不会写入任何平台配置"),
    );
    ExitCode::from(exit_code::ARG_ERROR)
}

// ── legacy platform writes: fail closed until generated executor exists ─────

// ── migrate(legacy container → source-first per-server sources) ────────────

fn run_migrate(
    mode: OutputMode,
    root: &Utf8Path,
    source: Option<&str>,
    dry_run: bool,
    _extract_secrets: bool,
    _apply: bool,
) -> ExitCode {
    let relative = Utf8Path::new(source.unwrap_or("mcp/cursor.mcp.template.json"));
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, Utf8Component::ParentDir))
    {
        emit_error_envelope(
            mode,
            exit_code::ARG_ERROR,
            "legacy MCP source must be a path below the selected root",
            Some("该只读盘点不会访问 --root 以外的路径"),
        );
        return ExitCode::from(exit_code::ARG_ERROR);
    }
    let template_path = root.join(relative);
    if dry_run {
        let count = std::fs::read_to_string(template_path.as_std_path())
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|value| {
                value
                    .get("mcpServers")?
                    .as_object()
                    .map(|items| items.len())
            })
            .unwrap_or(0);
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
        "legacy MCP migrate 只读：`--extract-secrets --apply` 不能绕过 reviewed source-first plan",
        Some(
            "使用 `ai-config mcp migrate --dry-run` 盘点；迁移请使用 `ai-config import` / `ai-config migrate source-first`",
        ),
    );
    ExitCode::from(exit_code::ARG_ERROR)
}

/// A prepared migration deliberately keeps literal values private. It is never Debug/Serialize.
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
