//! MCP 模板渲染 + 原子写。
//!
//! PRD v1.0 关键约束:
//! - §2.1 MCP 逐项:每条 server 独立条目;所有 enabled 合并到 `mcpServers`
//! - §2.1 缺变量明确告警,**不**写空字符串
//! - §6.1 退出码 4 = secrets 缺(`CoreError::SecretsMissing` 的 `exit_code()` 已经是 4)
//! - §10 A-7 / A-8:加 / 删 server → 4 份 mcp.json 一致
//! - §8.1 不擅改用户已有文件:原子写 tmp + rename,失败回滚;旧文件无 `managedBy` 字段**不**覆盖
//!
//! 实现策略:手写 `${VAR}` 替换(不依赖 minijinja / handlebars / tera),
//! 走 serde_json 合并,最后 `tokio::fs::rename` 原子提交(同 fs 同分区)。

#![allow(dead_code)]

use std::collections::{BTreeSet, HashMap};

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;
use serde_json::Value;

use crate::error::CoreError;
use crate::model::{McpServer, McpTransport, PlatformId};

// ── 公开常量 ────────────────────────────────────────────────────────

/// 本工具在生成 `mcp.json` 时写入的元字段(顶层)。
///
/// 读回时若**不**含此字段 → 视为用户手写 → `merge_with_existing` 返回
/// `CoreError::Unmanaged`,**不**写盘(PRD §8.1 硬约束 #5)。
pub const MANAGED_BY_KEY: &str = "managedBy";
pub const MANAGED_BY_VALUE: &str = "agent-manager";

/// `${VAR}` 匹配的占位符正则 — 取 `${` + 合法 key 字符 + `}`。
/// key 名约定与 POSIX env 兼容:`[A-Za-z_][A-Za-z0-9_]*`,并兼容点号(一些 MCP 工具
/// 用 `MINIO.ENDPOINT` 这类名字);保守起见放宽到 `[A-Za-z0-9_.]+`。
///
/// 返回一个**去重保序**的 `Vec<String>`:`BTreeSet` 用于去重(也顺便排序,稳定可测),
/// `Vec` 用于保 caller 拿到的顺序稳定且含原大小写(secret 区分大小写)。
pub(crate) fn collect_secret_keys(v: &Value) -> Vec<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out: Vec<String> = Vec::new();
    walk_collect(v, &mut seen, &mut out);
    out
}

fn walk_collect(v: &Value, seen: &mut BTreeSet<String>, out: &mut Vec<String>) {
    match v {
        Value::String(s) => scan_string(s, seen, out),
        Value::Array(arr) => {
            for item in arr {
                walk_collect(item, seen, out);
            }
        }
        Value::Object(map) => {
            for (_k, val) in map {
                walk_collect(val, seen, out);
            }
        }
        _ => {}
    }
}

fn scan_string(s: &str, seen: &mut BTreeSet<String>, out: &mut Vec<String>) {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'$' && bytes[i + 1] == b'{' {
            if let Some(end_rel) = find_closing_brace(&bytes[i + 2..]) {
                let raw = &s[i + 2..i + 2 + end_rel];
                if is_valid_key(raw) && seen.insert(raw.to_string()) {
                    out.push(raw.to_string());
                }
                i = i + 2 + end_rel + 1;
                continue;
            }
        }
        i += 1;
    }
}

fn find_closing_brace(bytes: &[u8]) -> Option<usize> {
    for (idx, b) in bytes.iter().enumerate() {
        if *b == b'}' {
            return Some(idx);
        }
        if !is_key_char(*b) {
            return None;
        }
    }
    None
}

fn is_key_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'.'
}

fn is_valid_key(raw: &str) -> bool {
    !raw.is_empty() && raw.bytes().all(is_key_char)
}

/// 把 `${VAR}` 占位符按 secrets 表替换。**不**在 map 里出现的 key 一律保留原样
/// (留给上层校验 → 报 `SecretsMissing`)。
fn substitute_string(s: &str, secrets: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'$' && bytes[i + 1] == b'{' {
            if let Some(end_rel) = find_closing_brace(&bytes[i + 2..]) {
                let raw = &s[i + 2..i + 2 + end_rel];
                if is_valid_key(raw) {
                    if let Some(value) = secrets.get(raw) {
                        out.push_str(value);
                    } else {
                        // 缺变量:原样保留(让 caller 一次性收集所有缺失 key)
                        out.push_str("${");
                        out.push_str(raw);
                        out.push('}');
                    }
                    i = i + 2 + end_rel + 1;
                    continue;
                }
            }
        }
        // 普通字节:char 边界处理交给 push_str
        let ch_end = next_char_boundary(s, i);
        out.push_str(&s[i..ch_end]);
        i = ch_end;
    }
    out
}

fn next_char_boundary(s: &str, i: usize) -> usize {
    let mut j = i + 1;
    while j < s.len() && !s.is_char_boundary(j) {
        j += 1;
    }
    j
}

fn substitute_value(v: Value, secrets: &HashMap<String, String>) -> Value {
    match v {
        Value::String(s) => Value::String(substitute_string(&s, secrets)),
        Value::Array(arr) => Value::Array(
            arr.into_iter()
                .map(|item| substitute_value(item, secrets))
                .collect(),
        ),
        Value::Object(map) => {
            let mut new_map = serde_json::Map::with_capacity(map.len());
            for (k, val) in map {
                new_map.insert(k, substitute_value(val, secrets));
            }
            Value::Object(new_map)
        }
        other => other,
    }
}

// ── 公开 API ────────────────────────────────────────────────────────

