//! 用户全局 MCP 配置:单一 `mcp.json`(明文 `mcpServers`,无 secrets 注入)。

use camino::{Utf8Path, Utf8PathBuf};
use serde_json::{Map, Value};

use crate::error::CoreError;
use crate::hermes_config::{self, HermesMigrateReport};
use crate::model::PlatformId;
use crate::template::{
    atomic_write_json, mcp_server_sync_state, read_mcp_json, remove_mcp_server_entry,
    upsert_mcp_server_entry, McpSyncState,
};

/// 列表 / GUI 中 MCP 资产的固定名称。
pub const MCP_ASSET_NAME: &str = "mcp.json";

/// 资产根下的 `mcp.json` 路径。
pub fn mcp_json_path(asset_root: &Utf8Path) -> Utf8PathBuf {
    asset_root.join(MCP_ASSET_NAME)
}

/// 确保 `mcp.json` 存在(空 `mcpServers`)。
pub fn ensure_mcp_json(asset_root: &Utf8Path) -> Result<Utf8PathBuf, CoreError> {
    let path = mcp_json_path(asset_root);
    if !path.is_file() {
        atomic_write_json(&path, &empty_mcp_document())?;
    }
    Ok(path)
}

fn empty_mcp_document() -> Value {
    serde_json::json!({ "mcpServers": {} })
}

fn normalize_mcp_document(mut v: Value) -> Value {
    if !v.is_object() {
        return empty_mcp_document();
    }
    if v.get("mcpServers").and_then(|x| x.as_object()).is_none() {
        v.as_object_mut()
            .expect("checked object")
            .insert("mcpServers".to_string(), Value::Object(Map::new()));
    }
    v
}

/// 读取资产根 `mcp.json`;不存在则 `None`。
pub fn load_mcp_document(asset_root: &Utf8Path) -> Result<Option<Value>, CoreError> {
    let path = mcp_json_path(asset_root);
    match read_mcp_json(&path)? {
        Some(v) => Ok(Some(normalize_mcp_document(v))),
        None => Ok(None),
    }
}

/// 写入资产根 `mcp.json`。
pub fn save_mcp_document(asset_root: &Utf8Path, doc: &Value) -> Result<(), CoreError> {
    let path = mcp_json_path(asset_root);
    atomic_write_json(&path, &normalize_mcp_document(doc.clone()))
}

