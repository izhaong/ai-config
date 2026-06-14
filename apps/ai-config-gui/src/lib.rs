//! ai-config 图形界面(Tauri 2 + React 19 前端)。
//!
//! Phase 3 W9:核心业务接入。
//!
//! ## 暴露给前端的 Tauri command(14 个)
//!
//! | Command | 入参 | 出参 | 走 core/store 路径 |
//! |---|---|---|---|
//! | `cmd_doctor` | — | `DoctorSummary` | (占位,W10 接真值) |
//! | `cmd_list` | `project?` | `AssetList` | `source::scan_project_root`(项目) / 全局根 |
//! | `cmd_skill_deploy` | `name, project, to` | `String` | `link::link` |
//! | `cmd_skill_retract` | `name, project, from` | `String` | `link::unlink` |
//! | `cmd_rule_deploy` | 同上 | `String` | `link::link`(.mdc 后缀) |
//! | `cmd_rule_retract` | 同上 | `String` | `link::unlink` |
//! | `cmd_mcp_deploy` | 同上 | `String` | `mcp_json::upsert_server_on_platform` |
//! | `cmd_mcp_retract` | 同上 | `String` | `mcp_json::remove_server_on_platform` |
//! | `cmd_agent_deploy` | 同上 | `String` | `link::link`(保留原 ext) |
//! | `cmd_agent_retract` | 同上 | `String` | `link::unlink` |
//! | `cmd_projects_list` | — | `Vec<Project>` | `Store::projects().list()` |
//! | `cmd_projects_add` | `name, root_path` | `Project` | `Store::projects().add()` |
//! | `cmd_projects_remove` | `name` | `()` | `Store::projects().remove()` |
//!
//! ## 共享状态
//!
//! `AppState` 持有 `Arc<Store>`(SQLite 持久化)和 `default_root: Arc<RwLock<Utf8PathBuf>>`(资产根)。
//! 同步阻塞 IO 走 `tokio::task::spawn_blocking` 包裹,避免锁住 Tauri runtime。

use std::sync::{Arc, Mutex};

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::RwLock;

use ai_config_core::error::CoreError;
use ai_config_core::link::{self, LinkKind};
use ai_config_core::mcp_json;
use ai_config_core::model::{AssetKind, PlatformId, Project};
use ai_config_core::paths;
use ai_config_core::platform;
use ai_config_core::source;
use ai_config_core::sync::{agent_link_src, asset_dest_for_at_base};
use ai_config_core::template::McpSyncState;
use ai_config_store::Store;
use ai_config_watcher::{dedupe_roots, start_debounced, WatchRoots, WatcherHandle};

// ── 共享状态 ──────────────────────────────────────────────────────

/// Tauri 主进程长驻共享状态。
pub struct AppState {
    /// SQLite 持久化(W9 起:projects 表)
    pub store: Arc<Store>,
    /// 默认资产根(`~/.ai-config/`;启动时自动创建子目录)
    pub default_root: Arc<RwLock<Utf8PathBuf>>,
    /// 资产目录文件监听句柄(项目增删时重启)
    pub watcher: Mutex<Option<WatcherHandle>>,
}

impl AppState {
    fn new() -> Result<Self, String> {
        let store = Store::open().map_err(|e| format!("打开 store 失败: {e}"))?;
        let default_root = paths::discover_global_asset_root();
        tracing::info!("ai-config 资产根: {default_root}");
        Ok(Self {
            store: Arc::new(store),
            default_root: Arc::new(RwLock::new(default_root)),
            watcher: Mutex::new(None),
        })
    }
}

// ── 返回给前端的结构 ──────────────────────────────────────────────

/// 单条资产在前端的呈现(per-kind + per-platform 状态)。
#[derive(Debug, Serialize)]
struct AssetEntry {
    name: String,
    kind: AssetKind,
    /// 对 skill:来自 `SKILL.md` 第一个 `# title` 行 / frontmatter;其它类型空字符串。
    description: String,
    /// 源路径(本仓库或项目 `.ai-config/`),给详情面板用。
    source_path: String,
    /// per-platform 链接状态(4 平台键都存在)。
    states: std::collections::HashMap<PlatformId, LinkState>,
}

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum LinkState {
    /// 链接存在且指向 src
    Linked,
    /// dest 不存在
    Unlinked,
    /// dest 存在但不是链接,或源丢了(破损)
    Broken,
    /// 资产源文件不存在(本工具之前的链接成 dangling)— W9 暂未产出,
    /// 留给 W10 store 引入 targets 表后产出(W9 走 fs 推断暂只 3 态)
    #[allow(dead_code)]
    Missing,
}

#[derive(Debug, Serialize)]
struct AssetList {
    /// 按 kind 分组的扁平列表(前端按 `activeKind` filter)
    entries: Vec<AssetEntry>,
    /// 本次扫描汇总(给 statusbar)
    broken_links: u32,
}

#[derive(Debug, Serialize)]
struct DoctorSummary {
    broken: u64,
    wrong_source: u64,
    wrong_type: u64,
    missing_secrets: Vec<String>,
    unregistered_projects: Vec<String>,
    platform_capability_issues: Vec<PlatformIssue>,
    exit_code: u8,
}

#[derive(Debug, Serialize)]
struct PlatformIssue {
    platform: PlatformId,
    kind: String,
    reason: String,
}

// ── Tauri command:健康检查(占位,W10 接真值) ─────────────────────

/// W9 保留 W8 占位:不接 `lifecycle::run_doctor` 共享逻辑(46KB,refactor 估 1h,
/// 超出 W9 预算)。W10 第一项实装。
#[tauri::command]
async fn cmd_doctor(state: State<'_, AppState>) -> Result<DoctorSummary, String> {
    let _ = state; // 暂未消费
    Ok(DoctorSummary {
        broken: 0,
        wrong_source: 0,
        wrong_type: 0,
        missing_secrets: vec![],
        unregistered_projects: vec![],
        platform_capability_issues: sync_capability_issues(),
        exit_code: 0,
    })
}

