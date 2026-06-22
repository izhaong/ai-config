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
//! | `cmd_skill_deploy` | `name, project, to` | `String` | `materialize::deploy`（实体硬拷贝） |
//! | `cmd_skill_retract` | `name, project, from` | `String` | `materialize::retract` |
//! | `cmd_rule_deploy` | 同上 | `String` | `materialize::deploy` |
//! | `cmd_rule_retract` | 同上 | `String` | `materialize::retract` |
//! | `cmd_mcp_deploy` | 同上 | `String` | `mcp_json::upsert_server_on_platform` |
//! | `cmd_mcp_retract` | 同上 | `String` | `mcp_json::remove_server_on_platform` |
//! | `cmd_agent_deploy` | 同上 | `String` | `materialize::deploy` |
//! | `cmd_agent_retract` | 同上 | `String` | `materialize::retract` |
//! | `cmd_projects_list` | — | `Vec<Project>` | `Store::projects().list()` |
//! | `cmd_projects_add` | `name, root_path` | `Project` | `Store::projects().add()` |
//! | `cmd_projects_remove` | `name` | `()` | `Store::projects().remove()` |
//!
//! ## 共享状态
//!
//! `AppState` 持有 `Arc<Store>`(SQLite 持久化)和 `default_root: Arc<RwLock<Utf8PathBuf>>`(资产根)。
//! 同步阻塞 IO 走 `tokio::task::spawn_blocking` 包裹,避免锁住 Tauri runtime。

mod command_bridge;
mod marketplace;

use std::sync::{Arc, Mutex};

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::RwLock;

use ai_config_core::asset_scope;
use ai_config_core::doctor;
use ai_config_core::error::CoreError;
use ai_config_core::git::{self, GitEnsureOutcome, GitRepoStatus, GitSyncConfig, GitSyncOutcome};
use ai_config_core::materialize;
use ai_config_core::mcp_json;
use ai_config_core::model::{AssetKind, PlatformId, Project};
use ai_config_core::paths;
use ai_config_core::platform;
use ai_config_core::platform_scan::{self, PlatformAssetEntry, PlatformAssetList};
use ai_config_core::source;
use ai_config_core::sync::{agent_link_src, asset_dest_for_at_base};
use ai_config_core::template::McpSyncState;
use ai_config_store::Store;
use ai_config_watcher::{dedupe_roots, start_debounced, WatchRoots, WatcherHandle};

use ai_config_core::asset_ops::parse_skill_meta;
use ai_config_core::asset_ops::AssetFileDetail;

// ── 共享状态 ──────────────────────────────────────────────────────

/// Tauri 主进程长驻共享状态。
pub struct AppState {
    /// SQLite 持久化(W9 起:projects 表)
    pub store: Arc<Store>,
    /// 默认资产根(`~/.ai-config/`;启动时自动创建子目录)
    pub default_root: Arc<RwLock<Utf8PathBuf>>,
    /// 资产目录文件监听句柄(项目增删时重启)
    pub watcher: Mutex<Option<WatcherHandle>>,
    /// 启动时 Git 初始化结果（供前端提示配置远程）
    pub git_bootstrap: Arc<RwLock<Option<GitEnsureOutcome>>>,
}

impl AppState {
    fn new() -> Result<Self, String> {
        let store = Store::open().map_err(|e| format!("打开 store 失败: {e}"))?;
        let default_root = paths::discover_global_asset_root();
        tracing::info!("ai-config 资产根: {default_root}");

        let git_outcome = bootstrap_git_repo(&default_root, &store);

        Ok(Self {
            store: Arc::new(store),
            default_root: Arc::new(RwLock::new(default_root)),
            watcher: Mutex::new(None),
            git_bootstrap: Arc::new(RwLock::new(Some(git_outcome))),
        })
    }
}