/// 列出 `mcpServers` 的 key(稳定排序)。
pub fn list_server_names(asset_root: &Utf8Path) -> Result<Vec<String>, CoreError> {
    let Some(doc) = load_mcp_document(asset_root)? else {
        return Ok(Vec::new());
    };
    let mut names: Vec<String> = doc
        .get("mcpServers")
        .and_then(|v| v.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    names.sort();
    Ok(names)
}

/// 从平台下发文件（`mcp.json` 或 Hermes `config.yaml`）读取单条 server 配置。
pub fn get_server_config_from_deploy_file(
    deploy_path: &Utf8Path,
    server_name: &str,
) -> Result<Value, CoreError> {
    if deploy_path.file_name() == Some("config.yaml") {
        return hermes_config::get_mcp_server_config(deploy_path, server_name);
    }
    let Some(doc) = read_mcp_json(deploy_path)? else {
        return Err(CoreError::AssetNotFound {
            kind: crate::model::AssetKind::Mcp,
            name: server_name.into(),
            hint: format!("平台 MCP 文件不存在: {deploy_path}"),
        });
    };
    doc.get("mcpServers")
        .and_then(|v| v.get(server_name))
        .cloned()
        .ok_or_else(|| CoreError::AssetNotFound {
            kind: crate::model::AssetKind::Mcp,
            name: server_name.into(),
            hint: format!("mcp.json 中无 server `{server_name}`"),
        })
}

/// 将 `mcp/servers/*.json` 与 `mcp/cursor.mcp.template.json` 合并进 `mcp.json`,并删除旧 `mcp/` 目录。
pub fn migrate_legacy_mcp_layout(asset_root: &Utf8Path) -> Result<(), CoreError> {
    let path = ensure_mcp_json(asset_root)?;
    let mut doc = load_mcp_document(asset_root)?.unwrap_or_else(empty_mcp_document);
    let servers = doc
        .get_mut("mcpServers")
        .and_then(|v| v.as_object_mut())
        .expect("normalized");

    let legacy_dir = asset_root.join("mcp/servers");
    if legacy_dir.is_dir() {
        for entry in std::fs::read_dir(legacy_dir.as_std_path()).map_err(CoreError::Io)? {
            let entry = entry.map_err(CoreError::Io)?;
            if !entry.file_type().map_err(CoreError::Io)?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.ends_with(".json") {
                continue;
            }
            let raw = std::fs::read_to_string(entry.path()).map_err(CoreError::Io)?;
            let v: Value = serde_json::from_str(&raw).map_err(|e| CoreError::TemplateRender {
                template: entry.path().display().to_string(),
                reason: format!("解析失败: {e}"),
                hint: "修复或删除该 server 文件".to_owned(),
            })?;
            let key = v
                .get("name")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| name.strip_suffix(".json").unwrap_or(&name).to_string());
            if v.get("enabled").and_then(|x| x.as_bool()) == Some(false) {
                continue;
            }
            let entry_value = if let Some(cfg) = v.get("config") {
                cfg.clone()
            } else {
                v
            };
            servers.entry(key).or_insert(entry_value);
        }
    }

    let template_path = asset_root.join("mcp/cursor.mcp.template.json");
    if servers.is_empty() && template_path.is_file() {
        let raw = std::fs::read_to_string(template_path.as_std_path()).map_err(CoreError::Io)?;
        let t: Value = serde_json::from_str(&raw).map_err(|e| CoreError::TemplateRender {
            template: template_path.to_string(),
            reason: format!("模板解析失败: {e}"),
            hint: "修复 mcp/cursor.mcp.template.json".to_owned(),
        })?;
        if let Some(obj) = t.get("mcpServers").and_then(|x| x.as_object()) {
            for (k, v) in obj {
                servers.entry(k.clone()).or_insert(v.clone());
            }
        }
    }

    atomic_write_json(&path, &doc)?;

    let legacy_mcp = asset_root.join("mcp");
    if legacy_mcp.exists() {
        std::fs::remove_dir_all(legacy_mcp.as_std_path()).map_err(CoreError::Io)?;
    }
    Ok(())
}

/// 整文件下发:将源 `mcp.json` 写入平台路径（Hermes → `config.yaml` 的 `mcp_servers`）。
pub fn deploy_mcp_json_file(
    src: &Utf8Path,
    dest: &Utf8Path,
    plat: PlatformId,
) -> Result<(), CoreError> {
    if plat == PlatformId::Hermes {
        return hermes_config::deploy_all_from_mcp_json(src, dest);
    }
    ensure_platform_mcp_independent(dest, src)?;
    let Some(doc) = read_mcp_json(src)? else {
        return Err(CoreError::TemplateRender {
            template: src.to_string(),
            reason: "源 mcp.json 不存在".to_owned(),
            hint: format!("在 {} 创建 mcp.json", src.parent().unwrap_or(src)),
        });
    };
    atomic_write_json(dest, &normalize_mcp_document(doc))
}

/// 收回平台 MCP（Hermes 仅移除 `mcp_servers` 中 agent-manager 管理的条目，见 per-server retract）。
pub fn retract_platform_mcp_json(dest: &Utf8Path, plat: PlatformId) -> Result<(), CoreError> {
    retract_platform_mcp_json_with_source(dest, plat, None)
}