/// 解析 `mcp/servers/<name>.json` 为 `McpServer`。
///
/// 期望的 JSON 形态(PRD §2.1 逐项):
/// ```json
/// {
///   "name": "minio",                  // 可选;缺省取文件名
///   "transport": "stdio" | "http" | "sse",  // 可选;从 config 推断
///   "enabled": true,                  // 可选;默认 true
///   "config": { ... }                 // 必填,实际的 mcpServers[name] 内容
/// }
/// ```
///
/// **不**做 `${VAR}` 注入;只扫一遍收集 `secret_keys`,渲染时再注入。
pub fn parse_server(json_path: &Utf8Path) -> Result<McpServer, CoreError> {
    let raw = std::fs::read_to_string(json_path.as_std_path()).map_err(|e| {
        CoreError::Io(std::io::Error::new(
            e.kind(),
            format!(
                "read mcp/servers/{}: {e}",
                json_path.file_name().unwrap_or("?")
            ),
        ))
    })?;
    let parsed: Value = serde_json::from_str(&raw)?;

    // 提取 name
    let name = parsed
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            json_path
                .file_stem()
                .map(|s| s.to_owned())
                .filter(|s: &String| !s.is_empty())
        })
        .ok_or_else(|| CoreError::TemplateRender {
            template: json_path.as_str().to_owned(),
            reason: "无法从 JSON 或文件名推断 server name".to_owned(),
            hint: "在 JSON 里显式给 `\"name\": \"xxx\"`".to_owned(),
        })?;

    // 提取 transport
    let transport = match parsed.get("transport").and_then(Value::as_str) {
        Some("stdio") => McpTransport::Stdio,
        Some("http") => McpTransport::Http,
        Some("sse") => McpTransport::Sse,
        Some(other) => {
            return Err(CoreError::TemplateRender {
                template: json_path.as_str().to_owned(),
                reason: format!("未知 transport: {other:?}(仅 stdio/http/sse)"),
                hint: "改 `transport` 字段为 stdio / http / sse 之一".to_owned(),
            });
        }
        None => infer_transport(parsed.get("config")),
    };

    // 提取 config
    let config = parsed
        .get("config")
        .cloned()
        .ok_or_else(|| CoreError::TemplateRender {
            template: json_path.as_str().to_owned(),
            reason: "缺 `config` 字段".to_owned(),
            hint: "把启动命令 / url / headers 放到 `config` 对象里".to_owned(),
        })?;

    // 扫描所有 ${VAR} 占位符(去重保序)
    let secret_keys = collect_secret_keys(&config);

    let enabled = parsed
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    Ok(McpServer {
        project_id: 0, // 由 source::scan_project_root 注入;Phase 1 占位为 0
        name,
        transport,
        config,
        secret_keys,
        enabled,
    })
}

fn infer_transport(config: Option<&Value>) -> McpTransport {
    match config {
        Some(v) => {
            if v.get("url").is_some() {
                McpTransport::Http
            } else {
                McpTransport::Stdio
            }
        }
        None => McpTransport::Stdio,
    }
}

// ── 迁移器:整份 mcp.json 模板 → 逐项 mcp/servers/<name>.json ────

/// `mcp migrate` 的运行结果。
///
/// - `written`:成功写入磁盘的 name 列表(顺序 = 模板 `mcpServers` 顺序)
/// - `skipped`:被跳过的 (name, reason)— 当前 reason 仅 2 种:
///   - `"already exists"`(目标 `mcp/servers/<name>.json` 已存在,**不**覆盖)
///   - `"name 含 '/' 或为空"`(文件名非法,本工具拒绝)
/// - `errors`:解析 / 推断失败的 (name, error_message)— **不**写盘,留给用户处理
///
/// 序列化:`#[derive(Serialize)]`,CLI `--json` 模式直接吐;人类模式用 `Display`。
#[derive(Debug, Default, Clone, Serialize)]
pub struct MigrateReport {
    pub written: Vec<String>,
    pub skipped: Vec<(String, String)>,
    pub errors: Vec<(String, String)>,
}

/// 把整份 mcp.json 模板(`{"mcpServers": {...}}`)拆成 `mcp/servers/<name>.json` 逐项文件。
///
/// ## 字段映射(对齐 PRD §2.1 + `parse_server` 反推)
///
/// - 外层 `mcpServers.<name>` 的 key → 目标单文件 `name`(也是文件名)
/// - 模板 value 内的 `type`(`stdio`/`http`)→ 目标 `transport`(rename;`type` 字段**不**写进 config)
/// - 缺 `type` 时,`config.url` 存在 → `transport: "http"`,否则 `"stdio"`
/// - 整个模板 value(去掉 `type` 字段后)→ 目标 `config`
/// - `enabled: true` 默认
///
/// ## 行为
///
/// - `mcp/servers/<name>.json` 已存在 → `skipped`,**不覆盖**(避免丢用户手动改动)
/// - 文件名非法(只禁 `/` 和空字符串,允许空格 / `-` / `_` / `.`)→ `skipped`,记 reason
/// - 顶层缺 `mcpServers` 或非 object → 整体 `Err`
/// - 模板 value 内的 `type` 取到未知值(如 `"ws"`)→ 该条进 `errors`,**不**写盘,继续下一条
///
/// **不**处理 `${VAR}` 占位符 — `parse_server` 重新读时自动扫 `secret_keys`。
///
/// `dry_run`:若 true,**不**写盘(只跑 `would_write` 集合),但仍 `create_dir_all`
/// 会被跳过 — 整个过程 0 副作用,纯报告。
pub fn migrate_from_template(
    template_path: &Utf8Path,
    dest_servers_dir: &Utf8Path,
) -> Result<MigrateReport, CoreError> {
    migrate_from_template_inner(template_path, dest_servers_dir, false)
}

/// `--dry-run` 版本的 `migrate_from_template`(语义见上,0 副作用)。
pub fn migrate_from_template_dry_run(
    template_path: &Utf8Path,
    dest_servers_dir: &Utf8Path,
) -> Result<MigrateReport, CoreError> {
    migrate_from_template_inner(template_path, dest_servers_dir, true)
}

