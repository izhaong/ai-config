//! 用户全局 MCP 配置:单一 `mcp.json`(明文 `mcpServers`,无 secrets 注入)。

use camino::{Utf8Path, Utf8PathBuf};
use serde_json::{Map, Value};

use crate::error::CoreError;
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

/// 整文件下发:将源 `mcp.json` 写入平台路径。
pub fn deploy_mcp_json_file(src: &Utf8Path, dest: &Utf8Path) -> Result<(), CoreError> {
    let Some(doc) = read_mcp_json(src)? else {
        return Err(CoreError::TemplateRender {
            template: src.to_string(),
            reason: "源 mcp.json 不存在".to_owned(),
            hint: format!("在 {} 创建 mcp.json", src.parent().unwrap_or(src)),
        });
    };
    atomic_write_json(dest, &normalize_mcp_document(doc))
}

/// 收回:删除平台 `mcp.json`。
pub fn retract_platform_mcp_json(dest: &Utf8Path) -> Result<(), CoreError> {
    if dest.is_file() {
        std::fs::remove_file(dest.as_std_path()).map_err(CoreError::Io)?;
    }
    Ok(())
}

/// 源与平台 `mcp.json` 是否一致。
pub fn mcp_json_file_sync_state(src: &Utf8Path, dest: &Utf8Path) -> McpSyncState {
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

/// 向平台 `mcp.json` 写入/更新单条 server(保留其它 key)。
pub fn upsert_server_on_platform(
    dest: &Utf8Path,
    name: &str,
    config: &Value,
) -> Result<(), CoreError> {
    upsert_mcp_server_entry(dest, name, config.clone())
}

/// 从平台 `mcp.json` 移除单条 server(文件不存在则 noop)。
pub fn remove_server_on_platform(dest: &Utf8Path, name: &str) -> Result<(), CoreError> {
    remove_mcp_server_entry(dest, name)
}

/// 源 `mcp.json` 中该 server 与平台 `mcp.json` 是否一致(key + 值)。
pub fn mcp_server_sync_state_on_platform(
    asset_root: &Utf8Path,
    server_name: &str,
    dest: &Utf8Path,
) -> McpSyncState {
    let Ok(Some(expected)) = get_server_config(asset_root, server_name) else {
        return McpSyncState::Broken;
    };
    mcp_server_sync_state(dest, server_name, &expected)
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
    mcp_server_sync_state_on_platform(asset_root, server_name, &adapter.mcp_json_path())
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
    fn per_server_platform_upsert_and_sync_state() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let cfg = serde_json::json!({ "command": "uvx", "args": ["mcp-server"] });
        upsert_server_in_document(&root, "svc-a", cfg.clone()).unwrap();
        let plat_mcp = root.join("plat-mcp.json");
        upsert_server_on_platform(&plat_mcp, "svc-a", &cfg).unwrap();
        assert_eq!(
            mcp_server_sync_state_on_platform(&root, "svc-a", &plat_mcp),
            McpSyncState::Linked
        );
        remove_server_on_platform(&plat_mcp, "svc-a").unwrap();
        assert_eq!(
            mcp_server_sync_state_on_platform(&root, "svc-a", &plat_mcp),
            McpSyncState::Unlinked
        );
    }
}