fn sync_capability_issues() -> Vec<PlatformIssue> {
    let mut out = vec![];
    for plat in [
        PlatformId::Cursor,
        PlatformId::Codex,
        PlatformId::Claude,
        PlatformId::Hermes,
    ] {
        for asset in [
            AssetKind::Skill,
            AssetKind::Rule,
            AssetKind::Mcp,
            AssetKind::Agent,
        ] {
            if platform_supports(plat, asset) {
                continue;
            }
            let reason = capability_skip_reason(plat, asset);
            out.push(PlatformIssue {
                platform: plat,
                kind: kind_str(asset).to_string(),
                reason,
            });
        }
    }
    out
}

fn capability_skip_reason(plat: PlatformId, kind: AssetKind) -> String {
    match (plat, kind) {
        (PlatformId::Codex, AssetKind::Rule) => {
            "Codex 通过 AGENTS.md 间接引用 rules，不支持全局 symlink 下发".into()
        }
        (PlatformId::Hermes, AssetKind::Rule) => {
            "Hermes 仅从工作目录 `.cursor/rules/*.mdc` 加载，不支持 ~/.hermes 全局 rules".into()
        }
        _ => format!(
            "platform `{}` 不支持 asset kind `{}`",
            platform_label(plat),
            kind_str(kind)
        ),
    }
}

fn platform_label(p: PlatformId) -> &'static str {
    match p {
        PlatformId::Cursor => "cursor",
        PlatformId::Codex => "codex",
        PlatformId::Claude => "claude",
        PlatformId::Hermes => "hermes",
    }
}

fn kind_str(k: AssetKind) -> &'static str {
    match k {
        AssetKind::Skill => "skill",
        AssetKind::Rule => "rule",
        AssetKind::Mcp => "mcp",
        AssetKind::Agent => "agent",
    }
}

// ── Tauri command:资产列表(cmd_list) ─────────────────────────────

/// 扫资产源,返回每条 per-platform 链接状态。
///
/// `project` 为 `None` 或 `"user-global"` → 走默认根(本仓库);否则按名字查
/// `Store` 拿 `Project.root_path`,扫该项目的 `.ai-config/`,与默认根做 override 合并。
#[tauri::command]
async fn cmd_list(
    state: State<'_, AppState>,
    project: Option<String>,
) -> Result<AssetList, String> {
    // 委托给 `resolve_scope` — 与其它 command 走同一路径,避免实现漂移。
    let (default_root, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;

    // 2. 扫（项目作用域仅 `<repo>/.ai-config/`；user-global 仅 `~/.ai-config/`）
    let scan = scan_assets_for_scope(&default_root, &asset_root)?;

    // 3. 扁平成 (kind, name, src) 列表
    let mut entries: Vec<(AssetKind, String, Utf8PathBuf)> = Vec::new();
    for p in &scan.skills {
        if let Some(name) = p.parent().and_then(|p| p.file_name()) {
            entries.push((AssetKind::Skill, name.to_string(), p.clone()));
        }
    }
    for p in &scan.rules {
        if let Some(stem) = p.file_stem() {
            entries.push((AssetKind::Rule, stem.to_string(), p.clone()));
        }
    }
    if let Some(ref mcp_path) = scan.mcp_json {
        let asset_root = mcp_path
            .parent()
            .ok_or_else(|| "mcp.json 路径无效".to_string())?;
        let server_names = mcp_json::list_server_names(asset_root)
            .map_err(|e| format!("list_server_names 失败: {e}"))?;
        for server_name in server_names {
            entries.push((AssetKind::Mcp, server_name, mcp_path.clone()));
        }
    }
    for p in &scan.agents {
        let name = if p.is_dir() {
            p.file_name().map(|s| s.to_string())
        } else {
            p.file_stem().map(|s| s.to_string())
        };
        if let Some(name) = name {
            entries.push((AssetKind::Agent, name, p.clone()));
        }
    }

    // 4. 对每条 × 4 平台算 dest + 状态
    let mut out: Vec<AssetEntry> = Vec::with_capacity(entries.len());
    let mut broken_total: u32 = 0;
    for (kind, name, src) in entries {
        let description = if kind == AssetKind::Mcp {
            let asset_root = src.parent().unwrap_or(&src);
            mcp_json::get_server_config(asset_root, &name)
                .ok()
                .flatten()
                .map(|cfg| mcp_json::server_transport_summary(&cfg))
                .unwrap_or_default()
        } else {
            parse_description(kind, &src)
        };
        let mut states = std::collections::HashMap::new();
        for plat in [
            PlatformId::Cursor,
            PlatformId::Codex,
            PlatformId::Claude,
            PlatformId::Hermes,
        ] {
            if !platform_supports(plat, kind) {
                states.insert(plat, LinkState::Unlinked);
                continue;
            }
            let st = if kind == AssetKind::Mcp {
                let asset_root = src.parent().unwrap_or(&src);
                mcp_link_state(plat, asset_root, &name, &deploy_base)
            } else {
                let dest = match compute_dest(plat, kind, &name, &src, &deploy_base) {
                    Some(d) => d,
                    None => {
                        states.insert(plat, LinkState::Unlinked);
                        continue;
                    }
                };
                let expected_src = match kind {
                    AssetKind::Skill => skill_link_src(&src),
                    AssetKind::Agent => agent_link_src(&src),
                    _ => src.clone(),
                };
                match link::check(&dest, &expected_src) {
                    link::LinkHealth::Linked { .. } => LinkState::Linked,
                    link::LinkHealth::Broken { .. } => LinkState::Broken,
                    link::LinkHealth::WrongSource { .. } | link::LinkHealth::WrongType { .. } => {
                        LinkState::Broken
                    }
                }
            };
            if st == LinkState::Broken {
                broken_total += 1;
            }
            states.insert(plat, st);
        }
        out.push(AssetEntry {
            name,
            kind,
            description,
            source_path: src.to_string(),
            states,
        });
    }

    Ok(AssetList {
        entries: out,
        broken_links: broken_total,
    })
}

/// 资产文件详情(供前端详情抽屉)。
#[derive(Debug, Serialize)]
struct AssetFileDetail {
    name: String,
    description: String,
    content: String,
    source_path: String,
    parent_path: String,
}

/// 单条 MCP server 是否已与平台 `mcp.json` 中同名 key 一致。
fn mcp_link_state(
    plat: PlatformId,
    asset_root: &Utf8Path,
    server_name: &str,
    deploy_base: &Utf8Path,
) -> LinkState {
    match mcp_json::mcp_server_sync_state_for_platform_at(
        asset_root,
        server_name,
        plat,
        deploy_base,
    ) {
        McpSyncState::Linked => LinkState::Linked,
        McpSyncState::Unlinked => LinkState::Unlinked,
        McpSyncState::WrongValue | McpSyncState::Broken => LinkState::Broken,
    }
}

/// Agent 目录优先读 `AGENT.md`,否则目录内首个 `.md`。
fn agent_edit_path(src: &Utf8Path) -> Option<Utf8PathBuf> {
    if !src.is_dir() {
        return None;
    }
    let agent_md = src.join("AGENT.md");
    if agent_md.is_file() {
        return Some(agent_md);
    }
    let entries = std::fs::read_dir(src.as_std_path()).ok()?;
    for entry in entries.flatten() {
        let path = Utf8PathBuf::from_path_buf(entry.path()).ok()?;
        if path.extension().map(|e| e == "md").unwrap_or(false) {
            return Some(path);
        }
    }
    None
}

/// Agent 源路径 → 实际读取的 markdown 文件(单文件或目录内主文件)。
fn agent_read_path(src: &Utf8Path) -> Utf8PathBuf {
    if src.is_dir() {
        agent_edit_path(src).unwrap_or_else(|| src.join("AGENT.md"))
    } else {
        src.to_path_buf()
    }
}

/// skill 下发目标为整个目录(`skills/<name>/`),非单文件 `SKILL.md`。
fn skill_link_src(skill_md: &Utf8Path) -> Utf8PathBuf {
    skill_md
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| skill_md.to_path_buf())
}