fn migrate_from_template_inner(
    template_path: &Utf8Path,
    dest_servers_dir: &Utf8Path,
    dry_run: bool,
) -> Result<MigrateReport, CoreError> {
    use std::fs;

    let raw =
        fs::read_to_string(template_path.as_std_path()).map_err(|e| CoreError::TemplateRender {
            template: template_path.as_str().to_owned(),
            reason: format!("读模板失败: {e}"),
            hint: "确认 source 路径存在、可读、是合法 JSON".to_owned(),
        })?;
    let parsed: Value = serde_json::from_str(&raw).map_err(|e| CoreError::TemplateRender {
        template: template_path.as_str().to_owned(),
        reason: format!("parse JSON 失败: {e}"),
        hint: "确认模板是合法 JSON,且顶层有 `mcpServers` object".to_owned(),
    })?;

    // 非 dry-run 才建 dest 目录(dry-run 0 副作用)
    if !dry_run {
        fs::create_dir_all(dest_servers_dir.as_std_path()).map_err(|e| {
            CoreError::TemplateRender {
                template: dest_servers_dir.as_str().to_owned(),
                reason: format!("创建 mcp/servers/ 目录失败: {e}"),
                hint: "确认 dest 路径的父目录可写".to_owned(),
            }
        })?;
    }

    let servers = parsed
        .get("mcpServers")
        .and_then(Value::as_object)
        .ok_or_else(|| CoreError::TemplateRender {
            template: template_path.as_str().to_owned(),
            reason: "顶层缺 `mcpServers` 字段或其不是 object".to_owned(),
            hint: "模板应为 `{\"mcpServers\": {\"<name>\": {...}, ...}}` 形态".to_owned(),
        })?;

    let mut report = MigrateReport::default();

    for (name, entry) in servers {
        // 1. 文件名合法性(只禁 `/` 和空,允许空格、`-`、`_`、`.`)
        if name.is_empty() || name.contains('/') {
            report
                .skipped
                .push((name.clone(), "name 含 '/' 或为空".to_owned()));
            continue;
        }

        // 2. 已存在 → skip(避免覆盖用户手动加的 enabled=false / description)
        //    dry-run 模式下**忽略**已存在检查(因为 dest 本身不存在),仍按"会写"算
        if !dry_run {
            let dest = dest_servers_dir.join(format!("{name}.json"));
            if dest.exists() {
                report
                    .skipped
                    .push((name.clone(), "already exists".to_owned()));
                continue;
            }
        }

        // 3. 推断 transport:type 优先,否则 url 存在 → http
        let transport = match entry.get("type").and_then(Value::as_str) {
            Some("stdio") => McpTransport::Stdio,
            Some("http") => McpTransport::Http,
            Some("sse") => McpTransport::Sse,
            Some(other) => {
                report
                    .errors
                    .push((name.clone(), format!("未知 type: {other:?}")));
                continue;
            }
            None => infer_transport(Some(entry)),
        };

        // 4. 构造 config:把 entry 内 `type` 字段剔除,剩下的全塞 config
        let mut config = entry.clone();
        if let Some(obj) = config.as_object_mut() {
            obj.remove("type");
        }

        // 5. 写单文件(dry-run 跳过 fs::write)
        if !dry_run {
            let payload = serde_json::json!({
                "name": name,
                "transport": match transport {
                    McpTransport::Stdio => "stdio",
                    McpTransport::Http => "http",
                    McpTransport::Sse => "sse",
                },
                "enabled": true,
                "config": config,
            });
            let body = match serde_json::to_string_pretty(&payload) {
                Ok(b) => b,
                Err(e) => {
                    report
                        .errors
                        .push((name.clone(), format!("serialize 失败: {e}")));
                    continue;
                }
            };
            let dest = dest_servers_dir.join(format!("{name}.json"));
            match fs::write(dest.as_std_path(), body) {
                Ok(()) => report.written.push(name.clone()),
                Err(e) => report
                    .errors
                    .push((name.clone(), format!("写文件失败: {e}"))),
            }
        } else {
            // dry-run:仅记 written(无 fs 操作)
            report.written.push(name.clone());
        }
    }

    Ok(report)
}

/// 渲染 mcp.json 到目标平台目录。
///
/// 流程:
/// 1. 过滤 `servers` 中的 `enabled` 项
/// 2. 对每条 config 做 `${VAR}` 替换(从 `secrets` map 取值)
/// 3. **缺变量**检查:任一占位符在 secrets 中**没有**对应 key → 返回
///    `CoreError::SecretsMissing`,并把**所有**缺失 key 列在 hint 里
///    (不写盘;不留空字符串 — PRD §2.1)
/// 4. 合并已有 mcp.json(如含 `managedBy == "agent-manager"`)
/// 5. 写到 `<dest>/mcp.json.tmp` → `rename` 到 `<dest>/mcp.json`(原子)
pub fn render_mcp(
    platform: PlatformId,
    dest_dir: &Utf8Path,
    servers: &[McpServer],
    secrets: &HashMap<String, String>,
) -> Result<Utf8PathBuf, CoreError> {
    let _ = platform; // Phase 1 占位:平台决定 mcp.json 路径;此处由 caller 给定 dest_dir

    // 1. enabled 过滤
    let enabled: Vec<&McpServer> = servers.iter().filter(|s| s.enabled).collect();

    // 3. 缺变量检查(先于任何写盘动作)
    let mut missing: Vec<(String, String)> = Vec::new(); // (server_name, key)
    for srv in &enabled {
        for key in &srv.secret_keys {
            if !secrets.contains_key(key) {
                missing.push((srv.name.clone(), key.clone()));
            }
        }
    }
    if !missing.is_empty() {
        // 第一个错(其它塞 hint)— 退出码 4(PRD §6.1)
        let (first_server, first_key) = missing[0].clone();
        let mut hint = format!("在 secrets.env 里补以下 {} 项:\n", missing.len());
        for (sname, key) in &missing {
            hint.push_str(&format!("  - server `{sname}` 缺 `{key}`\n"));
        }
        hint.push_str("写完后重跑 `agent-manager apply`");
        return Err(CoreError::SecretsMissing {
            key: first_key,
            template: format!("mcp/servers/{first_server}.json"),
            hint,
        });
    }

    // 2. 替换占位符 → 收集 mcpServers
    let mut mcp_servers = serde_json::Map::new();
    for srv in &enabled {
        let substituted = substitute_value(srv.config.clone(), secrets);
        mcp_servers.insert(srv.name.clone(), substituted);
    }

    // 4. 合并已有(若 managedBy == "agent-manager")
    let dest_file = dest_dir.join("mcp.json");
    let new_payload = merge_with_existing(&dest_file, Value::Object(mcp_servers))?;

    // 5. 原子写
    atomic_write_json(&dest_file, &new_payload)?;
    Ok(dest_file)
}

/// 单条 MCP 在目标 `mcp.json` 中的同步状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpSyncState {
    /// 目标文件不存在,或 `mcpServers` 无该 key
    Unlinked,
    /// key 存在且 config 与期望一致
    Linked,
    /// key 存在但 config 与期望不一致
    WrongValue,
    /// 文件存在但 JSON 非法或缺少 `mcpServers` 对象
    Broken,
}

/// 读取 `mcp.json` 顶层;文件不存在返回 `Ok(None)`。
pub fn read_mcp_json(path: &Utf8Path) -> Result<Option<Value>, CoreError> {
    match std::fs::read_to_string(path.as_std_path()) {
        Ok(raw) => {
            let v = serde_json::from_str(&raw).map_err(|e| CoreError::TemplateRender {
                template: path.as_str().to_owned(),
                reason: format!("mcp.json 解析失败: {e}"),
                hint: "修复 JSON 或备份后删除该文件".to_owned(),
            })?;
            Ok(Some(v))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CoreError::Io(e)),
    }
}