/// 收回平台 MCP 整文件；若与 `source_mcp` 同路径则 noop，避免误删 agent-manager 源。
pub fn retract_platform_mcp_json_with_source(
    dest: &Utf8Path,
    plat: PlatformId,
    source_mcp: Option<&Utf8Path>,
) -> Result<(), CoreError> {
    if plat == PlatformId::Hermes {
        // 整文件 retract 不删除 config.yaml；per-server 用 remove_server_on_platform。
        return Ok(());
    }
    if let Some(src) = source_mcp {
        if dest.as_str() == src.as_str() {
            return Ok(());
        }
    }
    if dest.is_file() {
        std::fs::remove_file(dest.as_std_path()).map_err(CoreError::Io)?;
    }
    Ok(())
}

/// 源与平台 MCP 是否一致（Hermes 比较 `mcp_servers` 全集）。
pub fn mcp_json_file_sync_state(src: &Utf8Path, dest: &Utf8Path, plat: PlatformId) -> McpSyncState {
    if plat == PlatformId::Hermes {
        let Some(asset_root) = src.parent() else {
            return McpSyncState::Broken;
        };
        let Ok(names) = list_server_names(asset_root) else {
            return McpSyncState::Broken;
        };
        if names.is_empty() {
            return McpSyncState::Unlinked;
        }
        let mut any_linked = false;
        let mut any_wrong = false;
        for name in names {
            match mcp_server_sync_state_on_platform(asset_root, &name, plat, dest) {
                McpSyncState::Linked => any_linked = true,
                McpSyncState::WrongValue | McpSyncState::Broken => any_wrong = true,
                McpSyncState::Unlinked => {}
            }
        }
        if any_wrong {
            return McpSyncState::WrongValue;
        }
        if any_linked {
            return McpSyncState::Linked;
        }
        return McpSyncState::Unlinked;
    }
    let Ok(Some(expected)) = read_mcp_json(src) else {
        return McpSyncState::Broken;
    };
    let Ok(Some(actual)) = read_mcp_json(dest) else {
        return McpSyncState::Unlinked;
    };
    if normalize_mcp_document(expected) == normalize_mcp_document(actual) {
        McpSyncState::Linked
    } else {
        McpSyncState::WrongValue
    }
}

/// 在 `mcp.json` 中增/改一条 server(值为 Cursor 原生 config 对象)。
pub fn upsert_server_in_document(
    asset_root: &Utf8Path,
    name: &str,
    config: Value,
) -> Result<(), CoreError> {
    let mut doc = load_mcp_document(asset_root)?.unwrap_or_else(empty_mcp_document);
    doc.get_mut("mcpServers")
        .and_then(|v| v.as_object_mut())
        .expect("normalized")
        .insert(name.to_owned(), config);
    save_mcp_document(asset_root, &doc)
}

/// 从 `mcp.json` 删除一条 server。
pub fn remove_server_from_document(asset_root: &Utf8Path, name: &str) -> Result<(), CoreError> {
    let mut doc = load_mcp_document(asset_root)?.unwrap_or_else(empty_mcp_document);
    doc.get_mut("mcpServers")
        .and_then(|v| v.as_object_mut())
        .expect("normalized")
        .remove(name);
    save_mcp_document(asset_root, &doc)
}

/// 读取单条 server 的 config 对象。
pub fn get_server_config(asset_root: &Utf8Path, name: &str) -> Result<Option<Value>, CoreError> {
    let Some(doc) = load_mcp_document(asset_root)? else {
        return Ok(None);
    };
    Ok(doc
        .get("mcpServers")
        .and_then(|v| v.as_object())
        .and_then(|m| m.get(name))
        .cloned())
}

/// 单条 server 的 transport 摘要(供 GUI 列表描述)。
pub fn server_transport_summary(config: &Value) -> String {
    if let Some(url) = config.get("url").and_then(|v| v.as_str()) {
        return format!("url: {url}");
    }
    let cmd = config.get("command").and_then(|v| v.as_str()).unwrap_or("");
    if cmd.is_empty() {
        return String::new();
    }
    let args = config
        .get("args")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    if args.is_empty() {
        format!("stdio: {cmd}")
    } else {
        format!("stdio: {cmd} {args}")
    }
}