/// 启动时确保 `~/.ai-config` 为 Git 仓库，并应用 store 中的远程配置。
fn bootstrap_git_repo(root: &Utf8Path, store: &Store) -> GitEnsureOutcome {
    let config = store.settings().git_config().unwrap_or_default();
    let branch = config.branch.as_str();
    let outcome = match git::ensure_repo(root, branch) {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("Git 仓库初始化失败: {e}");
            return GitEnsureOutcome {
                git_available: git::git_available(),
                was_repo: false,
                just_initialized: false,
                initial_commit: false,
            };
        }
    };
    if let Some(url) = config.remote_url.as_deref() {
        if let Err(e) = git::apply_remote(root, Some(url)) {
            tracing::warn!("应用 Git 远程失败: {e}");
        }
    }
    outcome
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
    /// 链接存在且指向 src（本工具纳管）
    Linked,
    /// 内容一致但非本工具下发
    Synced,
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

/// 健康检查（与 CLI `ai-config doctor` 同源）。
#[tauri::command]
async fn cmd_doctor(state: State<'_, AppState>) -> Result<DoctorSummary, String> {
    let default_root = state.default_root.read().await.clone();
    let report = tokio::task::spawn_blocking(move || doctor::compute_report(&default_root))
        .await
        .map_err(|e| format!("spawn_blocking join: {e}"))?
        .map_err(|e| e.to_string())?;
    Ok(DoctorSummary {
        broken: report.broken as u64,
        wrong_source: report.wrong_source as u64,
        wrong_type: report.wrong_type as u64,
        missing_secrets: report.missing_secrets.into_iter().map(|m| m.key).collect(),
        unregistered_projects: report.unregistered_projects,
        platform_capability_issues: report
            .platform_capability_issues
            .into_iter()
            .map(|i| PlatformIssue {
                platform: parse_plat(&i.platform).unwrap_or(PlatformId::Cursor),
                kind: i.kind,
                reason: i.reason,
            })
            .collect(),
        exit_code: report.exit_code,
    })
}