/// 从 SKILL.md frontmatter / 正文抽 name、description。
fn parse_skill_meta(content: &str) -> (Option<String>, String) {
    let mut name = None;
    let mut description = String::new();
    if content.starts_with("---") {
        let after_first = content[3..].trim_start_matches('\n');
        if let Some(end) = after_first.find("\n---") {
            for line in after_first[..end].lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("name:") {
                    name = Some(rest.trim().trim_matches(['"', '\'']).to_string());
                } else if let Some(rest) = line.strip_prefix("description:") {
                    let d = rest.trim().trim_matches(['"', '\'']).to_string();
                    if d == ">" || d == "|" {
                        continue;
                    }
                    if !d.is_empty() {
                        description = d.chars().take(200).collect();
                    }
                } else if description.is_empty() && line.starts_with('>') {
                    description = line
                        .trim_start_matches('>')
                        .trim()
                        .chars()
                        .take(200)
                        .collect();
                }
            }
        }
    }
    if description.is_empty() {
        for line in content.lines() {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("# ") {
                description = rest
                    .chars()
                    .take(200)
                    .collect::<String>()
                    .trim()
                    .to_string();
                break;
            }
        }
    }
    (name, description)
}

/// 从 SKILL.md / mcp.json 等源文件抽"一句话描述"。
///
/// - Skill:frontmatter `description:` 或 H1 回退
/// - Rule:frontmatter `description:` 或 H1 回退(与 skill 同解析)
/// - Mcp:读 mcp server JSON 的 `description` 字段(可选)
/// - Agent:frontmatter `description:` 或 H1 回退(与 skill 同解析)
fn parse_description(kind: AssetKind, src: &Utf8Path) -> String {
    match kind {
        AssetKind::Skill => std::fs::read_to_string(src)
            .ok()
            .map(|c| parse_skill_meta(&c).1)
            .unwrap_or_default(),
        AssetKind::Rule => std::fs::read_to_string(src)
            .ok()
            .map(|c| parse_skill_meta(&c).1)
            .unwrap_or_default(),
        AssetKind::Mcp => std::fs::read_to_string(src)
            .ok()
            .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
            .and_then(|v| {
                v.get("description")
                    .and_then(|d| d.as_str())
                    .map(|s| s.chars().take(80).collect::<String>())
            })
            .unwrap_or_default(),
        AssetKind::Agent => std::fs::read_to_string(agent_read_path(src))
            .ok()
            .map(|c| parse_skill_meta(&c).1)
            .unwrap_or_default(),
    }
}

/// 4 平台 × 4 类资产能力矩阵;与 `PlatformAdapter::supports` 同源。
fn platform_supports(plat: PlatformId, kind: AssetKind) -> bool {
    platform::for_id(plat)
        .map(|a| a.supports(kind))
        .unwrap_or(false)
}

/// 算一条资产在某个平台的目标路径(委托 `core::sync::asset_dest_for_at_base`)。
fn compute_dest(
    plat: PlatformId,
    kind: AssetKind,
    name: &str,
    src: &Utf8Path,
    deploy_base: &Utf8Path,
) -> Option<Utf8PathBuf> {
    asset_dest_for_at_base(plat, kind, name, src, deploy_base)
}

// ── Tauri command:4 类资产单条 deploy / retract ───────────────────

