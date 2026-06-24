//! Hermes 平台 MCP：写入 `~/.hermes/config.yaml` 的 `mcp_servers` 段（非 mcp.json）。
//!
//! 官方约定见 Hermes Agent MCP 文档与 `hermes_cli/mcp_config.py`。

use camino::{Utf8Path, Utf8PathBuf};
use serde_json::{Map, Value};
use serde_yaml::{Mapping, Value as YamlValue};

use crate::error::CoreError;
use crate::paths;
use crate::template::McpSyncState;

/// Hermes 用户级 MCP 配置路径（始终 `$HOME/.hermes/config.yaml`）。
pub fn hermes_config_path() -> Utf8PathBuf {
    hermes_config_path_at(&paths::global_deploy_base())
}

pub fn hermes_config_path_at(home: &Utf8Path) -> Utf8PathBuf {
    home.join(".hermes/config.yaml")
}

/// Cursor/Codex 风格 JSON config → Hermes `mcp_servers` 条目（去掉 `type` 等）。
pub fn normalize_server_for_hermes(config: &Value) -> Value {
    let Some(obj) = config.as_object() else {
        return config.clone();
    };
    let mut out = Map::with_capacity(obj.len());
    for (k, v) in obj {
        if k == "type" {
            continue;
        }
        out.insert(k.clone(), v.clone());
    }
    Value::Object(out)
}

fn yaml_err(path: &Utf8Path, reason: impl Into<String>, hint: impl Into<String>) -> CoreError {
    CoreError::TemplateRender {
        template: path.as_str().to_owned(),
        reason: reason.into(),
        hint: hint.into(),
    }
}

fn read_config_yaml(path: &Utf8Path) -> Result<Option<YamlValue>, CoreError> {
    match std::fs::read_to_string(path.as_std_path()) {
        Ok(raw) => {
            let v = serde_yaml::from_str(&raw).map_err(|e| {
                yaml_err(
                    path,
                    format!("config.yaml 解析失败: {e}"),
                    "修复 YAML 或从 config.yaml.ai-config.bak.* 恢复",
                )
            })?;
            Ok(Some(v))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CoreError::Io(e)),
    }
}

fn ensure_root_mapping(_path: &Utf8Path, root: Option<YamlValue>) -> Mapping {
    match root {
        Some(YamlValue::Mapping(m)) => m,
        Some(_) => Mapping::new(),
        None => Mapping::new(),
    }
}