fn platform_label(p: PlatformId) -> &'static str {
    platform::platform_label(p)
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
    for p in &scan.commands {
        if let Some(stem) = p.file_stem() {
            entries.push((AssetKind::Command, stem.to_string(), p.clone()));
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
            if !platform_supports(plat, kind, &deploy_base) {
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
                match materialize::check(&dest, &expected_src) {
                    materialize::DeployHealth::Linked { .. } => {
                        if materialize::is_managed_deploy(&dest) {
                            LinkState::Linked
                        } else {
                            LinkState::Synced
                        }
                    }
                    materialize::DeployHealth::Broken => LinkState::Broken,
                    materialize::DeployHealth::Unlinked => LinkState::Unlinked,
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

// ── Tauri command:平台反向列表 ─────────────────────────────────────

/// 扫描指定平台上的资产（反向同步视图）。
#[tauri::command]
async fn cmd_list_platform(
    state: State<'_, AppState>,
    project: Option<String>,
    platform: String,
    kind: Option<String>,
) -> Result<PlatformAssetList, String> {
    let plat = parse_plat(&platform)?;
    let (default_root, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;

    let filter_kind = kind.as_deref().map(parse_kind).transpose()?;

    let kinds: Vec<AssetKind> = match filter_kind {
        Some(k) => vec![k],
        None => vec![
            AssetKind::Skill,
            AssetKind::Rule,
            AssetKind::Mcp,
            AssetKind::Agent,
            AssetKind::Command,
        ],
    };

    let mut entries: Vec<PlatformAssetEntry> = Vec::new();
    for k in kinds {
        let mut batch =
            platform_scan::scan_platform_assets(plat, k, &deploy_base, &default_root, &asset_root)
                .map_err(|e| e.to_string())?;
        entries.append(&mut batch);
    }

    Ok(PlatformAssetList {
        entries,
        platform: plat,
    })
}

/// 返回当前作用域下、指定资产种类在各平台的根路径（GUI 平台 dock 提示用）。
#[derive(Debug, Serialize)]
struct PlatformKindPath {
    platform: String,
    path: String,
    supported: bool,
}

#[tauri::command]
async fn cmd_platform_kind_paths(
    state: State<'_, AppState>,
    project: Option<String>,
    kind: String,
) -> Result<Vec<PlatformKindPath>, String> {
    let k = parse_kind(&kind)?;
    let (_, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let mut out = Vec::with_capacity(5);
    for plat in ai_config_core::platform::ui_platform_ids() {
        let supported = if plat == PlatformId::AiConfig {
            true
        } else {
            platform_supports(plat, k, &deploy_base)
        };
        let path = ai_config_core::platform::kind_asset_path(plat, k, &deploy_base, &asset_root)
            .map(|p| p.to_string())
            .unwrap_or_default();
        out.push(PlatformKindPath {
            platform: platform_label(plat).to_string(),
            path,
            supported,
        });
    }
    Ok(out)
}

fn parse_kind(s: &str) -> Result<AssetKind, String> {
    asset_scope::parse_asset_kind(s).map_err(|e| e.to_string())
}

/// 从平台导入 skill 到 ai-config 源。
#[tauri::command]
async fn cmd_skill_import(
    state: State<'_, AppState>,
    name: String,
    project: Option<String>,
    from_platform: String,
) -> Result<String, String> {
    let plat = parse_plat(&from_platform)?;
    if plat == PlatformId::AiConfig {
        return Err("不能从 ai-config 源导入到自身".into());
    }
    let (default_root, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let name_for_msg = name.clone();
    let dest = tokio::task::spawn_blocking(move || {
        platform_scan::import_skill_from_platform(
            &name,
            plat,
            &default_root,
            &asset_root,
            &deploy_base,
        )
    })
    .await
    .map_err(|e| format!("spawn_blocking join: {e}"))?
    .map_err(|e| e.to_string())?;
    Ok(format!("skill `{name_for_msg}` 已导入到 {dest}"))
}

/// 通过 `npx skills add` 从远程仓库安装 skill 到当前浏览平台。
#[tauri::command]
async fn cmd_skill_add(
    state: State<'_, AppState>,
    source: String,
    skill_name: Option<String>,
    project: Option<String>,
    target_platform: String,
) -> Result<String, String> {
    let plat = parse_plat(&target_platform)?;
    let (default_root, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    command_bridge::add_remote_skill(
        default_root,
        asset_root,
        deploy_base,
        source,
        skill_name,
        plat,
    )
    .await
}

/// 批量通过 `npx skills add` 安装到多个平台。
#[tauri::command]
async fn cmd_skill_add_batch(
    state: State<'_, AppState>,
    project: Option<String>,
    skills: Vec<SkillAddBatchItemDto>,
    target_platforms: Vec<String>,
) -> Result<ai_config_core::skills_add::SkillAddBatchOutcome, String> {
    if skills.is_empty() {
        return Err("未选择任何 skill".into());
    }
    if target_platforms.is_empty() {
        return Err("未选择任何目标平台".into());
    }
    let platforms: Result<Vec<PlatformId>, String> =
        target_platforms.iter().map(|s| parse_plat(s)).collect();
    let platforms = platforms?;
    let (default_root, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let items: Vec<ai_config_core::skills_add::SkillAddBatchItem> = skills
        .into_iter()
        .map(|s| ai_config_core::skills_add::SkillAddBatchItem {
            source: s.source,
            skill_name: s.skill_name,
        })
        .collect();
    command_bridge::add_remote_skills_batch(default_root, asset_root, deploy_base, items, platforms)
        .await
}

#[derive(serde::Deserialize)]
struct SkillAddBatchItemDto {
    source: String,
    skill_name: Option<String>,
}

/// 拉取 [Claude Marketplace](https://www.claudemarketplace.net/skills) skill 列表。
#[tauri::command]
async fn cmd_marketplace_list_skills(
    sort: String,
    q: Option<String>,
    source_filter: Option<String>,
    offset: Option<u32>,
    limit: Option<u32>,
) -> Result<marketplace::MarketplaceListResult, String> {
    let offset = offset.unwrap_or(0);
    let limit = limit.unwrap_or(50);
    let sort_owned = sort;
    let q_owned = q;
    let source_owned = source_filter;
    tokio::task::spawn_blocking(move || {
        marketplace::list_skills(
            &sort_owned,
            q_owned.as_deref(),
            source_owned.as_deref(),
            offset,
            limit,
        )
    })
    .await
    .map_err(|e| format!("spawn_blocking join: {e}"))?
}

#[tauri::command]
async fn cmd_rule_import(
    state: State<'_, AppState>,
    name: String,
    project: Option<String>,
    from_platform: String,
) -> Result<String, String> {
    let plat = parse_plat(&from_platform)?;
    if plat == PlatformId::AiConfig {
        return Err("不能从 ai-config 源导入到自身".into());
    }
    let (default_root, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let name_for_msg = name.clone();
    let dest = tokio::task::spawn_blocking(move || {
        platform_scan::import_rule_from_platform(
            &name,
            plat,
            &default_root,
            &asset_root,
            &deploy_base,
        )
    })
    .await
    .map_err(|e| format!("spawn_blocking join: {e}"))?
    .map_err(|e| e.to_string())?;
    Ok(format!("rule `{name_for_msg}` 已导入到 {dest}"))
}

#[tauri::command]
async fn cmd_agent_import(
    state: State<'_, AppState>,
    name: String,
    project: Option<String>,
    from_platform: String,
) -> Result<String, String> {
    let plat = parse_plat(&from_platform)?;
    if plat == PlatformId::AiConfig {
        return Err("不能从 ai-config 源导入到自身".into());
    }
    let (default_root, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let name_for_msg = name.clone();
    let dest = tokio::task::spawn_blocking(move || {
        platform_scan::import_agent_from_platform(
            &name,
            plat,
            &default_root,
            &asset_root,
            &deploy_base,
        )
    })
    .await
    .map_err(|e| format!("spawn_blocking join: {e}"))?
    .map_err(|e| e.to_string())?;
    Ok(format!("agent `{name_for_msg}` 已导入到 {dest}"))
}

#[tauri::command]
async fn cmd_mcp_import(
    state: State<'_, AppState>,
    name: String,
    project: Option<String>,
    from_platform: String,
) -> Result<String, String> {
    let plat = parse_plat(&from_platform)?;
    if plat == PlatformId::AiConfig {
        return Err("不能从 ai-config 源导入到自身".into());
    }
    let (default_root, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let name_for_msg = name.clone();
    let dest = tokio::task::spawn_blocking(move || {
        platform_scan::import_mcp_from_platform(
            &name,
            plat,
            &default_root,
            &asset_root,
            &deploy_base,
        )
    })
    .await
    .map_err(|e| format!("spawn_blocking join: {e}"))?
    .map_err(|e| e.to_string())?;
    Ok(format!("mcp `{name_for_msg}` 已导入到 {dest}"))
}

/// 从平台已下发路径只读预览（无 ai-config 源时供详情抽屉使用）。
#[tauri::command]
async fn cmd_read_platform_asset(
    path: String,
    kind: String,
    name: String,
) -> Result<AssetFileDetail, String> {
    let kind = parse_kind(&kind)?;
    command_bridge::read_platform_preview(kind, path, name).await
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
        AssetKind::Command => std::fs::read_to_string(src)
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
fn platform_supports(plat: PlatformId, kind: AssetKind, deploy_base: &Utf8Path) -> bool {
    platform::supports_at_scope(plat, kind, deploy_base)
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

// ── Tauri command:单条资产 deploy / retract / CRUD（core::asset_ops）────────

macro_rules! asset_deploy_cmd {
    ($fn:ident, $kind:expr) => {
        #[tauri::command]
        async fn $fn(
            state: State<'_, AppState>,
            name: String,
            project: Option<String>,
            to: String,
        ) -> Result<String, String> {
            let (dr, ar, db) = resolve_scope(&state, project.as_deref()).await?;
            let plat = parse_deploy_plat(&to)?;
            command_bridge::deploy(dr, ar, db, $kind, name, plat).await
        }
    };
}

macro_rules! asset_deploy_from_platform_cmd {
    ($fn:ident, $kind:expr) => {
        #[tauri::command]
        async fn $fn(
            state: State<'_, AppState>,
            name: String,
            project: Option<String>,
            from: String,
            to: String,
        ) -> Result<String, String> {
            let (dr, ar, db) = resolve_scope(&state, project.as_deref()).await?;
            let from_plat = parse_deploy_plat(&from)?;
            let to_plat = parse_deploy_plat(&to)?;
            command_bridge::deploy_from_platform(dr, ar, db, $kind, name, from_plat, to_plat).await
        }
    };
}

macro_rules! asset_retract_cmd {
    ($fn:ident, $kind:expr) => {
        #[tauri::command]
        async fn $fn(
            state: State<'_, AppState>,
            name: String,
            project: Option<String>,
            from: String,
        ) -> Result<String, String> {
            let (dr, ar, db) = resolve_scope(&state, project.as_deref()).await?;
            let plat = platform::parse_platform_str(&from).map_err(|e| e.to_string())?;
            command_bridge::retract(dr, ar, db, $kind, name, plat).await
        }
    };
}

macro_rules! asset_get_cmd {
    ($fn:ident, $kind:expr) => {
        #[tauri::command]
        async fn $fn(
            state: State<'_, AppState>,
            name: String,
            project: Option<String>,
        ) -> Result<AssetFileDetail, String> {
            let (dr, ar, db) = resolve_scope(&state, project.as_deref()).await?;
            command_bridge::get_detail(dr, ar, db, $kind, name).await
        }
    };
}

macro_rules! asset_save_cmd {
    ($fn:ident, $kind:expr) => {
        #[tauri::command]
        async fn $fn(
            state: State<'_, AppState>,
            name: String,
            content: String,
            project: Option<String>,
        ) -> Result<String, String> {
            let (dr, ar, db) = resolve_scope(&state, project.as_deref()).await?;
            command_bridge::save_content(dr, ar, db, $kind, name, content).await
        }
    };
}

macro_rules! asset_retract_source_cmd {
    ($fn:ident, $kind:expr) => {
        #[tauri::command]
        async fn $fn(
            state: State<'_, AppState>,
            name: String,
            project: Option<String>,
        ) -> Result<String, String> {
            let (dr, ar, db) = resolve_scope(&state, project.as_deref()).await?;
            command_bridge::retract_source(dr, ar, db, $kind, name).await
        }
    };
}

macro_rules! asset_delete_cmd {
    ($fn:ident, $kind:expr) => {
        #[tauri::command]
        async fn $fn(
            state: State<'_, AppState>,
            name: String,
            project: Option<String>,
        ) -> Result<String, String> {
            let (dr, ar, db) = resolve_scope(&state, project.as_deref()).await?;
            command_bridge::delete_source(dr, ar, db, $kind, name).await
        }
    };
}

asset_deploy_cmd!(cmd_skill_deploy, AssetKind::Skill);
asset_deploy_from_platform_cmd!(cmd_skill_deploy_from_platform, AssetKind::Skill);
asset_get_cmd!(cmd_skill_get, AssetKind::Skill);
asset_save_cmd!(cmd_skill_save, AssetKind::Skill);
asset_delete_cmd!(cmd_skill_delete, AssetKind::Skill);
asset_retract_source_cmd!(cmd_skill_retract_source, AssetKind::Skill);
asset_retract_cmd!(cmd_skill_retract, AssetKind::Skill);

asset_deploy_cmd!(cmd_rule_deploy, AssetKind::Rule);
asset_deploy_from_platform_cmd!(cmd_rule_deploy_from_platform, AssetKind::Rule);
asset_get_cmd!(cmd_rule_get, AssetKind::Rule);
asset_save_cmd!(cmd_rule_save, AssetKind::Rule);
asset_delete_cmd!(cmd_rule_delete, AssetKind::Rule);
asset_retract_source_cmd!(cmd_rule_retract_source, AssetKind::Rule);
asset_retract_cmd!(cmd_rule_retract, AssetKind::Rule);

asset_deploy_cmd!(cmd_command_deploy, AssetKind::Command);
asset_deploy_from_platform_cmd!(cmd_command_deploy_from_platform, AssetKind::Command);
asset_get_cmd!(cmd_command_get, AssetKind::Command);
asset_save_cmd!(cmd_command_save, AssetKind::Command);
asset_delete_cmd!(cmd_command_delete, AssetKind::Command);
asset_retract_source_cmd!(cmd_command_retract_source, AssetKind::Command);
asset_retract_cmd!(cmd_command_retract, AssetKind::Command);

asset_deploy_cmd!(cmd_mcp_deploy, AssetKind::Mcp);
asset_get_cmd!(cmd_mcp_get, AssetKind::Mcp);
asset_save_cmd!(cmd_mcp_save, AssetKind::Mcp);
asset_delete_cmd!(cmd_mcp_delete, AssetKind::Mcp);
asset_retract_source_cmd!(cmd_mcp_retract_source, AssetKind::Mcp);
asset_retract_cmd!(cmd_mcp_retract, AssetKind::Mcp);

asset_deploy_cmd!(cmd_agent_deploy, AssetKind::Agent);
asset_deploy_from_platform_cmd!(cmd_agent_deploy_from_platform, AssetKind::Agent);
asset_get_cmd!(cmd_agent_get, AssetKind::Agent);
asset_save_cmd!(cmd_agent_save, AssetKind::Agent);
asset_delete_cmd!(cmd_agent_delete, AssetKind::Agent);
asset_retract_source_cmd!(cmd_agent_retract_source, AssetKind::Agent);
asset_retract_cmd!(cmd_agent_retract, AssetKind::Agent);

#[tauri::command]
async fn cmd_command_import(
    state: State<'_, AppState>,
    name: String,
    project: Option<String>,
    from_platform: String,
) -> Result<String, String> {
    let plat = parse_plat(&from_platform)?;
    if plat == PlatformId::AiConfig {
        return Err("不能从 ai-config 源导入到自身".into());
    }
    let (default_root, asset_root, deploy_base) = resolve_scope(&state, project.as_deref()).await?;
    let name_for_msg = name.clone();
    let dest = tokio::task::spawn_blocking(move || {
        platform_scan::import_command_from_platform(
            &name,
            plat,
            &default_root,
            &asset_root,
            &deploy_base,
        )
    })
    .await
    .map_err(|e| format!("spawn_blocking join: {e}"))?
    .map_err(|e| e.to_string())?;
    Ok(format!("command `{name_for_msg}` 已导入到 {dest}"))
}

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
    let resolved_name = {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            path.file_name()
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "无法从路径推断项目名，请填写项目名".to_string())?
        } else {
            trimmed.to_string()
        }
    };
    let (repo_root, asset_root) = paths::resolve_project_roots(&path);
    let repo_for_store = repo_root.clone();
    let asset_for_layout = asset_root.clone();
    let project = tokio::task::spawn_blocking(move || -> Result<Project, String> {
        paths::ensure_asset_layout(&asset_for_layout)
            .map_err(|e| format!("初始化项目 .ai-config 目录失败: {e}"))?;
        store
            .projects()
            .add(&resolved_name, &repo_for_store)
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

#[derive(Debug, serde::Deserialize)]
struct AssetTransferItem {
    kind: String,
    name: String,
}

/// 在系统文件管理器中打开目录。
#[tauri::command]
async fn cmd_reveal_path(path: String) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("路径为空".into());
    }
    tokio::task::spawn_blocking(move || reveal_path_in_file_manager(&path))
        .await
        .map_err(|e| format!("spawn_blocking join: {e}"))?
}

fn reveal_path_in_file_manager(path: &str) -> Result<(), String> {
    let p = Utf8Path::new(path);
    let target = if p.is_file() {
        p.parent().unwrap_or(p)
    } else {
        p
    };
    if !target.exists() {
        return Err(format!("路径不存在: {target}"));
    }
    let status = {
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open")
                .arg(target.as_str())
                .status()
        }
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("explorer")
                .arg(target.as_str())
                .status()
        }
        #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
        {
            std::process::Command::new("xdg-open")
                .arg(target.as_str())
                .status()
        }
    }
    .map_err(|e| format!("打开文件夹失败: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("无法打开: {target}"))
    }
}

/// 跨项目复制已纳管资产（粘贴到目标项目源）。
#[tauri::command]
async fn cmd_assets_transfer(
    app: AppHandle,
    state: State<'_, AppState>,
    from_project: String,
    to_project: String,
    items: Vec<AssetTransferItem>,
) -> Result<String, String> {
    if items.is_empty() {
        return Err("未选择任何资产".into());
    }
    let (from_def, from_root, _) = resolve_scope(&state, Some(&from_project)).await?;
    let (to_def, to_root, _) = resolve_scope(&state, Some(&to_project)).await?;
    let count = items.len();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        for item in &items {
            let kind = parse_kind(&item.kind)?;
            platform_scan::copy_asset_to_asset_root(
                kind, &item.name, &from_def, &from_root, &to_def, &to_root,
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("spawn_blocking join: {e}"))??;
    restart_asset_watcher(&app, &state).await?;
    Ok(format!("已复制 {count} 项到目标项目"))
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
    platform::parse_platform_str(s).map_err(|e| e.to_string())
}

fn parse_deploy_plat(s: &str) -> Result<PlatformId, String> {
    platform::parse_deploy_platform_str(s).map_err(|e| e.to_string())
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

/// 按作用域扫描资产源。
fn scan_assets_for_scope(
    default_root: &Utf8Path,
    asset_root: &Utf8Path,
) -> Result<source::ScanResult, String> {
    platform_scan::scan_source_for_scope(default_root, asset_root)
        .map_err(|e| format!("scan 失败: {e}"))
}

#[allow(dead_code)]
fn core_err_to_string(e: CoreError) -> String {
    e.to_string()
}

// ── Tauri command: ~/.ai-config Git 同步 ─────────────────────────

#[derive(Debug, Serialize)]
struct GitBootstrapResponse {
    asset_root: String,
    outcome: GitEnsureOutcome,
    status: GitRepoStatus,
    config: GitSyncConfig,
}

/// 启动时 Git 检查：确保仓库存在并返回状态（前端挂载时调用）。
#[tauri::command]
async fn cmd_git_bootstrap(state: State<'_, AppState>) -> Result<GitBootstrapResponse, String> {
    let root = state.default_root.read().await.clone();
    let store = Arc::clone(&state.store);
    let bootstrap = state.git_bootstrap.read().await.clone();
    tokio::task::spawn_blocking(move || {
        let config = store.settings().git_config().map_err(|e| e.to_string())?;
        let just = bootstrap
            .as_ref()
            .map(|o| o.just_initialized)
            .unwrap_or(false);
        let status = git::repo_status(&root, just);
        Ok(GitBootstrapResponse {
            asset_root: git::asset_root_display(&root),
            outcome: bootstrap.unwrap_or(GitEnsureOutcome {
                git_available: git::git_available(),
                was_repo: status.is_repo,
                just_initialized: false,
                initial_commit: false,
            }),
            status,
            config,
        })
    })
    .await
    .map_err(|e| format!("spawn_blocking join: {e}"))?
}

#[tauri::command]
async fn cmd_git_status(state: State<'_, AppState>) -> Result<GitRepoStatus, String> {
    let root = state.default_root.read().await.clone();
    let bootstrap = state.git_bootstrap.read().await.clone();
    tokio::task::spawn_blocking(move || {
        let just = bootstrap
            .as_ref()
            .map(|o| o.just_initialized)
            .unwrap_or(false);
        Ok(git::repo_status(&root, just))
    })
    .await
    .map_err(|e| format!("spawn_blocking join: {e}"))?
}

#[tauri::command]
async fn cmd_git_config_get(state: State<'_, AppState>) -> Result<GitSyncConfig, String> {
    let store = Arc::clone(&state.store);
    tokio::task::spawn_blocking(move || store.settings().git_config().map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("spawn_blocking join: {e}"))?
}

#[tauri::command]
async fn cmd_git_config_set(
    state: State<'_, AppState>,
    remote_url: Option<String>,
    branch: Option<String>,
) -> Result<GitSyncConfig, String> {
    let root = state.default_root.read().await.clone();
    let store = Arc::clone(&state.store);
    tokio::task::spawn_blocking(move || {
        let mut config = store.settings().git_config().map_err(|e| e.to_string())?;
        if remote_url.is_some() {
            config.remote_url = remote_url.filter(|s| !s.trim().is_empty());
        }
        if let Some(b) = branch.filter(|s| !s.trim().is_empty()) {
            config.branch = b;
        }
        store
            .settings()
            .set_git_config(&config)
            .map_err(|e| e.to_string())?;
        git::apply_remote(&root, config.remote_url.as_deref()).map_err(|e| e.to_string())?;
        Ok(config)
    })
    .await
    .map_err(|e| format!("spawn_blocking join: {e}"))?
}

#[tauri::command]
async fn cmd_git_sync(state: State<'_, AppState>) -> Result<GitSyncOutcome, String> {
    let root = state.default_root.read().await.clone();
    let store = Arc::clone(&state.store);
    tokio::task::spawn_blocking(move || {
        let config = store.settings().git_config().map_err(|e| e.to_string())?;
        git::sync_repo(&root, &config, "chore: ai-config 同步").map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("spawn_blocking join: {e}"))?
}

#[tauri::command]
async fn cmd_git_pull(state: State<'_, AppState>) -> Result<String, String> {
    let root = state.default_root.read().await.clone();
    let store = Arc::clone(&state.store);
    tokio::task::spawn_blocking(move || {
        let config = store.settings().git_config().map_err(|e| e.to_string())?;
        if config
            .remote_url
            .as_deref()
            .filter(|s| !s.is_empty())
            .is_none()
        {
            return Err("未配置远程仓库".to_string());
        }
        git::apply_remote(&root, config.remote_url.as_deref()).map_err(|e| e.to_string())?;
        let pulled = git::pull(&root, &config.branch).map_err(|e| e.to_string())?;
        Ok(if pulled {
            "已从远程拉取".to_string()
        } else {
            "拉取完成（无变更）".to_string()
        })
    })
    .await
    .map_err(|e| format!("spawn_blocking join: {e}"))?
}

#[tauri::command]
async fn cmd_git_push(state: State<'_, AppState>) -> Result<String, String> {
    let root = state.default_root.read().await.clone();
    let store = Arc::clone(&state.store);
    tokio::task::spawn_blocking(move || {
        let config = store.settings().git_config().map_err(|e| e.to_string())?;
        if config
            .remote_url
            .as_deref()
            .filter(|s| !s.is_empty())
            .is_none()
        {
            return Err("未配置远程仓库".to_string());
        }
        git::apply_remote(&root, config.remote_url.as_deref()).map_err(|e| e.to_string())?;
        let pushed = git::push(&root, &config.branch, true).map_err(|e| e.to_string())?;
        Ok(if pushed {
            "已推送到远程".to_string()
        } else {
            "推送完成".to_string()
        })
    })
    .await
    .map_err(|e| format!("spawn_blocking join: {e}"))?
}

// ── Tauri 主入口 ──────────────────────────────────────────────────

/// Tauri 主入口。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
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
            cmd_list_platform,
            cmd_platform_kind_paths,
            cmd_skill_import,
            cmd_skill_add,
            cmd_skill_add_batch,
            cmd_marketplace_list_skills,
            cmd_rule_import,
            cmd_command_import,
            cmd_agent_import,
            cmd_mcp_import,
            cmd_skill_get,
            cmd_skill_save,
            cmd_skill_delete,
            cmd_skill_retract_source,
            cmd_skill_deploy,
            cmd_skill_deploy_from_platform,
            cmd_skill_retract,
            cmd_rule_get,
            cmd_rule_save,
            cmd_rule_delete,
            cmd_rule_retract_source,
            cmd_rule_deploy,
            cmd_rule_deploy_from_platform,
            cmd_rule_retract,
            cmd_command_get,
            cmd_command_save,
            cmd_command_delete,
            cmd_command_retract_source,
            cmd_command_deploy,
            cmd_command_deploy_from_platform,
            cmd_command_retract,
            cmd_mcp_get,
            cmd_mcp_save,
            cmd_mcp_delete,
            cmd_mcp_retract_source,
            cmd_mcp_deploy,
            cmd_mcp_retract,
            cmd_agent_get,
            cmd_agent_save,
            cmd_agent_delete,
            cmd_agent_retract_source,
            cmd_agent_deploy,
            cmd_agent_deploy_from_platform,
            cmd_agent_retract,
            cmd_projects_list,
            cmd_projects_add,
            cmd_projects_remove,
            cmd_reveal_path,
            cmd_read_platform_asset,
            cmd_assets_transfer,
            cmd_git_bootstrap,
            cmd_git_status,
            cmd_git_config_get,
            cmd_git_config_set,
            cmd_git_sync,
            cmd_git_pull,
            cmd_git_push,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ai-config GUI");
}

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