/// skill deploy:在目标平台建 symlink(链整个目录)。
#[tauri::command]
async fn cmd_skill_deploy(
    state: State<'_, AppState>,
    name: String,
    project: Option<String>,
    to: String,
) -> Result<String, String> {
    let (default_root, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let plat = parse_plat(&to)?;
    let skill_md = locate_source(&default_root, &asset_root, AssetKind::Skill, &name)?;
    let src = skill_link_src(&skill_md);
    let dest = compute_dest(plat, AssetKind::Skill, &name, &skill_md, &deploy_base)
        .ok_or_else(|| format!("无法算 skill `{name}` → {plat:?} 的 dest"))?;
    let dest_str = dest.to_string();
    tokio::task::spawn_blocking(move || link::link(&src, &dest, LinkKind::auto()))
        .await
        .map_err(|e| format!("spawn_blocking join: {e}"))?
        .map_err(|e| format!("link::link 失败: {e}"))?;
    Ok(format!("skill `{name}` → {plat:?} OK ({dest_str})"))
}

/// 读取单个 skill 的完整 SKILL.md。
#[tauri::command]
async fn cmd_skill_get(
    state: State<'_, AppState>,
    name: String,
    project: Option<String>,
) -> Result<AssetFileDetail, String> {
    let (default_root, asset_root, _deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let skill_md = locate_source(&default_root, &asset_root, AssetKind::Skill, &name)?;
    let content =
        std::fs::read_to_string(&skill_md).map_err(|e| format!("读取 SKILL.md 失败: {e}"))?;
    let (fm_name, description) = parse_skill_meta(&content);
    let parent_path = skill_md.parent().map(|p| p.to_string()).unwrap_or_default();
    Ok(AssetFileDetail {
        name: fm_name.unwrap_or(name),
        description,
        content,
        source_path: skill_md.to_string(),
        parent_path,
    })
}

/// 保存 skill 的 SKILL.md 正文。
#[tauri::command]
async fn cmd_skill_save(
    state: State<'_, AppState>,
    name: String,
    content: String,
    project: Option<String>,
) -> Result<String, String> {
    let (default_root, asset_root, _deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let skill_md = locate_source(&default_root, &asset_root, AssetKind::Skill, &name)?;
    let path = skill_md.to_string();
    tokio::task::spawn_blocking(move || std::fs::write(&path, &content))
        .await
        .map_err(|e| format!("spawn_blocking join: {e}"))?
        .map_err(|e| format!("写入 SKILL.md 失败: {e}"))?;
    Ok(format!("skill `{name}` 已保存"))
}

/// 删除整个 skill 目录(`skills/<name>/`)。
#[tauri::command]
async fn cmd_skill_delete(
    state: State<'_, AppState>,
    name: String,
    project: Option<String>,
) -> Result<String, String> {
    let (default_root, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    retract_all_platforms_best_effort(&default_root, &asset_root, &deploy_base, AssetKind::Skill, &name);
    let skill_md = locate_source(&default_root, &asset_root, AssetKind::Skill, &name)?;
    let skill_dir = skill_md
        .parent()
        .ok_or_else(|| format!("skill `{name}` 无父目录"))?
        .to_path_buf();
    let dir_str = skill_dir.to_string();
    tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&skill_dir))
        .await
        .map_err(|e| format!("spawn_blocking join: {e}"))?
        .map_err(|e| format!("删除 skill 目录失败: {e}"))?;
    Ok(format!("skill `{name}` 已删除 ({dir_str})"))
}

#[tauri::command]
async fn cmd_skill_retract(
    state: State<'_, AppState>,
    name: String,
    project: Option<String>,
    from: String,
) -> Result<String, String> {
    let (default_root, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let plat = parse_plat(&from)?;
    let src = locate_source(&default_root, &asset_root, AssetKind::Skill, &name)?;
    let dest = compute_dest(plat, AssetKind::Skill, &name, &src, &deploy_base)
        .ok_or_else(|| format!("无法算 skill `{name}` ← {plat:?} 的 dest"))?;
    let dest_str = dest.to_string();
    tokio::task::spawn_blocking(move || link::unlink(&dest))
        .await
        .map_err(|e| format!("spawn_blocking join: {e}"))?
        .map_err(|e| format!("link::unlink 失败: {e}"))?;
    Ok(format!("skill `{name}` ← {plat:?} OK ({dest_str})"))
}

#[tauri::command]
async fn cmd_rule_deploy(
    state: State<'_, AppState>,
    name: String,
    project: Option<String>,
    to: String,
) -> Result<String, String> {
    let (default_root, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let plat = parse_plat(&to)?;
    if !platform_supports(plat, AssetKind::Rule) {
        return Err(format!("{plat:?} 不支持 rule"));
    }
    let src = locate_source(&default_root, &asset_root, AssetKind::Rule, &name)?;
    let dest = compute_dest(plat, AssetKind::Rule, &name, &src, &deploy_base)
        .ok_or_else(|| format!("无法算 rule `{name}` → {plat:?} 的 dest"))?;
    let dest_str = dest.to_string();
    tokio::task::spawn_blocking(move || link::link(&src, &dest, LinkKind::auto()))
        .await
        .map_err(|e| format!("spawn_blocking join: {e}"))?
        .map_err(|e| format!("link::link 失败: {e}"))?;
    Ok(format!("rule `{name}` → {plat:?} OK ({dest_str})"))
}

#[tauri::command]
async fn cmd_rule_retract(
    state: State<'_, AppState>,
    name: String,
    project: Option<String>,
    from: String,
) -> Result<String, String> {
    let (default_root, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let plat = parse_plat(&from)?;
    if !platform_supports(plat, AssetKind::Rule) {
        return Err(format!("{plat:?} 不支持 rule"));
    }
    let src = locate_source(&default_root, &asset_root, AssetKind::Rule, &name)?;
    let dest = compute_dest(plat, AssetKind::Rule, &name, &src, &deploy_base)
        .ok_or_else(|| format!("无法算 rule `{name}` ← {plat:?} 的 dest"))?;
    let dest_str = dest.to_string();
    tokio::task::spawn_blocking(move || link::unlink(&dest))
        .await
        .map_err(|e| format!("spawn_blocking join: {e}"))?
        .map_err(|e| format!("link::unlink 失败: {e}"))?;
    Ok(format!("rule `{name}` ← {plat:?} OK ({dest_str})"))
}

#[tauri::command]
async fn cmd_rule_get(
    state: State<'_, AppState>,
    name: String,
    project: Option<String>,
) -> Result<AssetFileDetail, String> {
    let (default_root, asset_root, _deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let src = locate_source(&default_root, &asset_root, AssetKind::Rule, &name)?;
    let content = std::fs::read_to_string(&src).map_err(|e| format!("读取 rule 失败: {e}"))?;
    let (fm_name, description) = parse_skill_meta(&content);
    let parent_path = src.parent().map(|p| p.to_string()).unwrap_or_default();
    Ok(AssetFileDetail {
        name: fm_name.unwrap_or(name),
        description,
        content,
        source_path: src.to_string(),
        parent_path,
    })
}

#[tauri::command]
async fn cmd_rule_save(
    state: State<'_, AppState>,
    name: String,
    content: String,
    project: Option<String>,
) -> Result<String, String> {
    let (default_root, asset_root, _deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let src = locate_source(&default_root, &asset_root, AssetKind::Rule, &name)?;
    let path = src.to_string();
    tokio::task::spawn_blocking(move || std::fs::write(&path, &content))
        .await
        .map_err(|e| format!("spawn_blocking join: {e}"))?
        .map_err(|e| format!("写入 rule 失败: {e}"))?;
    Ok(format!("rule `{name}` 已保存"))
}

#[tauri::command]
async fn cmd_rule_delete(
    state: State<'_, AppState>,
    name: String,
    project: Option<String>,
) -> Result<String, String> {
    let (default_root, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    retract_all_platforms_best_effort(&default_root, &asset_root, &deploy_base, AssetKind::Rule, &name);
    let src = locate_source(&default_root, &asset_root, AssetKind::Rule, &name)?;
    let path = src.to_path_buf();
    let path_str = path.to_string();
    tokio::task::spawn_blocking(move || std::fs::remove_file(&path))
        .await
        .map_err(|e| format!("spawn_blocking join: {e}"))?
        .map_err(|e| format!("删除 rule 失败: {e}"))?;
    Ok(format!("rule `{name}` 已删除 ({path_str})"))
}

#[tauri::command]
async fn cmd_mcp_get(
    state: State<'_, AppState>,
    name: String,
    project: Option<String>,
) -> Result<AssetFileDetail, String> {
    let (default_root, asset_root, _deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let mcp_path = locate_mcp_json(&default_root, &asset_root)?;
    let asset_root = mcp_asset_root(&mcp_path)?;
    let config = mcp_json::get_server_config(asset_root, &name)
        .map_err(|e| format!("读取 MCP server 失败: {e}"))?
        .ok_or_else(|| format!("MCP server `{name}` 找不到"))?;
    let content =
        serde_json::to_string_pretty(&config).map_err(|e| format!("JSON 序列化失败: {e}"))?;
    let description = mcp_json::server_transport_summary(&config);
    let parent_path = mcp_path.to_string();
    Ok(AssetFileDetail {
        name,
        description,
        content,
        source_path: mcp_path.to_string(),
        parent_path,
    })
}

#[tauri::command]
async fn cmd_mcp_deploy(
    state: State<'_, AppState>,
    name: String,
    project: Option<String>,
    to: String,
) -> Result<String, String> {
    let (default_root, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let plat = parse_plat(&to)?;
    let mcp_path = locate_mcp_json(&default_root, &asset_root)?;
    let mcp_doc_root = mcp_asset_root(&mcp_path)?;
    let config = mcp_json::get_server_config(mcp_doc_root, &name)
        .map_err(|e| format!("读取 MCP server 失败: {e}"))?
        .ok_or_else(|| format!("MCP server `{name}` 找不到"))?;
    let dest_json = platform::for_scope(plat, &deploy_base)
        .map_err(|e| format!("platform 适配器失败: {e}"))?
        .mcp_json_path();
    let dest_for_blocking = dest_json.clone();
    let plat_label = format!("{plat:?}");
    let name_for_blocking = name.clone();
    tokio::task::spawn_blocking(move || -> Result<String, String> {
        mcp_json::upsert_server_on_platform(&dest_for_blocking, &name_for_blocking, &config)
            .map_err(|e| e.to_string())?;
        Ok(format!(
            "mcp `{name_for_blocking}` → {plat_label} OK ({dest_for_blocking})"
        ))
    })
    .await
    .map_err(|e| format!("spawn_blocking join: {e}"))?
}

#[tauri::command]
async fn cmd_mcp_retract(
    state: State<'_, AppState>,
    name: String,
    project: Option<String>,
    from: String,
) -> Result<String, String> {
    let (default_root, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let plat = parse_plat(&from)?;
    let _mcp_path = locate_mcp_json(&default_root, &asset_root)?;
    let dest_json = platform::for_scope(plat, &deploy_base)
        .map_err(|e| format!("platform 适配器失败: {e}"))?
        .mcp_json_path();
    let dest_for_blocking = dest_json.clone();
    let plat_label = format!("{plat:?}");
    let name_for_blocking = name.clone();
    tokio::task::spawn_blocking(move || -> Result<String, String> {
        mcp_json::remove_server_on_platform(&dest_for_blocking, &name_for_blocking)
            .map_err(|e| e.to_string())?;
        Ok(format!(
            "mcp `{name_for_blocking}` ← {plat_label} 已移除 ({dest_for_blocking})"
        ))
    })
    .await
    .map_err(|e| format!("spawn_blocking join: {e}"))?
}

#[tauri::command]
async fn cmd_mcp_save(
    state: State<'_, AppState>,
    name: String,
    content: String,
    project: Option<String>,
) -> Result<String, String> {
    let (default_root, asset_root, _deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let mcp_path = locate_mcp_json(&default_root, &asset_root)?;
    let asset_root = mcp_asset_root(&mcp_path)?;
    let config = serde_json::from_str::<serde_json::Value>(&content)
        .map_err(|e| format!("JSON 格式无效: {e}"))?;
    if !config.is_object() {
        return Err("MCP server config 须为 JSON object".to_string());
    }
    let asset_root_for_blocking = asset_root.to_path_buf();
    let name_for_blocking = name.clone();
    tokio::task::spawn_blocking(move || {
        mcp_json::upsert_server_in_document(&asset_root_for_blocking, &name_for_blocking, config)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("spawn_blocking join: {e}"))??;
    Ok(format!("mcp `{name}` 已保存"))
}

#[tauri::command]
async fn cmd_mcp_delete(
    state: State<'_, AppState>,
    name: String,
    project: Option<String>,
) -> Result<String, String> {
    let (default_root, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    retract_all_platforms_best_effort(&default_root, &asset_root, &deploy_base, AssetKind::Mcp, &name);
    let mcp_path = locate_mcp_json(&default_root, &asset_root)?;
    let asset_root = mcp_asset_root(&mcp_path)?;
    let asset_root_for_blocking = asset_root.to_path_buf();
    let name_for_blocking = name.clone();
    tokio::task::spawn_blocking(move || {
        mcp_json::remove_server_from_document(&asset_root_for_blocking, &name_for_blocking)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("spawn_blocking join: {e}"))??;
    Ok(format!("mcp `{name}` 已从 mcp.json 移除"))
}

#[tauri::command]
async fn cmd_agent_get(
    state: State<'_, AppState>,
    name: String,
    project: Option<String>,
) -> Result<AssetFileDetail, String> {
    let (default_root, asset_root, _deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let src = locate_source(&default_root, &asset_root, AssetKind::Agent, &name)?;
    let read_path = agent_edit_path(&src).unwrap_or_else(|| src.clone());
    let content =
        std::fs::read_to_string(&read_path).map_err(|e| format!("读取 agent 失败: {e}"))?;
    let description = parse_description(AssetKind::Agent, &src);
    let parent_path = read_path
        .parent()
        .map(|p| p.to_string())
        .unwrap_or_default();
    Ok(AssetFileDetail {
        name,
        description,
        content,
        source_path: read_path.to_string(),
        parent_path,
    })
}

#[tauri::command]
async fn cmd_agent_save(
    state: State<'_, AppState>,
    name: String,
    content: String,
    project: Option<String>,
) -> Result<String, String> {
    let (default_root, asset_root, _deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let src = locate_source(&default_root, &asset_root, AssetKind::Agent, &name)?;
    let write_path = agent_edit_path(&src).unwrap_or_else(|| src.clone());
    let path = write_path.to_string();
    tokio::task::spawn_blocking(move || std::fs::write(&path, &content))
        .await
        .map_err(|e| format!("spawn_blocking join: {e}"))?
        .map_err(|e| format!("写入 agent 失败: {e}"))?;
    Ok(format!("agent `{name}` 已保存"))
}

#[tauri::command]
async fn cmd_agent_delete(
    state: State<'_, AppState>,
    name: String,
    project: Option<String>,
) -> Result<String, String> {
    let (default_root, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    retract_all_platforms_best_effort(&default_root, &asset_root, &deploy_base, AssetKind::Agent, &name);
    let src = locate_source(&default_root, &asset_root, AssetKind::Agent, &name)?;
    let path = src.to_path_buf();
    let path_str = path.to_string();
    tokio::task::spawn_blocking(move || {
        if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        }
    })
    .await
    .map_err(|e| format!("spawn_blocking join: {e}"))?
    .map_err(|e| format!("删除 agent 失败: {e}"))?;
    Ok(format!("agent `{name}` 已删除 ({path_str})"))
}

#[tauri::command]
async fn cmd_agent_deploy(
    state: State<'_, AppState>,
    name: String,
    project: Option<String>,
    to: String,
) -> Result<String, String> {
    let (default_root, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let plat = parse_plat(&to)?;
    if !platform_supports(plat, AssetKind::Agent) {
        return Err(format!("{plat:?} 不支持 agent"));
    }
    let src = locate_source(&default_root, &asset_root, AssetKind::Agent, &name)?;
    let link_src = agent_link_src(&src);
    let dest = compute_dest(plat, AssetKind::Agent, &name, &src, &deploy_base)
        .ok_or_else(|| format!("无法算 agent `{name}` → {plat:?} 的 dest"))?;
    let dest_str = dest.to_string();
    tokio::task::spawn_blocking(move || link::link(&link_src, &dest, LinkKind::auto()))
        .await
        .map_err(|e| format!("spawn_blocking join: {e}"))?
        .map_err(|e| format!("link::link 失败: {e}"))?;
    Ok(format!("agent `{name}` → {plat:?} OK ({dest_str})"))
}

#[tauri::command]
async fn cmd_agent_retract(
    state: State<'_, AppState>,
    name: String,
    project: Option<String>,
    from: String,
) -> Result<String, String> {
    let (default_root, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let plat = parse_plat(&from)?;
    if !platform_supports(plat, AssetKind::Agent) {
        return Err(format!("{plat:?} 不支持 agent"));
    }
    let src = locate_source(&default_root, &asset_root, AssetKind::Agent, &name)?;
    let dest = compute_dest(plat, AssetKind::Agent, &name, &src, &deploy_base)
        .ok_or_else(|| format!("无法算 agent `{name}` ← {plat:?} 的 dest"))?;
    let dest_str = dest.to_string();
    tokio::task::spawn_blocking(move || link::unlink(&dest))
        .await
        .map_err(|e| format!("spawn_blocking join: {e}"))?
        .map_err(|e| format!("link::unlink 失败: {e}"))?;
    Ok(format!("agent `{name}` ← {plat:?} OK ({dest_str})"))
}

// ── Tauri command:项目注册(cmd_projects_*) ─────────────────────

#[tauri::command]
async fn cmd_projects_list(state: State<'_, AppState>) -> Result<Vec<Project>, String> {
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || store.projects().list())
        .await
        .map_err(|e| format!("spawn_blocking join: {e}"))?
        .map_err(|e| format!("projects.list 失败: {e}"))
}

#[tauri::command]
async fn cmd_projects_add(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    root_path: String,
) -> Result<Project, String> {
    let store = state.store.clone();
    let path = Utf8PathBuf::from(&root_path);
    if !path.is_dir() {
        return Err(format!("`{root_path}` 不是已存在的目录"));
    }
    let (repo_root, asset_root) = paths::resolve_project_roots(&path);
    let repo_for_store = repo_root.clone();
    let asset_for_layout = asset_root.clone();
    let project = tokio::task::spawn_blocking(move || -> Result<Project, String> {
        paths::ensure_asset_layout(&asset_for_layout)
            .map_err(|e| format!("初始化项目 .ai-config 目录失败: {e}"))?;
        store
            .projects()
            .add(&name, &repo_for_store)
            .map_err(|e| format!("projects.add 失败: {e}"))
    })
    .await
    .map_err(|e| format!("spawn_blocking join: {e}"))??;
    restart_asset_watcher(&app, &state).await?;
    Ok(project)
}

#[tauri::command]
async fn cmd_projects_remove(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || store.projects().remove(&name))
        .await
        .map_err(|e| format!("spawn_blocking join: {e}"))?
        .map_err(|e| format!("projects.remove 失败: {e}"))?;
    restart_asset_watcher(&app, &state).await?;
    Ok(())
}

// ── 内部辅助 ──────────────────────────────────────────────────────

/// 收集需要监听的资产根(默认根 + 已注册项目),去重。
async fn collect_watch_roots(state: &AppState) -> Result<Vec<Utf8PathBuf>, String> {
    let default_root = state.default_root.read().await.clone();
    let store = state.store.clone();
    let projects = tokio::task::spawn_blocking(move || store.projects().list())
        .await
        .map_err(|e| format!("spawn_blocking join: {e}"))?
        .map_err(|e| format!("projects.list 失败: {e}"))?;
    let mut roots = vec![default_root];
    for p in projects {
        let (_, asset_root) = paths::resolve_project_roots(&p.root_path);
        roots.push(asset_root);
    }
    Ok(dedupe_roots(roots))
}

/// 启动或重启资产目录监听,变动后向前端 `assets-changed`。
async fn restart_asset_watcher(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let asset_roots = collect_watch_roots(state).await?;
    let handle = app.clone();
    let watcher = start_debounced(WatchRoots { asset_roots }, move || {
        if let Err(e) = handle.emit("assets-changed", ()) {
            tracing::warn!("emit assets-changed 失败: {e}");
        }
    })
    .map_err(|e| format!("启动文件监听失败: {e}"))?;

    let mut guard = state
        .watcher
        .lock()
        .map_err(|e| format!("watcher mutex 中毒: {e}"))?;
    *guard = Some(watcher);
    tracing::info!("资产目录文件监听已就绪");
    Ok(())
}

/// 解析 `to` / `from` 字符串到 `PlatformId`。
fn parse_plat(s: &str) -> Result<PlatformId, String> {
    match s {
        "cursor" | "Cursor" | "cu" => Ok(PlatformId::Cursor),
        "codex" | "Codex" | "cx" => Ok(PlatformId::Codex),
        "claude" | "Claude" | "cl" => Ok(PlatformId::Claude),
        "hermes" | "Hermes" | "he" => Ok(PlatformId::Hermes),
        other => Err(format!(
            "未知平台 `{other}`(预期 cursor/codex/claude/hermes)"
        )),
    }
}

/// 解析当前作用域:全局默认根、资产扫描根、平台下发根。
async fn resolve_scope(
    state: &State<'_, AppState>,
    project: Option<&str>,
) -> Result<(Utf8PathBuf, Utf8PathBuf, Utf8PathBuf), String> {
    let default_root = state.default_root.read().await.clone();
    match project {
        None | Some("user-global") | Some("") => Ok((
            default_root.clone(),
            default_root,
            paths::global_deploy_base(),
        )),
        Some(name) => {
            let owned: String = name.to_string();
            let store_for_blocking = state.store.clone();
            let stored = tokio::task::spawn_blocking(move || -> Result<Utf8PathBuf, String> {
                store_for_blocking
                    .projects()
                    .get_by_name(&owned)
                    .map_err(|e| e.to_string())?
                    .map(|p| p.root_path)
                    .ok_or_else(|| format!("项目 `{owned}` 未注册"))
            })
            .await
            .map_err(|e| format!("spawn_blocking join: {e}"))??;
            let (repo_root, asset_root) = paths::resolve_project_roots(&stored);
            Ok((
                default_root,
                asset_root,
                paths::project_deploy_base(&repo_root),
            ))
        }
    }
}

/// 当前项目在 store 里的 id(用于 mcp render 的 project_id 标记)。
///
/// W9 暂未在 mcp render 路径上消费(W9 的 mcp deploy 把 secrets 传空,project_id
/// 没传到 template::render_mcp);保留签名,W10 接 secrets + 真值用。
#[allow(dead_code)]
async fn resolve_project_id(
    state: &State<'_, AppState>,
    project: Option<&str>,
) -> Result<u64, String> {
    match project {
        None | Some("user-global") | Some("") => Ok(0),
        Some(name) => {
            let owned: String = name.to_string();
            let store_for_blocking = state.store.clone();
            tokio::task::spawn_blocking(move || -> Result<u64, String> {
                store_for_blocking
                    .projects()
                    .get_by_name(&owned)
                    .map(|opt| opt.map(|p| p.id).unwrap_or(0))
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| format!("spawn_blocking join: {e}"))?
        }
    }
}

/// 删除前尽力收回各平台链接；失败不阻断删源文件。
fn retract_all_platforms_best_effort(
    default_root: &Utf8Path,
    asset_root: &Utf8Path,
    deploy_base: &Utf8Path,
    kind: AssetKind,
    name: &str,
) {
    if kind == AssetKind::Mcp {
        let Ok(mcp_path) = locate_mcp_json(default_root, asset_root) else {
            return;
        };
        let Ok(mcp_doc_root) = mcp_asset_root(&mcp_path) else {
            return;
        };
        if mcp_json::get_server_config(mcp_doc_root, name)
            .ok()
            .flatten()
            .is_none()
        {
            return;
        }
        for plat in [
            PlatformId::Cursor,
            PlatformId::Codex,
            PlatformId::Claude,
            PlatformId::Hermes,
        ] {
            if !platform_supports(plat, kind) {
                continue;
            }
            if let Ok(adapter) = platform::for_scope(plat, deploy_base) {
                let _ = mcp_json::remove_server_on_platform(&adapter.mcp_json_path(), name);
            }
        }
        return;
    }
    let Ok(src) = locate_source(default_root, asset_root, kind, name) else {
        return;
    };
    for plat in [
        PlatformId::Cursor,
        PlatformId::Codex,
        PlatformId::Claude,
        PlatformId::Hermes,
    ] {
        if !platform_supports(plat, kind) {
            continue;
        }
        let Some(dest) = compute_dest(plat, kind, name, &src, deploy_base) else {
            continue;
        };
        let _ = link::unlink(&dest);
    }
}

/// 按作用域扫描资产源。
///
/// - **user-global**（`asset_root == default_root`）：只扫 `~/.ai-config/`
/// - **已注册项目**（`asset_root` 为 `<repo>/.ai-config/`）：只扫项目树，**不**混入全局条目
fn scan_assets_for_scope(
    default_root: &Utf8Path,
    asset_root: &Utf8Path,
) -> Result<source::ScanResult, String> {
    if asset_root != default_root {
        if !asset_root.exists() {
            return Ok(source::ScanResult::default());
        }
        return source::scan_project_root(asset_root).map_err(|e| format!("scan 失败: {e}"));
    }
    source::scan_project_root(asset_root).map_err(|e| format!("scan 失败: {e}"))
}

/// 定位当前作用域的 `mcp.json` 源路径。
fn locate_mcp_json(
    default_root: &Utf8Path,
    asset_root: &Utf8Path,
) -> Result<Utf8PathBuf, String> {
    let scan = scan_assets_for_scope(default_root, asset_root)?;
    scan.mcp_json
        .ok_or_else(|| format!("源 `{}` 找不到", mcp_json::MCP_ASSET_NAME))
}

fn mcp_asset_root(mcp_path: &Utf8Path) -> Result<&Utf8Path, String> {
    mcp_path
        .parent()
        .ok_or_else(|| format!("mcp.json 路径无效: {mcp_path}"))
}

/// 在当前作用域资产根里定位一条资产的源路径。
fn locate_source(
    default_root: &Utf8Path,
    asset_root: &Utf8Path,
    kind: AssetKind,
    name: &str,
) -> Result<Utf8PathBuf, String> {
    let scan = scan_assets_for_scope(default_root, asset_root)?;
    let pool: Vec<Utf8PathBuf> = match kind {
        AssetKind::Skill => scan
            .skills
            .iter()
            .filter(|p| {
                p.parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n == name)
                    .unwrap_or(false)
            })
            .cloned()
            .collect(),
        AssetKind::Rule => scan
            .rules
            .iter()
            .filter(|p| p.file_stem().map(|n| n == name).unwrap_or(false))
            .cloned()
            .collect(),
        AssetKind::Mcp => {
            let Some(mcp_path) = scan.mcp_json else {
                return Err(format!("源 `{}` 找不到", mcp_json::MCP_ASSET_NAME));
            };
            let asset_root = mcp_asset_root(&mcp_path)?;
            if mcp_json::get_server_config(asset_root, name)
                .map_err(|e| format!("读取 MCP 失败: {e}"))?
                .is_none()
            {
                return Err(format!("MCP server `{name}` 找不到"));
            }
            vec![mcp_path]
        }
        AssetKind::Agent => scan
            .agents
            .iter()
            .filter(|p| {
                let candidate = if p.is_dir() {
                    p.file_name().map(|s| s.to_string())
                } else {
                    p.file_stem().map(|s| s.to_string())
                };
                candidate.as_deref() == Some(name)
            })
            .cloned()
            .collect(),
    };
    pool.into_iter()
        .next()
        .ok_or_else(|| format!("资产 `{kind:?}` 名 `{name}` 在源里找不到"))
}

#[allow(dead_code)]
fn core_err_to_string(e: CoreError) -> String {
    e.to_string()
}

// ── Tauri 主入口 ──────────────────────────────────────────────────

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn agent_description_uses_frontmatter_like_skill() {
        let content = r#"---
name: frontend-dev
description: zh-cloud Web frontend expert
---

你是专家。
"#;
        assert_eq!(parse_skill_meta(content).1, "zh-cloud Web frontend expert");
    }

    #[test]
    fn rule_description_uses_frontmatter() {
        let content = r#"---
description: 跨端 UX 默认偏好
alwaysApply: false
---

## 一般原则
"#;
        assert_eq!(parse_skill_meta(content).1, "跨端 UX 默认偏好");
    }

    #[test]
    fn agent_description_falls_back_to_h1_without_frontmatter() {
        let content = "# My Agent Title\n\nbody";
        assert_eq!(parse_skill_meta(content).1, "My Agent Title");
    }
}

/// Tauri 主入口。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            // 启动期 trace,前端 console 可看到 daemon 启动信息
            let state: State<'_, AppState> = app.state();
            tracing::info!(
                "ai-config GUI v{} 启动;store @ {}",
                env!("CARGO_PKG_VERSION"),
                state.store.path()
            );
            let handle = app.handle().clone();
            tauri::async_runtime::block_on(async {
                if let Err(e) = restart_asset_watcher(&handle, &state).await {
                    tracing::warn!("文件监听未启动: {e}");
                }
            });
            #[cfg(debug_assertions)]
            {
                if let Some(window) = app.get_webview_window("main") {
                    tracing::info!("main webview window 已就绪: {:?}", window.title());
                }
            }
            Ok(())
        })
        .manage(AppState::new().expect("AppState::new"))
        .invoke_handler(tauri::generate_handler![
            cmd_doctor,
            cmd_list,
            cmd_skill_get,
            cmd_skill_save,
            cmd_skill_delete,
            cmd_skill_deploy,
            cmd_skill_retract,
            cmd_rule_get,
            cmd_rule_save,
            cmd_rule_delete,
            cmd_rule_deploy,
            cmd_rule_retract,
            cmd_mcp_get,
            cmd_mcp_save,
            cmd_mcp_delete,
            cmd_mcp_deploy,
            cmd_mcp_retract,
            cmd_agent_get,
            cmd_agent_save,
            cmd_agent_delete,
            cmd_agent_deploy,
            cmd_agent_retract,
            cmd_projects_list,
            cmd_projects_add,
            cmd_projects_remove,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ai-config GUI");
}