fn backup_config_if_exists(path: &Utf8Path) -> Result<(), CoreError> {
    if !path.is_file() {
        return Ok(());
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let bak = path.with_extension(format!("yaml.ai-config.bak.{ts}"));
    std::fs::copy(path.as_std_path(), bak.as_std_path()).map_err(CoreError::Io)?;
    Ok(())
}

fn atomic_write_yaml(path: &Utf8Path, doc: &YamlValue) -> Result<(), CoreError> {
    paths::ensure_parent_dir(path)?;
    backup_config_if_exists(path)?;
    let serialized = serde_yaml::to_string(doc).map_err(|e| {
        yaml_err(
            path,
            format!("config.yaml 序列化失败: {e}"),
            "检查 mcp_servers 字段类型",
        )
    })?;
    let tmp = path.with_extension("yaml.tmp");
    std::fs::write(tmp.as_std_path(), serialized.as_bytes()).map_err(CoreError::Io)?;
    if let Err(e) = std::fs::rename(tmp.as_std_path(), path.as_std_path()) {
        let _ = std::fs::remove_file(tmp.as_std_path());
        return Err(CoreError::Io(e));
    }
    Ok(())
}

fn yaml_entry_to_json(v: &YamlValue) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

fn json_entry_to_yaml(v: &Value) -> Result<YamlValue, CoreError> {
    serde_json::from_value(v.clone()).map_err(CoreError::Json)
}

/// 向 `config.yaml` 的 `mcp_servers` 写入/更新单条 server。
pub fn upsert_mcp_server(
    config_path: &Utf8Path,
    server_name: &str,
    config: &Value,
) -> Result<(), CoreError> {
    let normalized = normalize_server_for_hermes(config);
    let entry_yaml = json_entry_to_yaml(&normalized)?;
    let mut root = ensure_root_mapping(config_path, read_config_yaml(config_path)?);
    let servers = root
        .entry(YamlValue::String("mcp_servers".into()))
        .or_insert_with(|| YamlValue::Mapping(Mapping::new()));
    let servers_map = match servers {
        YamlValue::Mapping(m) => m,
        _ => {
            return Err(yaml_err(
                config_path,
                "`mcp_servers` 不是 mapping",
                "把 mcp_servers 改为 YAML mapping",
            ))
        }
    };
    servers_map.insert(YamlValue::String(server_name.into()), entry_yaml);
    atomic_write_yaml(config_path, &YamlValue::Mapping(root))
}

/// 从 `config.yaml` 的 `mcp_servers` 移除单条 server。
pub fn remove_mcp_server(config_path: &Utf8Path, server_name: &str) -> Result<(), CoreError> {
    let Some(mut root) = read_config_yaml(config_path)? else {
        return Ok(());
    };
    let YamlValue::Mapping(ref mut map) = root else {
        return Ok(());
    };
    let Some(servers) = map.get_mut(YamlValue::String("mcp_servers".into())) else {
        return Ok(());
    };
    let YamlValue::Mapping(ref mut servers_map) = servers else {
        return Ok(());
    };
    servers_map.remove(YamlValue::String(server_name.into()));
    if servers_map.is_empty() {
        map.remove(YamlValue::String("mcp_servers".into()));
    }
    atomic_write_yaml(config_path, &root)
}

/// 从 Hermes `config.yaml` 读取单条 `mcp_servers` 配置。
pub fn get_mcp_server_config(
    config_path: &Utf8Path,
    server_name: &str,
) -> Result<Value, CoreError> {
    let raw = std::fs::read_to_string(config_path.as_std_path()).map_err(CoreError::Io)?;
    let doc: YamlValue = serde_yaml::from_str(&raw).map_err(|e| CoreError::TemplateRender {
        template: config_path.to_string(),
        reason: format!("解析 config.yaml: {e}"),
        hint: "修复 Hermes config.yaml".into(),
    })?;
    let YamlValue::Mapping(map) = doc else {
        return Err(CoreError::AssetNotFound {
            kind: crate::model::AssetKind::Mcp,
            name: server_name.into(),
            hint: "config.yaml 无根 mapping".into(),
        });
    };
    let Some(YamlValue::Mapping(servers)) = map.get(YamlValue::String("mcp_servers".into())) else {
        return Err(CoreError::AssetNotFound {
            kind: crate::model::AssetKind::Mcp,
            name: server_name.into(),
            hint: "config.yaml 无 mcp_servers".into(),
        });
    };
    let Some(entry) = servers.get(YamlValue::String(server_name.into())) else {
        return Err(CoreError::AssetNotFound {
            kind: crate::model::AssetKind::Mcp,
            name: server_name.into(),
            hint: format!("Hermes mcp_servers 中无 `{server_name}`"),
        });
    };
    serde_json::to_value(entry).map_err(CoreError::Json)
}

/// 判定单条 server 是否已同步到 Hermes `config.yaml`。
pub fn mcp_server_sync_state(
    config_path: &Utf8Path,
    server_name: &str,
    expected_entry: &Value,
) -> McpSyncState {
    let expected = normalize_server_for_hermes(expected_entry);
    let Some(root) = read_config_yaml(config_path).ok().flatten() else {
        return McpSyncState::Unlinked;
    };
    let YamlValue::Mapping(map) = root else {
        return McpSyncState::Broken;
    };
    let Some(servers) = map.get(YamlValue::String("mcp_servers".into())) else {
        return McpSyncState::Unlinked;
    };
    let YamlValue::Mapping(servers_map) = servers else {
        return McpSyncState::Broken;
    };
    let Some(actual_yaml) = servers_map.get(YamlValue::String(server_name.into())) else {
        return McpSyncState::Unlinked;
    };
    let actual = yaml_entry_to_json(actual_yaml);
    if actual == expected {
        McpSyncState::Linked
    } else {
        McpSyncState::WrongValue
    }
}

/// 将源 `mcp.json` 中全部 `mcpServers` 合并进 Hermes `config.yaml`。
pub fn deploy_all_from_mcp_json(
    src_mcp_json: &Utf8Path,
    config_path: &Utf8Path,
) -> Result<(), CoreError> {
    use crate::template::read_mcp_json;
    let Some(doc) = read_mcp_json(src_mcp_json)? else {
        return Err(CoreError::TemplateRender {
            template: src_mcp_json.to_string(),
            reason: "源 mcp.json 不存在".to_owned(),
            hint: format!(
                "在 {} 创建 mcp.json",
                src_mcp_json.parent().unwrap_or(src_mcp_json)
            ),
        });
    };
    let Some(servers) = doc.get("mcpServers").and_then(|v| v.as_object()) else {
        return Ok(());
    };
    for (name, cfg) in servers {
        upsert_mcp_server(config_path, name, cfg)?;
    }
    Ok(())
}

/// 官方默认 skills 目录：`$HOME/.hermes/skills`。
pub fn default_hermes_skills_dir_at(home: &Utf8Path) -> Utf8PathBuf {
    home.join(".hermes/skills")
}

/// 非默认 skills 目录写入 `config.yaml` → `skills.external_dirs`（Hermes 官方发现机制）。
pub fn ensure_external_skills_dir(
    config_path: &Utf8Path,
    skills_dir: &Utf8Path,
) -> Result<(), CoreError> {
    let default = default_hermes_skills_dir_at(&paths::global_deploy_base());
    if skills_dir == default {
        return Ok(());
    }
    let dir_str = skills_dir.as_str();
    let mut root = ensure_root_mapping(config_path, read_config_yaml(config_path)?);
    let skills = root
        .entry(YamlValue::String("skills".into()))
        .or_insert_with(|| YamlValue::Mapping(Mapping::new()));
    let YamlValue::Mapping(ref mut skills_map) = skills else {
        return Err(yaml_err(
            config_path,
            "`skills` 不是 mapping",
            "把 skills 改为 YAML mapping",
        ));
    };
    let external = skills_map
        .entry(YamlValue::String("external_dirs".into()))
        .or_insert_with(|| YamlValue::Sequence(Vec::new()));
    let YamlValue::Sequence(ref mut seq) = external else {
        return Err(yaml_err(
            config_path,
            "`skills.external_dirs` 不是列表",
            "把 external_dirs 改为 YAML 列表",
        ));
    };
    let entry = YamlValue::String(dir_str.into());
    if !seq.contains(&entry) {
        seq.push(entry);
    }
    atomic_write_yaml(config_path, &YamlValue::Mapping(root))
}

/// Hermes skill 软链成功后调用：自定义 `HERMES_SKILLS_DIR` 时同步 `external_dirs`。
pub fn after_skill_deploy(skills_dir: &Utf8Path) -> Result<(), CoreError> {
    ensure_external_skills_dir(&hermes_config_path(), skills_dir)
}

/// 遗留 `~/.hermes/mcp.json` → `config.yaml` 的 `mcp_servers`（成功后 rename 为 .bak）。
#[derive(Debug, Clone, Default)]
pub struct HermesMigrateReport {
    pub merged: Vec<String>,
    pub skipped_conflict: Vec<String>,
    pub legacy_renamed: Option<String>,
}

pub fn migrate_legacy_hermes_mcp_json(home: &Utf8Path) -> Result<HermesMigrateReport, CoreError> {
    use crate::template::read_mcp_json;
    let legacy = home.join(".hermes/mcp.json");
    let mut report = HermesMigrateReport::default();
    if !legacy.is_file() {
        return Ok(report);
    }
    let Some(doc) = read_mcp_json(&legacy)? else {
        return Ok(report);
    };
    let Some(servers) = doc.get("mcpServers").and_then(|v| v.as_object()) else {
        return Ok(report);
    };
    let config_path = hermes_config_path_at(home);
    for (name, cfg) in servers {
        let normalized = normalize_server_for_hermes(cfg);
        let state = mcp_server_sync_state(&config_path, name, cfg);
        match state {
            McpSyncState::Unlinked => {
                upsert_mcp_server(&config_path, name, &normalized)?;
                report.merged.push(name.clone());
            }
            McpSyncState::Linked => {
                report
                    .skipped_conflict
                    .push(format!("{name}: 已存在且一致"));
            }
            McpSyncState::WrongValue => {
                report
                    .skipped_conflict
                    .push(format!("{name}: config.yaml 中已有不同配置，跳过"));
            }
            McpSyncState::Broken => {
                report
                    .skipped_conflict
                    .push(format!("{name}: config.yaml 损坏，跳过"));
            }
        }
    }
    if !report.merged.is_empty() || legacy.is_file() {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let bak = legacy.with_extension(format!("json.ai-config.bak.{ts}"));
        std::fs::rename(legacy.as_std_path(), bak.as_std_path()).map_err(CoreError::Io)?;
        report.legacy_renamed = Some(bak.to_string());
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn normalize_strips_type_field() {
        let cfg = serde_json::json!({
            "type": "stdio",
            "command": "npx",
            "args": ["-y", "pkg"]
        });
        let out = normalize_server_for_hermes(&cfg);
        assert!(out.get("type").is_none());
        assert_eq!(out["command"].as_str(), Some("npx"));
    }

    #[test]
    fn upsert_remove_and_sync_state() {
        let tmp = TempDir::new().unwrap();
        let home = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(home.join(".hermes")).unwrap();
        let cfg_path = hermes_config_path_at(&home);
        fs::write(cfg_path.as_std_path(), "model:\n  default: test\n").unwrap();
        let cfg = serde_json::json!({ "command": "uvx", "args": ["mcp-server"] });
        upsert_mcp_server(&cfg_path, "svc-a", &cfg).unwrap();
        assert_eq!(
            mcp_server_sync_state(&cfg_path, "svc-a", &cfg),
            McpSyncState::Linked
        );
        let raw = fs::read_to_string(cfg_path.as_std_path()).unwrap();
        assert!(raw.contains("mcp_servers:"));
        assert!(raw.contains("model:"));
        remove_mcp_server(&cfg_path, "svc-a").unwrap();
        assert_eq!(
            mcp_server_sync_state(&cfg_path, "svc-a", &cfg),
            McpSyncState::Unlinked
        );
    }

    #[test]
    fn migrate_legacy_mcp_json() {
        let tmp = TempDir::new().unwrap();
        let home = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(home.join(".hermes")).unwrap();
        fs::write(
            home.join(".hermes/mcp.json"),
            r#"{"mcpServers":{"foo":{"command":"echo"}}}"#,
        )
        .unwrap();
        let report = migrate_legacy_hermes_mcp_json(&home).unwrap();
        assert_eq!(report.merged, vec!["foo".to_string()]);
        assert!(!home.join(".hermes/mcp.json").exists());
        assert_eq!(
            mcp_server_sync_state(
                &hermes_config_path_at(&home),
                "foo",
                &serde_json::json!({"command":"echo"})
            ),
            McpSyncState::Linked
        );
    }

    #[test]
    fn ensure_external_skills_dir_appends_to_config() {
        let tmp = TempDir::new().unwrap();
        let home = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(home.join(".hermes")).unwrap();
        let cfg_path = hermes_config_path_at(&home);
        let custom = home.join("custom-skills");
        fs::create_dir_all(&custom).unwrap();

        // 使用与 global_deploy_base 无关的路径比较：直接测 ensure 逻辑
        let _home_guard = crate::test_env::EnvGuard::set("HOME", home.as_str());
        ensure_external_skills_dir(&cfg_path, &custom).unwrap();
        let raw = fs::read_to_string(cfg_path.as_std_path()).unwrap();
        assert!(raw.contains("external_dirs:"));
        assert!(raw.contains("custom-skills"));
    }

    #[test]
    fn get_mcp_server_config_reads_yaml_entry() {
        let tmp = TempDir::new().unwrap();
        let home = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(home.join(".hermes")).unwrap();
        let cfg_path = hermes_config_path_at(&home);
        fs::write(
            cfg_path.as_std_path(),
            "mcp_servers:\n  svc-a:\n    command: uvx\n    args:\n      - demo\n",
        )
        .unwrap();

        let cfg = get_mcp_server_config(&cfg_path, "svc-a").unwrap();
        assert_eq!(cfg["command"].as_str(), Some("uvx"));
        assert_eq!(cfg["args"][0].as_str(), Some("demo"));
    }
}