/// 平台 `mcp.json` 是否与 agent-manager 源共用同一文件（symlink / 硬链接 / 同路径）。
pub fn platform_mcp_aliases_source(dest: &Utf8Path, source_mcp: &Utf8Path) -> bool {
    crate::path_independence::paths_alias(dest, source_mcp)
}

/// 若平台 MCP 文件与源共用 inode/路径，则 materialize 为独立副本（硬拷贝 JSON 内容）。
pub fn ensure_platform_mcp_independent(
    dest: &Utf8Path,
    source_mcp: &Utf8Path,
) -> Result<(), CoreError> {
    if !platform_mcp_aliases_source(dest, source_mcp) {
        return Ok(());
    }
    let doc = match read_mcp_json(dest)? {
        Some(d) => d,
        None => read_mcp_json(source_mcp)?.unwrap_or_else(empty_mcp_document),
    };
    if let Some(parent) = dest.parent() {
        crate::paths::ensure_parent_dir(parent)?;
    }
    if dest.exists() || std::fs::symlink_metadata(dest.as_std_path()).is_ok() {
        std::fs::remove_file(dest.as_std_path()).map_err(CoreError::Io)?;
    }
    atomic_write_json(dest, &normalize_mcp_document(doc))
}

/// 向平台 MCP 写入/更新单条 server(保留其它 key)。
pub fn upsert_server_on_platform(
    plat: PlatformId,
    dest: &Utf8Path,
    name: &str,
    config: &Value,
    source_mcp: Option<&Utf8Path>,
) -> Result<(), CoreError> {
    if let Some(src) = source_mcp.filter(|_| plat != PlatformId::Hermes) {
        ensure_platform_mcp_independent(dest, src)?;
    }
    match plat {
        PlatformId::Hermes => hermes_config::upsert_mcp_server(dest, name, config),
        _ => upsert_mcp_server_entry(dest, name, config.clone()),
    }
}

/// 从平台 MCP 移除单条 server(文件不存在则 noop)。
pub fn remove_server_on_platform(
    plat: PlatformId,
    dest: &Utf8Path,
    name: &str,
    source_mcp: Option<&Utf8Path>,
) -> Result<(), CoreError> {
    if let Some(src) = source_mcp.filter(|_| plat != PlatformId::Hermes) {
        ensure_platform_mcp_independent(dest, src)?;
    }
    match plat {
        PlatformId::Hermes => hermes_config::remove_mcp_server(dest, name),
        _ => remove_mcp_server_entry(dest, name),
    }
}

/// 源 `mcp.json` 中该 server 与平台 MCP 是否一致(key + 值)。
pub fn mcp_server_sync_state_on_platform(
    asset_root: &Utf8Path,
    server_name: &str,
    plat: PlatformId,
    dest: &Utf8Path,
) -> McpSyncState {
    let Ok(Some(expected)) = get_server_config(asset_root, server_name) else {
        return McpSyncState::Broken;
    };
    match plat {
        PlatformId::Hermes => hermes_config::mcp_server_sync_state(dest, server_name, &expected),
        _ => mcp_server_sync_state(dest, server_name, &expected),
    }
}

/// 包装:按平台 ID 比对单条 server 同步状态。
pub fn mcp_server_sync_state_for_platform(
    asset_root: &Utf8Path,
    server_name: &str,
    plat: PlatformId,
) -> McpSyncState {
    mcp_server_sync_state_for_platform_at(
        asset_root,
        server_name,
        plat,
        &crate::paths::global_deploy_base(),
    )
}