/// 将单条 server 渲染为 `mcpServers[name]` 的 config(含 `${VAR}` 替换)。
///
/// 缺 secrets 时返回 `SecretsMissing`(与 `render_mcp` 一致,用于下发)。
pub fn render_server_entry(
    srv: &McpServer,
    secrets: &HashMap<String, String>,
) -> Result<Value, CoreError> {
    for key in &srv.secret_keys {
        if !secrets.contains_key(key) {
            return Err(CoreError::SecretsMissing {
                key: key.clone(),
                template: format!("mcp/servers/{}.json", srv.name),
                hint: format!("在 secrets.env 里补 `{key}` 后重试下发"),
            });
        }
    }
    Ok(substitute_value(srv.config.clone(), secrets))
}

/// 用于状态比对的期望 config(尽力替换 secrets,缺失占位符原样保留)。
pub fn expected_server_entry_for_compare(
    srv: &McpServer,
    secrets: &HashMap<String, String>,
) -> Value {
    substitute_value(srv.config.clone(), secrets)
}

/// 判定单条 server 是否已同步到 `mcp.json` 的 `mcpServers`(key + 值一致)。
pub fn mcp_server_sync_state(
    mcp_json_path: &Utf8Path,
    server_name: &str,
    expected_entry: &Value,
) -> McpSyncState {
    let root = match read_mcp_json(mcp_json_path) {
        Ok(Some(v)) => v,
        Ok(None) => return McpSyncState::Unlinked,
        Err(_) => return McpSyncState::Broken,
    };
    let Some(actual) = root
        .get("mcpServers")
        .and_then(|v| v.as_object())
        .and_then(|m| m.get(server_name))
    else {
        return McpSyncState::Unlinked;
    };
    if actual == expected_entry {
        McpSyncState::Linked
    } else {
        McpSyncState::WrongValue
    }
}

/// 向 `mcp.json` 的 `mcpServers` 写入/更新单条(保留其它 server 与顶层字段)。
pub fn upsert_mcp_server_entry(
    mcp_json_path: &Utf8Path,
    server_name: &str,
    entry: Value,
) -> Result<(), CoreError> {
    let mut root = match read_mcp_json(mcp_json_path)? {
        Some(v) => v,
        None => serde_json::json!({ "mcpServers": {} }),
    };
    let obj = root
        .as_object_mut()
        .ok_or_else(|| CoreError::TemplateRender {
            template: mcp_json_path.as_str().to_owned(),
            reason: "mcp.json 顶层不是 object".to_owned(),
            hint: "把 mcp.json 顶层改为 object".to_owned(),
        })?;
    let servers = obj
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let servers_obj = servers
        .as_object_mut()
        .ok_or_else(|| CoreError::TemplateRender {
            template: mcp_json_path.as_str().to_owned(),
            reason: "`mcpServers` 不是 object".to_owned(),
            hint: "把 `mcpServers` 改为 object".to_owned(),
        })?;
    servers_obj.insert(server_name.to_owned(), entry);
    atomic_write_json(mcp_json_path, &root)
}

/// 从 `mcp.json` 的 `mcpServers` 移除单条 key(文件不存在则 noop)。
pub fn remove_mcp_server_entry(
    mcp_json_path: &Utf8Path,
    server_name: &str,
) -> Result<(), CoreError> {
    let Some(mut root) = read_mcp_json(mcp_json_path)? else {
        return Ok(());
    };
    if let Some(servers) = root.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        servers.remove(server_name);
    }
    atomic_write_json(mcp_json_path, &root)
}

/// 读已有 `mcp.json` → 决定是否合并。
///
/// - 文件**不存在** → 返回 `new_servers` 直接包成 `{ "mcpServers": ... }` 顶层
/// - 文件存在,顶层含 `"managedBy": "agent-manager"` → 合并;`mcpServers` 字段整块
///   用新的替换,`mcpServers` 之外的字段保留
/// - 文件存在,**不**含 `managedBy == "agent-manager"` → 返回
///   `CoreError::Unmanaged`,**不**写盘(PRD §8.1 硬约束 #5)
pub fn merge_with_existing(dest: &Utf8Path, new_servers: Value) -> Result<Value, CoreError> {
    // new_servers 期望是 Object 且 key 全是 server_name(由 render_mcp 构造);
    // 但 merge_with_existing 自身也允许 new_servers 已经是整份 payload(测试用)。

    let existing = match std::fs::read_to_string(dest.as_std_path()) {
        Ok(s) => {
            Some(
                serde_json::from_str::<Value>(&s).map_err(|e| CoreError::TemplateRender {
                    template: dest.as_str().to_owned(),
                    reason: format!("已有 mcp.json 解析失败: {e}"),
                    hint: "把已有文件备份后删除,再重跑".to_owned(),
                })?,
            )
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(CoreError::Io(e)),
    };

    // 顶层结构:由 new_servers 是否带 "mcpServers" key 决定 —
    // 内部调用(render_mcp):new_servers = `{ name: cfg, ... }`,需包成
    // `{ "mcpServers": {...}, "managedBy": "agent-manager" }`
    // 测试/外部调用:new_servers = 完整 payload(含 mcpServers),直接加 managedBy
    let new_payload = if new_servers
        .as_object()
        .is_some_and(|m| m.contains_key("mcpServers"))
    {
        let mut obj = new_servers.as_object().cloned().unwrap_or_default();
        obj.insert(
            MANAGED_BY_KEY.to_owned(),
            Value::String(MANAGED_BY_VALUE.to_owned()),
        );
        Value::Object(obj)
    } else {
        // 内部分支:把 name→config 字典包成 mcpServers
        let mut m = serde_json::Map::new();
        m.insert("mcpServers".to_owned(), new_servers);
        m.insert(
            MANAGED_BY_KEY.to_owned(),
            Value::String(MANAGED_BY_VALUE.to_owned()),
        );
        Value::Object(m)
    };

    let Some(existing) = existing else {
        // 全新文件:直接返回
        return Ok(new_payload);
    };

    // 检查 managedBy 字段
    let managed = existing
        .get(MANAGED_BY_KEY)
        .and_then(Value::as_str)
        .map(str::to_owned);
    match managed.as_deref() {
        Some(MANAGED_BY_VALUE) => {
            // 合并:mcpServers 字段用新的整块替换,其它顶层字段保留
            let mut obj =
                existing
                    .as_object()
                    .cloned()
                    .ok_or_else(|| CoreError::TemplateRender {
                        template: dest.as_str().to_owned(),
                        reason: "已有 mcp.json 顶层不是 object".to_owned(),
                        hint: "本工具要求 mcp.json 顶层为 object".to_owned(),
                    })?;
            obj.insert(
                "mcpServers".to_owned(),
                new_payload
                    .get("mcpServers")
                    .cloned()
                    .unwrap_or(Value::Object(serde_json::Map::new())),
            );
            obj.insert(
                MANAGED_BY_KEY.to_owned(),
                Value::String(MANAGED_BY_VALUE.to_owned()),
            );
            Ok(Value::Object(obj))
        }
        Some(other) => {
            // 别的工具写的 managedBy → 也视为外部,拒绝
            Err(unmanaged_err(dest, &format!("managedBy={other}")))
        }
        None => Err(unmanaged_err(dest, "顶层无 `managedBy` 字段")),
    }
}

fn unmanaged_err(path: &Utf8Path, reason: &str) -> CoreError {
    CoreError::TemplateRender {
        template: path.as_str().to_owned(),
        reason: format!("目标 mcp.json 是用户手写或外部工具产物 ({reason}),本工具拒绝覆盖"),
        hint: format!(
            "二选一:(a) `mv {path} {path}.bak` 后重跑;(b) 在该 mcp.json 顶层加 `\"managedBy\": \"agent-manager\"` 表示授权本工具接管"
        ),
    }
}

/// 原子写 JSON:写 `<dest>.tmp` → `rename` 到 `<dest>`。
/// rename 失败时尝试删 tmp 并把原文件回滚(若之前存在)。**不**改 0600 权限
/// 以外的 fs 元数据(PRD §8.1 硬约束:不擅改用户文件 → 沿用原权限)。
pub(crate) fn atomic_write_json(dest: &Utf8Path, payload: &Value) -> Result<(), CoreError> {
    use tokio::runtime::Handle;
    crate::paths::ensure_parent_dir(dest)?;
    let serialized = serde_json::to_string_pretty(payload).map_err(CoreError::Json)?;

    let tmp = dest.with_extension("json.tmp");

    // 同步写 tmp(block_on 或直接在 sync fs;Phase 1 用 std::fs + 兜底 rename)
    if let Err(e) = std::fs::write(tmp.as_std_path(), serialized.as_bytes()) {
        return Err(CoreError::Io(e));
    }

    // 兜底:尽量同步 rename;若失败回滚 tmp。
    if let Err(e) = std::fs::rename(tmp.as_std_path(), dest.as_std_path()) {
        let _ = std::fs::remove_file(tmp.as_std_path());
        return Err(CoreError::Io(e));
    }

    // 上面在 sync 上下文跑;tokio Handle 仅作 hint(无实际 await)。
    let _ = Handle::try_current();
    Ok(())
}

// ── 单元测试 ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::exit_code;
    use std::fs;
    use tempfile::TempDir;

    // ─── 辅助:写一个临时 server.json 文件 ──────────────────────────

    fn write_server(dir: &Utf8Path, name: &str, body: &str) -> Utf8PathBuf {
        let p = dir.join(format!("{name}.json"));
        fs::write(p.as_std_path(), body).unwrap();
        p
    }

    // ─── T0: 原子写 JSON 自动建父目录(平台 mcp.json) ───────────────

    #[test]
    fn atomic_write_json_creates_missing_parent_dir() {
        let tmp = TempDir::new().unwrap();
        let dest = Utf8PathBuf::from_path_buf(tmp.path().join(".cursor/mcp.json")).unwrap();
        assert!(!dest.parent().unwrap().exists());
        atomic_write_json(&dest, &serde_json::json!({ "mcpServers": {} })).unwrap();
        assert!(dest.is_file());
    }

    // ─── T1: 解析有效 server JSON ─────────────────────────────────

    #[test]
    fn parse_server_extracts_name_transport_config_and_secret_keys() {
        let tmp = TempDir::new().unwrap();
        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();

        let body = r#"{
            "name": "minio",
            "transport": "stdio",
            "enabled": true,
            "config": {
                "command": "docker",
                "args": ["run", "-e", "MINIO_ENDPOINT"],
                "env": {
                    "MINIO_ENDPOINT": "${MINIO_ENDPOINT}",
                    "MINIO_ACCESS_KEY": "${MINIO_ACCESS_KEY}"
                }
            }
        }"#;
        let path = write_server(&dir, "minio", body);
        let srv = parse_server(&path).expect("parse ok");

        assert_eq!(srv.name, "minio");
        assert_eq!(srv.transport, McpTransport::Stdio);
        assert!(srv.enabled);
        assert!(srv.config.get("command").is_some());
        assert_eq!(srv.secret_keys.len(), 2);
        assert!(srv.secret_keys.contains(&"MINIO_ENDPOINT".to_string()));
        assert!(srv.secret_keys.contains(&"MINIO_ACCESS_KEY".to_string()));
    }

    // ─── T2: 合并 2 条 server 输出 mcpServers 含 2 项 ─────────────

    #[test]
    fn render_mcp_merges_two_enabled_servers_into_mcp_servers() {
        let tmp = TempDir::new().unwrap();
        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();

        let mut secrets = HashMap::new();
        secrets.insert("MINIO_ENDPOINT".into(), "https://minio.example.com".into());
        secrets.insert("OBSIDIAN_API_KEY".into(), "obs-xyz".into());

        let minio = McpServer {
            project_id: 0,
            name: "minio".into(),
            transport: McpTransport::Stdio,
            config: serde_json::json!({
                "command": "docker",
                "env": {"ENDPOINT": "${MINIO_ENDPOINT}"}
            }),
            secret_keys: vec!["MINIO_ENDPOINT".into()],
            enabled: true,
        };
        let obsidian = McpServer {
            project_id: 0,
            name: "obsidian".into(),
            transport: McpTransport::Stdio,
            config: serde_json::json!({
                "command": "uvx",
                "env": {"API_KEY": "${OBSIDIAN_API_KEY}"}
            }),
            secret_keys: vec!["OBSIDIAN_API_KEY".into()],
            enabled: true,
        };

        let out =
            render_mcp(PlatformId::Cursor, &dir, &[minio, obsidian], &secrets).expect("render ok");

        assert!(out.exists(), "mcp.json 应当被写入");
        let parsed: Value =
            serde_json::from_str(&fs::read_to_string(out.as_std_path()).unwrap()).unwrap();

        let mcp = parsed.get("mcpServers").expect("顶层 mcpServers 字段");
        let mcp = mcp.as_object().unwrap();
        assert_eq!(mcp.len(), 2, "2 条 server 都该进 mcpServers");
        assert!(mcp.contains_key("minio"));
        assert!(mcp.contains_key("obsidian"));
        // 占位符已被替换
        let minio_endpoint = mcp["minio"]["env"]["ENDPOINT"].as_str().unwrap();
        assert_eq!(minio_endpoint, "https://minio.example.com");
        let obs_key = mcp["obsidian"]["env"]["API_KEY"].as_str().unwrap();
        assert_eq!(obs_key, "obs-xyz");
        // managedBy 元字段
        assert_eq!(
            parsed.get("managedBy").and_then(Value::as_str),
            Some("agent-manager")
        );
    }

    // ─── T3: 缺 ${MINIO_ENDPOINT} → Err(且退出码 4) ───────────────

    #[test]
    fn render_mcp_missing_secret_returns_secrets_missing_with_exit_code_4() {
        let tmp = TempDir::new().unwrap();
        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();

        let secrets = HashMap::new(); // 故意空,什么都不配
        let minio = McpServer {
            project_id: 0,
            name: "minio".into(),
            transport: McpTransport::Stdio,
            config: serde_json::json!({
                "command": "docker",
                "env": {"ENDPOINT": "${MINIO_ENDPOINT}"}
            }),
            secret_keys: vec!["MINIO_ENDPOINT".into()],
            enabled: true,
        };

        let err = render_mcp(
            PlatformId::Cursor,
            &dir,
            std::slice::from_ref(&minio),
            &secrets,
        )
        .expect_err("必须失败");

        // 退出码 4(PRD §6.1)
        assert_eq!(err.exit_code(), exit_code::SECRETS_MISSING);
        // Display 不含空字符串(占位符**原样**保留,提示哪个 key)
        let s = err.to_string();
        assert!(s.contains("MINIO_ENDPOINT"), "错误信息必须含 key 名: {s}");
        assert!(s.contains("minio"), "错误信息必须含 server 名: {s}");
        assert!(!s.contains("\"\""), "Display 绝不能含空字符串: {s}");

        // 缺变量时**不**写盘
        assert!(
            !dir.join("mcp.json").exists(),
            "缺变量必须不写 mcp.json(避免半成品)"
        );
        assert!(!dir.join("mcp.json.tmp").exists(), "缺变量时也不应留 tmp");
    }

    // ─── T4: 原子写后读回 OK ──────────────────────────────────────

    #[test]
    fn atomic_write_then_readback_succeeds() {
        let tmp = TempDir::new().unwrap();
        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();

        let payload = serde_json::json!({
            "mcpServers": {
                "fetch": {"command": "uvx", "args": ["mcp-server-fetch"]}
            },
            "managedBy": "agent-manager"
        });
        let dest = dir.join("mcp.json");
        atomic_write_json(&dest, &payload).expect("write ok");

        // 读回 = payload
        let s = fs::read_to_string(dest.as_std_path()).unwrap();
        let back: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(back, payload);
        // tmp 不应残留
        assert!(!dir.join("mcp.json.tmp").exists());
    }

    // ─── T5: 已有 mcp.json 不含 managedBy → 不覆盖(返回 Err Unmanaged) ─

    #[test]
    fn merge_with_existing_refuses_unmanaged_file() {
        let tmp = TempDir::new().unwrap();
        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();

        // 模拟用户手写的 mcp.json(无 managedBy)
        let user_written = serde_json::json!({
            "mcpServers": {
                "user-foo": {"command": "echo", "args": ["hi"]}
            }
        });
        let dest = dir.join("mcp.json");
        fs::write(
            dest.as_std_path(),
            serde_json::to_string_pretty(&user_written).unwrap(),
        )
        .unwrap();

        // 我们要写入的新 server 字典
        let new_servers = serde_json::json!({
            "minio": {"command": "docker", "args": []}
        });
        let err =
            merge_with_existing(&dest, new_servers).expect_err("必须拒绝覆盖无 managedBy 的文件");

        // 错误信息:是 TemplateRender 分类(退出码 3),hint 提示备份或加 managedBy
        let s = err.to_string();
        assert!(
            s.contains("managedBy") || s.contains("用户手写"),
            "错误必须解释原因: {s}"
        );

        // 文件**没有**被改
        let after = fs::read_to_string(dest.as_std_path()).unwrap();
        assert_eq!(after, serde_json::to_string_pretty(&user_written).unwrap());
    }

    // ─── 补:已 managed 的旧 mcp.json + 新 server → mcpServers 整块替换,其它字段保留

    #[test]
    fn merge_with_existing_managed_file_replaces_mcp_servers_block_and_preserves_others() {
        let tmp = TempDir::new().unwrap();
        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();

        let existing = serde_json::json!({
            "managedBy": "agent-manager",
            "mcpServers": {"old": {"command": "x"}},
            "x-note": "用户给本工具加的旁注"
        });
        let dest = dir.join("mcp.json");
        fs::write(
            dest.as_std_path(),
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();

        let new_payload = serde_json::json!({
            "mcpServers": {"new1": {"command": "a"}, "new2": {"command": "b"}}
        });
        let merged = merge_with_existing(&dest, new_payload).expect("merged ok");
        let obj = merged.as_object().unwrap();

        // mcpServers 整块换
        assert!(obj["mcpServers"].get("old").is_none());
        assert!(obj["mcpServers"].get("new1").is_some());
        assert!(obj["mcpServers"].get("new2").is_some());
        // 其它字段保留
        assert_eq!(obj["x-note"], "用户给本工具加的旁注");
        // managedBy 保留
        assert_eq!(obj["managedBy"], "agent-manager");
    }

    // ─── 补:disabled server 不会被渲染 ─────────────────────────────

    #[test]
    fn render_mcp_skips_disabled_servers() {
        let tmp = TempDir::new().unwrap();
        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();

        let secrets = HashMap::new();
        let on = McpServer {
            project_id: 0,
            name: "on".into(),
            transport: McpTransport::Stdio,
            config: serde_json::json!({"command": "a"}),
            secret_keys: vec![],
            enabled: true,
        };
        let off = McpServer {
            project_id: 0,
            name: "off".into(),
            transport: McpTransport::Stdio,
            config: serde_json::json!({"command": "b"}),
            secret_keys: vec![],
            enabled: false,
        };

        let out = render_mcp(PlatformId::Cursor, &dir, &[on, off], &secrets).unwrap();
        let parsed: Value =
            serde_json::from_str(&fs::read_to_string(out.as_std_path()).unwrap()).unwrap();
        let mcp = parsed["mcpServers"].as_object().unwrap();
        assert_eq!(mcp.len(), 1);
        assert!(mcp.contains_key("on"));
        assert!(!mcp.contains_key("off"));
    }

    // ─── 补:占位符提取 — 嵌套 + 重复 key 去重 ──────────────────────

    #[test]
    fn collect_secret_keys_handles_nested_and_dedup() {
        let v = serde_json::json!({
            "headers": {
                "Authorization": "Bearer ${TOKEN}",
                "X-Trace": "${TOKEN}-${OTHER}"
            },
            "args": ["--token=${TOKEN}"]
        });
        let keys = collect_secret_keys(&v);
        // TOKEN 出现 3 次,只 push 一次;OTHER 排第二
        assert_eq!(keys, vec!["TOKEN".to_string(), "OTHER".to_string()]);
    }

    // ─── 补:parse_server 从文件名兜底 name ─────────────────────────

    #[test]
    fn parse_server_falls_back_to_file_stem_for_name() {
        let tmp = TempDir::new().unwrap();
        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let body = r#"{"config": {"command": "echo"}}"#;
        let path = write_server(&dir, "fetch", body);
        let srv = parse_server(&path).unwrap();
        assert_eq!(srv.name, "fetch");
        assert_eq!(srv.transport, McpTransport::Stdio);
    }

    // ─── 补:parse_server url 字段 → transport 推断 http ───────────

    #[test]
    fn parse_server_infers_http_from_url_field() {
        let tmp = TempDir::new().unwrap();
        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let body = r#"{
            "name": "context7",
            "config": {
                "url": "https://mcp.context7.com/mcp",
                "headers": {"X-Api-Key": "${CONTEXT7_API_KEY}"}
            }
        }"#;
        let path = write_server(&dir, "context7", body);
        let srv = parse_server(&path).unwrap();
        assert_eq!(srv.transport, McpTransport::Http);
        assert_eq!(srv.secret_keys, vec!["CONTEXT7_API_KEY".to_string()]);
    }

    // ─── migrate_from_template ──────────────────────────────────

    /// 写一个 `mcp/servers/<name>.json`,body 是裸 JSON 字符串。
    /// (与文件内已有的 `write_server` 同语义,这里 inline 写一份,避免依赖顺序)
    fn write_server_inline(dir: &Utf8Path, name: &str, body: &str) -> Utf8PathBuf {
        use std::fs;
        let servers = dir.join("mcp").join("servers");
        fs::create_dir_all(servers.as_std_path()).unwrap();
        let path = servers.join(format!("{name}.json"));
        fs::write(path.as_std_path(), body).unwrap();
        path
    }

    /// 写一份最小可用的整份 mcp.json 模板到 `<tmp>/template.json`。
    fn write_template(dir: &Utf8Path, body: &str) -> Utf8PathBuf {
        use std::fs;
        let path = dir.join("template.json");
        fs::write(path.as_std_path(), body).unwrap();
        path
    }

    #[test]
    fn migrate_from_template_writes_all_servers() {
        let tmp = TempDir::new().unwrap();
        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        // 用 3 条最常见形态的 server(覆盖 type=stdio / 无 type 有 command / 无 type 有 url)
        let template_body = r#"{
            "mcpServers": {
                "fetch":   {"command": "uvx", "args": ["mcp-server-fetch"]},
                "aistor-minio": {"command": "docker", "args": ["run"], "env": {"MINIO_ENDPOINT": "${MINIO_ENDPOINT}"}},
                "context7": {"url": "https://mcp.context7.com/mcp", "headers": {"CONTEXT7_API_KEY": "${CONTEXT7_API_KEY}"}}
            }
        }"#;
        let template = write_template(&dir, template_body);
        let dest = dir.join("mcp").join("servers");

        let report = migrate_from_template(&template, &dest).expect("migrate ok");

        // 3 条全写
        assert_eq!(report.written.len(), 3, "written: {:?}", report.written);
        assert!(report.written.contains(&"fetch".to_string()));
        assert!(report.written.contains(&"aistor-minio".to_string()));
        assert!(report.written.contains(&"context7".to_string()));
        assert!(report.skipped.is_empty(), "skipped: {:?}", report.skipped);
        assert!(report.errors.is_empty(), "errors: {:?}", report.errors);

        // 3 个文件落地
        assert!(dest.join("fetch.json").is_file());
        assert!(dest.join("aistor-minio.json").is_file());
        assert!(dest.join("context7.json").is_file());

        // fetch 写出来的内容:transport 推断 stdio,config 内**无** type 字段
        let fetch_body: Value = serde_json::from_str(
            &fs::read_to_string(dest.join("fetch.json").as_std_path()).unwrap(),
        )
        .unwrap();
        assert_eq!(fetch_body["name"], "fetch");
        assert_eq!(fetch_body["transport"], "stdio");
        assert_eq!(fetch_body["enabled"], true);
        assert_eq!(fetch_body["config"]["command"], "uvx");
        assert!(
            fetch_body["config"].get("type").is_none(),
            "config 不该有 type 字段"
        );

        // context7 推断 http
        let ctx_body: Value = serde_json::from_str(
            &fs::read_to_string(dest.join("context7.json").as_std_path()).unwrap(),
        )
        .unwrap();
        assert_eq!(ctx_body["transport"], "http");
    }

    #[test]
    fn migrate_from_template_handles_type_field_rename() {
        let tmp = TempDir::new().unwrap();
        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        // 模板里 dart 显式给 type=stdio,jenkins 给 type=http
        let template_body = r#"{
            "mcpServers": {
                "dart":    {"type": "stdio", "command": "dart mcp-server", "args": []},
                "jenkins": {"type": "http",  "url": "https://example.com/mcp", "headers": {}}
            }
        }"#;
        let template = write_template(&dir, template_body);
        let dest = dir.join("mcp").join("servers");

        let report = migrate_from_template(&template, &dest).expect("migrate ok");
        assert_eq!(report.written.len(), 2);
        assert!(report.errors.is_empty(), "errors: {:?}", report.errors);

        // dart:transport=stdio,config 内**无** type 字段(rename 成功)
        let dart_body: Value = serde_json::from_str(
            &fs::read_to_string(dest.join("dart.json").as_std_path()).unwrap(),
        )
        .unwrap();
        assert_eq!(dart_body["transport"], "stdio");
        assert!(
            dart_body["config"].get("type").is_none(),
            "type 字段已被剔除"
        );

        // jenkins:transport=http
        let jenkins_body: Value = serde_json::from_str(
            &fs::read_to_string(dest.join("jenkins.json").as_std_path()).unwrap(),
        )
        .unwrap();
        assert_eq!(jenkins_body["transport"], "http");
        assert!(jenkins_body["config"].get("type").is_none());
    }

    #[test]
    fn migrate_from_template_infers_transport_from_url() {
        let tmp = TempDir::new().unwrap();
        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        // 无 type,但有 url → 推断 http
        let template_body = r#"{
            "mcpServers": {
                "remote-svc": {"url": "https://example.com/mcp", "headers": {}}
            }
        }"#;
        let template = write_template(&dir, template_body);
        let dest = dir.join("mcp").join("servers");

        let report = migrate_from_template(&template, &dest).expect("migrate ok");
        assert_eq!(report.written, vec!["remote-svc".to_string()]);
        let body: Value = serde_json::from_str(
            &fs::read_to_string(dest.join("remote-svc.json").as_std_path()).unwrap(),
        )
        .unwrap();
        assert_eq!(body["transport"], "http");
    }

    #[test]
    fn migrate_from_template_skips_existing_files() {
        let tmp = TempDir::new().unwrap();
        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let template_body = r#"{
            "mcpServers": {
                "fetch":   {"command": "uvx"},
                "context7": {"url": "https://x.com/mcp"}
            }
        }"#;
        let template = write_template(&dir, template_body);
        let dest = dir.join("mcp").join("servers");

        // 预放一个 fetch.json(模拟"用户已迁移过 + 改过 enabled")
        let pre_existing = write_server_inline(
            &dir,
            "fetch",
            r#"{"name":"fetch","transport":"stdio","enabled":false,"config":{"command":"uvx","user_edited":true}}"#,
        );
        let pre_content = fs::read_to_string(pre_existing.as_std_path()).unwrap();

        let report = migrate_from_template(&template, &dest).expect("migrate ok");

        // fetch skipped,context7 写
        assert_eq!(report.written, vec!["context7".to_string()]);
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].0, "fetch");
        assert_eq!(report.skipped[0].1, "already exists");
        assert!(report.errors.is_empty());

        // fetch.json 内容**未**被覆盖(用户改的 enabled=false 还在)
        let after_content = fs::read_to_string(pre_existing.as_std_path()).unwrap();
        assert_eq!(
            pre_content, after_content,
            "skip 必须保证已存在文件字节级未变"
        );
    }

    #[test]
    fn migrate_from_template_errors_on_missing_mcp_servers_key() {
        let tmp = TempDir::new().unwrap();
        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let template = write_template(&dir, r#"{"foo": 1, "bar": 2}"#);
        let dest = dir.join("mcp").join("servers");

        let err = migrate_from_template(&template, &dest).unwrap_err();
        // 应是 TemplateRender 错(reason 提及 "mcpServers")
        match err {
            CoreError::TemplateRender { reason, .. } => {
                assert!(
                    reason.contains("mcpServers"),
                    "reason 应提示缺 mcpServers 字段,实际:`{reason}`"
                );
            }
            other => panic!("应是 TemplateRender,实际:{other:?}"),
        }
        // dest 目录已建好(用户可原地修模板重跑),但**不**应有任何 server 文件
        assert!(dest.is_dir(), "err 时 dest 目录应已创建");
        let entries: Vec<_> = std::fs::read_dir(dest.as_std_path())
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert!(entries.is_empty(), "err 时不应写任何 server 文件");
    }

    #[test]
    fn migrate_from_template_errors_on_unknown_type() {
        let tmp = TempDir::new().unwrap();
        let dir = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        // 1 条 type="ws"(未知),1 条正常 → 前者进 errors,后者 written
        let template_body = r#"{
            "mcpServers": {
                "future-ws": {"type": "ws", "url": "wss://x"},
                "fetch":     {"command": "uvx"}
            }
        }"#;
        let template = write_template(&dir, template_body);
        let dest = dir.join("mcp").join("servers");

        let report = migrate_from_template(&template, &dest).expect("整体 Ok,部分进 errors");
        assert_eq!(report.written, vec!["fetch".to_string()]);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].0, "future-ws");
        assert!(report.errors[0].1.contains("未知 type"));
        assert!(report.skipped.is_empty());
        // future-ws.json 不应存在
        assert!(!dest.join("future-ws.json").exists());
    }

    #[test]
    fn mcp_upsert_remove_and_sync_state() {
        let tmp = TempDir::new().unwrap();
        let mcp_json = Utf8PathBuf::from_path_buf(tmp.path().join("mcp.json")).unwrap();
        let entry_a = serde_json::json!({"command": "uvx", "args": ["mcp-a"]});
        let entry_b = serde_json::json!({"url": "https://example.com/mcp"});

        assert_eq!(
            mcp_server_sync_state(&mcp_json, "svc-a", &entry_a),
            McpSyncState::Unlinked
        );

        upsert_mcp_server_entry(&mcp_json, "svc-a", entry_a.clone()).unwrap();
        upsert_mcp_server_entry(&mcp_json, "svc-b", entry_b.clone()).unwrap();

        assert_eq!(
            mcp_server_sync_state(&mcp_json, "svc-a", &entry_a),
            McpSyncState::Linked
        );
        assert_eq!(
            mcp_server_sync_state(&mcp_json, "svc-b", &entry_b),
            McpSyncState::Linked
        );

        let wrong = serde_json::json!({"command": "other"});
        assert_eq!(
            mcp_server_sync_state(&mcp_json, "svc-a", &wrong),
            McpSyncState::WrongValue
        );

        remove_mcp_server_entry(&mcp_json, "svc-a").unwrap();
        assert_eq!(
            mcp_server_sync_state(&mcp_json, "svc-a", &entry_a),
            McpSyncState::Unlinked
        );
        assert_eq!(
            mcp_server_sync_state(&mcp_json, "svc-b", &entry_b),
            McpSyncState::Linked,
            "移除一条 key 不应影响其它 server"
        );

        let raw = fs::read_to_string(mcp_json.as_std_path()).unwrap();
        let root: Value = serde_json::from_str(&raw).unwrap();
        assert!(root.get("svc-b").is_none(), "entry 应在 mcpServers 下");
        assert!(root["mcpServers"].get("svc-b").is_some());
    }

    #[test]
    fn mcp_sync_state_broken_on_invalid_json() {
        let tmp = TempDir::new().unwrap();
        let mcp_json = Utf8PathBuf::from_path_buf(tmp.path().join("bad.json")).unwrap();
        fs::write(mcp_json.as_std_path(), "not-json").unwrap();
        let entry = serde_json::json!({});
        assert_eq!(
            mcp_server_sync_state(&mcp_json, "x", &entry),
            McpSyncState::Broken
        );
    }
}