/// 指定下发根目录的 MCP 同步状态(项目作用域传仓库根)。
pub fn mcp_server_sync_state_for_platform_at(
    asset_root: &Utf8Path,
    server_name: &str,
    plat: PlatformId,
    deploy_base: &Utf8Path,
) -> McpSyncState {
    let Ok(adapter) = crate::platform::for_scope(plat, deploy_base) else {
        return McpSyncState::Broken;
    };
    mcp_server_sync_state_on_platform(asset_root, server_name, plat, &adapter.mcp_deploy_path())
}

/// 遗留 `~/.hermes/mcp.json` 合并进 `config.yaml`。
pub fn migrate_legacy_hermes_mcp_json(home: &Utf8Path) -> Result<HermesMigrateReport, CoreError> {
    hermes_config::migrate_legacy_hermes_mcp_json(home)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn migrate_legacy_servers_into_mcp_json() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(root.join("mcp/servers")).unwrap();
        fs::write(
            root.join("mcp/servers/foo.json"),
            r#"{"name":"foo","enabled":true,"config":{"command":"uvx"}}"#,
        )
        .unwrap();
        migrate_legacy_mcp_layout(&root).unwrap();
        assert!(root.join("mcp.json").is_file());
        assert!(!root.join("mcp").exists());
        let doc = load_mcp_document(&root).unwrap().unwrap();
        assert_eq!(doc["mcpServers"]["foo"]["command"].as_str(), Some("uvx"));
    }

    #[test]
    fn ordinary_layout_setup_leaves_legacy_mcp_directory_unmodified() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let legacy_server = root.join("mcp/servers/foo.json");
        fs::create_dir_all(legacy_server.parent().unwrap()).unwrap();
        let legacy_contents = r#"{"name":"foo","enabled":true,"config":{"command":"uvx"}}"#;
        fs::write(&legacy_server, legacy_contents).unwrap();

        crate::paths::ensure_asset_layout(&root).unwrap();

        assert_eq!(fs::read_to_string(&legacy_server).unwrap(), legacy_contents);
        assert!(!root.join("mcp.json").exists());
    }

    #[test]
    fn global_initialization_leaves_legacy_mcp_directory_unmodified() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let legacy_server = root.join("mcp/servers/foo.json");
        fs::create_dir_all(legacy_server.parent().unwrap()).unwrap();
        let legacy_contents = r#"{"name":"foo","enabled":true,"config":{"command":"uvx"}}"#;
        fs::write(&legacy_server, legacy_contents).unwrap();
        fs::create_dir_all(root.join("skills/kept")).unwrap();
        fs::write(root.join("skills/kept/SKILL.md"), "# kept").unwrap();
        let _root_guard = crate::test_env::EnvGuard::set("AGENT_MANAGER_ROOT", root.as_str());

        assert_eq!(crate::paths::init_user_asset_root().unwrap(), root);

        assert_eq!(fs::read_to_string(&legacy_server).unwrap(), legacy_contents);
        assert!(!root.join("mcp.json").exists());
    }

    #[test]
    fn upsert_gui_style_agent_manager_server() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        ensure_mcp_json(&root).unwrap();
        let cfg = serde_json::json!({
            "command": "agent-manager",
            "args": ["serve"]
        });
        upsert_server_in_document(&root, "agent-manager", cfg.clone()).unwrap();
        let stored = get_server_config(&root, "agent-manager")
            .unwrap()
            .expect("stored");
        assert_eq!(stored["command"].as_str(), Some("agent-manager"));
        assert_eq!(stored["args"].as_array().map(|a| a.len()), Some(1));
    }

    #[test]
    fn per_server_platform_upsert_and_sync_state() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let cfg = serde_json::json!({ "command": "uvx", "args": ["mcp-server"] });
        upsert_server_in_document(&root, "svc-a", cfg.clone()).unwrap();
        let plat_mcp = root.join("plat-mcp.json");
        upsert_server_on_platform(PlatformId::Cursor, &plat_mcp, "svc-a", &cfg, None).unwrap();
        assert_eq!(
            mcp_server_sync_state_on_platform(&root, "svc-a", PlatformId::Cursor, &plat_mcp),
            McpSyncState::Linked
        );
        remove_server_on_platform(PlatformId::Cursor, &plat_mcp, "svc-a", None).unwrap();
        assert_eq!(
            mcp_server_sync_state_on_platform(&root, "svc-a", PlatformId::Cursor, &plat_mcp),
            McpSyncState::Unlinked
        );
    }

    /// 删一条 server 必须保留其它 server 与顶层字段。
    /// 这是 GUI「点击 Cursor 图标删除整个 mcp 记录」问题的核心防护：
    /// 任何路径都不应清空 mcpServers 整张表。
    #[test]
    fn remove_one_server_keeps_others_and_top_level_fields() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();

        let cfg_a = serde_json::json!({ "command": "uvx", "args": ["a"] });
        let cfg_b = serde_json::json!({ "url": "https://example.com/b" });
        let cfg_c = serde_json::json!({ "type": "stdio", "command": "c" });

        upsert_server_in_document(&root, "alpha", cfg_a.clone()).unwrap();
        upsert_server_in_document(&root, "beta", cfg_b.clone()).unwrap();
        upsert_server_in_document(&root, "gamma", cfg_c.clone()).unwrap();

        // 删 alpha
        remove_server_from_document(&root, "alpha").unwrap();

        let names = list_server_names(&root).unwrap();
        assert_eq!(names, vec!["beta".to_string(), "gamma".to_string()]);

        // 顶层 mcpServers 仍存在
        let doc = load_mcp_document(&root).unwrap().unwrap();
        assert!(doc.get("mcpServers").and_then(|v| v.as_object()).is_some());
        assert_eq!(doc["mcpServers"]["beta"], cfg_b);
        assert_eq!(doc["mcpServers"]["gamma"], cfg_c);
        assert!(doc["mcpServers"].get("alpha").is_none());

        // 删不存在的 server：noop，其它仍保留
        remove_server_from_document(&root, "alpha").unwrap();
        remove_server_from_document(&root, "nonexistent").unwrap();
        let names = list_server_names(&root).unwrap();
        assert_eq!(names, vec!["beta".to_string(), "gamma".to_string()]);
    }

    /// IDE 平台 mcp.json：删一条 server 不影响其它 server，也不影响 mcp.json 文件本身存在。
    #[test]
    fn remove_platform_server_keeps_other_servers() {
        let tmp = TempDir::new().unwrap();
        let plat = tmp.path().join(".cursor").join("mcp.json");
        std::fs::create_dir_all(plat.parent().unwrap()).unwrap();

        let cfg_a = serde_json::json!({ "command": "uvx", "args": ["a"] });
        let cfg_b = serde_json::json!({ "command": "uvx", "args": ["b"] });

        upsert_server_on_platform(
            PlatformId::Cursor,
            &Utf8PathBuf::from_path_buf(plat.clone()).unwrap(),
            "alpha",
            &cfg_a,
            None,
        )
        .unwrap();
        upsert_server_on_platform(
            PlatformId::Cursor,
            &Utf8PathBuf::from_path_buf(plat.clone()).unwrap(),
            "beta",
            &cfg_b,
            None,
        )
        .unwrap();

        remove_server_on_platform(
            PlatformId::Cursor,
            &Utf8PathBuf::from_path_buf(plat.clone()).unwrap(),
            "alpha",
            None,
        )
        .unwrap();

        let raw = std::fs::read_to_string(&plat).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(doc["mcpServers"].get("alpha").is_none());
        assert_eq!(doc["mcpServers"]["beta"], cfg_b);
    }

    #[test]
    fn hermes_upsert_writes_config_yaml() {
        let tmp = TempDir::new().unwrap();
        let home = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(home.join(".hermes")).unwrap();
        let root = home.join("agent-manager-asset");
        fs::create_dir_all(&root).unwrap();
        let cfg = serde_json::json!({
            "type": "stdio",
            "command": "npx",
            "args": ["-y", "pkg"]
        });
        upsert_server_in_document(&root, "svc-a", cfg.clone()).unwrap();
        let dest = hermes_config::hermes_config_path_at(&home);
        upsert_server_on_platform(PlatformId::Hermes, &dest, "svc-a", &cfg, None).unwrap();
        assert_eq!(
            mcp_server_sync_state_on_platform(&root, "svc-a", PlatformId::Hermes, &dest),
            McpSyncState::Linked
        );
        let raw = fs::read_to_string(dest.as_std_path()).unwrap();
        assert!(raw.contains("mcp_servers:"));
        assert!(!raw.contains("type:"));
    }

    /// 平台 mcp.json 若 symlink 到 agent-manager 源，retract 单条 server 不得改动源文件。
    #[test]
    #[cfg(unix)]
    fn remove_platform_server_does_not_mutate_symlinked_source() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let source_mcp = root.join("mcp.json");
        let plat_dir = root.join(".cursor");
        std::fs::create_dir_all(&plat_dir).unwrap();
        let plat_mcp = plat_dir.join("mcp.json");

        let cfg_a = serde_json::json!({ "command": "uvx", "args": ["a"] });
        let cfg_b = serde_json::json!({ "command": "uvx", "args": ["b"] });
        upsert_server_in_document(&root, "alpha", cfg_a.clone()).unwrap();
        upsert_server_in_document(&root, "beta", cfg_b.clone()).unwrap();

        std::os::unix::fs::symlink(&source_mcp, plat_mcp.as_std_path()).unwrap();
        assert!(platform_mcp_aliases_source(&plat_mcp, &source_mcp));

        remove_server_on_platform(PlatformId::Cursor, &plat_mcp, "alpha", Some(&source_mcp))
            .unwrap();

        let source_names = list_server_names(&root).unwrap();
        assert_eq!(
            source_names,
            vec!["alpha".to_string(), "beta".to_string()],
            "源 mcp.json 不得因平台 retract 丢失条目"
        );

        let plat_raw = std::fs::read_to_string(plat_mcp.as_std_path()).unwrap();
        let plat_doc: serde_json::Value = serde_json::from_str(&plat_raw).unwrap();
        assert!(plat_doc["mcpServers"].get("alpha").is_none());
        assert_eq!(plat_doc["mcpServers"]["beta"], cfg_b);
        assert!(
            std::fs::symlink_metadata(plat_mcp.as_std_path())
                .map(|m| !m.file_type().is_symlink())
                .unwrap_or(false),
            "materialize 后平台文件应为独立副本"
        );
    }

    #[test]
    fn get_server_config_from_deploy_file_reads_mcp_json() {
        let tmp = TempDir::new().unwrap();
        let path = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf())
            .unwrap()
            .join("mcp.json");
        crate::paths::ensure_parent_dir(&path).unwrap();
        fs::write(
            &path,
            r#"{"mcpServers":{"Chrome DevTools MCP":{"command":"npx","args":["chrome-devtools-mcp"]}}}"#,
        )
        .unwrap();

        let cfg = get_server_config_from_deploy_file(&path, "Chrome DevTools MCP").unwrap();
        assert_eq!(cfg["command"].as_str(), Some("npx"));
    }

    #[test]
    fn get_server_config_from_deploy_file_errors_when_server_missing() {
        let tmp = TempDir::new().unwrap();
        let path = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf())
            .unwrap()
            .join("mcp.json");
        crate::paths::ensure_parent_dir(&path).unwrap();
        fs::write(&path, r#"{"mcpServers":{}}"#).unwrap();

        let err = get_server_config_from_deploy_file(&path, "missing").unwrap_err();
        assert!(err.to_string().contains("missing"));
    }
}
