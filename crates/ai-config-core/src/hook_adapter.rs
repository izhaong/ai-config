//! Hook 分平台下发 / 收回（Cursor hooks.json、Codex hooks.json、Claude settings、Hermes config.yaml）。

use std::fs;

use camino::{Utf8Path, Utf8PathBuf};
use serde_json::{json, Map, Value};
use serde_yaml::{Mapping, Value as YamlValue};

use crate::error::CoreError;
use crate::hook::{self, HookScriptSpec};
use crate::model::PlatformId;
use crate::paths;
use crate::platform;
use crate::template::atomic_write_json;

const MANAGED_BY: &str = "ai-config";

/// 平台 hooks 脚本目录（如 `~/.cursor/hooks/`）。
pub fn platform_hooks_dir(deploy_base: &Utf8Path, plat: PlatformId) -> Utf8PathBuf {
    match plat {
        PlatformId::Cursor => deploy_base.join(".cursor/hooks"),
        PlatformId::Codex => deploy_base.join(".codex/hooks"),
        PlatformId::Claude => claude_hooks_script_dir(deploy_base),
        PlatformId::Hermes => paths::home_dir().join(".hermes/agent-hooks"),
        PlatformId::AiConfig => hook::hooks_dir(deploy_base),
    }
}

/// 项目已用 `.cursor/hooks/` 作为 Claude/Cursor 共享脚本目录。
pub fn project_uses_shared_cursor_hooks(deploy_base: &Utf8Path) -> bool {
    if deploy_base == paths::home_dir() {
        return false;
    }
    let settings = deploy_base.join(".claude/settings.json");
    if settings.is_file() {
        if let Ok(content) = fs::read_to_string(settings.as_std_path()) {
            if content.contains(".cursor/hooks/") {
                return true;
            }
        }
    }
    deploy_base.join(".cursor/hooks").is_dir()
}

fn claude_hooks_script_dir(deploy_base: &Utf8Path) -> Utf8PathBuf {
    if project_uses_shared_cursor_hooks(deploy_base) {
        deploy_base.join(".cursor/hooks")
    } else {
        deploy_base.join(".claude/hooks")
    }
}

/// 平台侧脚本文件路径（扁平 `hooks/<filename>`，与 Cursor `command` 一致）。
pub fn platform_script_path(
    deploy_base: &Utf8Path,
    plat: PlatformId,
    script_filename: &str,
) -> Utf8PathBuf {
    platform_hooks_dir(deploy_base, plat).join(script_filename)
}

/// 兼容旧调用：返回脚本文件路径。
pub fn platform_scripts_dir(
    deploy_base: &Utf8Path,
    plat: PlatformId,
    script_filename: &str,
) -> Utf8PathBuf {
    platform_script_path(deploy_base, plat, script_filename)
}

pub fn platform_config_path_for(deploy_base: &Utf8Path, plat: PlatformId) -> Utf8PathBuf {
    platform_config_path(deploy_base, plat)
}

fn platform_config_path(deploy_base: &Utf8Path, plat: PlatformId) -> Utf8PathBuf {
    match plat {
        PlatformId::Cursor => deploy_base.join(".cursor/hooks.json"),
        PlatformId::Codex => deploy_base.join(".codex/hooks.json"),
        PlatformId::Claude => deploy_base.join(".claude/settings.json"),
        PlatformId::Hermes => paths::home_dir().join(".hermes/config.yaml"),
        PlatformId::AiConfig => Utf8PathBuf::new(),
    }
}

pub fn deploy(
    asset_root: &Utf8Path,
    deploy_base: &Utf8Path,
    script_filename: &str,
    plat: PlatformId,
) -> Result<String, CoreError> {
    let spec = hook::load_spec(asset_root, script_filename)?;
    if !hook::enabled_on_platform(&spec, plat) {
        return Err(CoreError::UnsupportedAsset {
            platform: plat,
            asset: crate::model::AssetKind::Hook,
            hint: format!(
                "hook `{script_filename}` 未启用平台 {}",
                platform::platform_label(plat)
            ),
        });
    }
    if !platform::supports_at_scope(plat, crate::model::AssetKind::Hook, deploy_base) {
        return Err(CoreError::UnsupportedAsset {
            platform: plat,
            asset: crate::model::AssetKind::Hook,
            hint: platform::capability_skip_reason(plat, crate::model::AssetKind::Hook),
        });
    }

    copy_script(&spec, deploy_base, plat)?;

    match plat {
        PlatformId::Cursor | PlatformId::Codex => {
            merge_cursor_codex_hooks(deploy_base, plat, &spec)?;
        }
        PlatformId::Claude => merge_claude_settings(deploy_base, &spec)?,
        PlatformId::Hermes => merge_hermes_config(&spec)?,
        PlatformId::AiConfig => {}
    }

    let dest = platform_config_path(deploy_base, plat);
    Ok(format!(
        "Hook `{script_filename}` → {} OK ({})",
        platform::platform_label(plat),
        if plat == PlatformId::AiConfig {
            spec.script_path.as_str()
        } else {
            dest.as_str()
        }
    ))
}

pub fn retract(
    asset_root: &Utf8Path,
    deploy_base: &Utf8Path,
    script_filename: &str,
    plat: PlatformId,
) -> Result<String, CoreError> {
    let _ = asset_root;
    let token = hook::managed_command_token(script_filename);
    let script_dest = platform_script_path(deploy_base, plat, script_filename);

    match plat {
        PlatformId::Cursor | PlatformId::Codex => {
            let path = platform_config_path(deploy_base, plat);
            if path.is_file() {
                let existing = read_json(&path)?;
                let merged = remove_cursor_codex_managed(existing, &token)?;
                if merged["hooks"].as_object().is_some_and(|m| m.is_empty()) && path.is_file() {
                    let _ = fs::remove_file(path.as_std_path());
                } else {
                    atomic_write_json(&path, &merged)?;
                }
            }
        }
        PlatformId::Claude => {
            let path = platform_config_path(deploy_base, plat);
            if path.is_file() {
                let existing = read_json(&path)?;
                let merged = remove_claude_managed(existing, &token)?;
                atomic_write_json(&path, &merged)?;
            }
        }
        PlatformId::Hermes => remove_hermes_managed(script_filename)?,
        PlatformId::AiConfig => {}
    }

    if script_dest.exists() {
        let shared_cursor = plat == PlatformId::Claude
            && project_uses_shared_cursor_hooks(deploy_base)
            && script_dest.as_str().contains("/.cursor/hooks/");
        if !shared_cursor {
            if script_dest.is_dir() {
                fs::remove_dir_all(script_dest.as_std_path()).map_err(CoreError::Io)?;
            } else {
                fs::remove_file(script_dest.as_std_path()).map_err(CoreError::Io)?;
            }
        }
    }

    Ok(format!(
        "Hook `{script_filename}` ← {} OK",
        platform::platform_label(plat)
    ))
}

pub fn is_deployed(deploy_base: &Utf8Path, plat: PlatformId, script_filename: &str) -> bool {
    let token = hook::managed_command_token(script_filename);
    match plat {
        PlatformId::Cursor | PlatformId::Codex | PlatformId::Claude => {
            config_contains_token(&platform_config_path(deploy_base, plat), &token)
        }
        PlatformId::Hermes => hermes_contains_hook(script_filename),
        PlatformId::AiConfig => hook::script_path(deploy_base, script_filename).is_file(),
    }
}

/// 从源平台读取某脚本的 hooks 绑定（用于跨平台拷贝）。
pub fn read_platform_bindings(
    deploy_base: &Utf8Path,
    plat: PlatformId,
    script_filename: &str,
) -> Result<Vec<hook::HookBinding>, CoreError> {
    match plat {
        PlatformId::AiConfig => Ok(hook::load_spec(deploy_base, script_filename)?.bindings),
        PlatformId::Cursor | PlatformId::Codex => {
            let path = platform_config_path(deploy_base, plat);
            if !path.is_file() {
                return Ok(Vec::new());
            }
            let doc = read_json(&path)?;
            Ok(extract_json_bindings(
                &doc,
                script_filename,
                plat == PlatformId::Cursor,
            ))
        }
        PlatformId::Claude => {
            let path = platform_config_path(deploy_base, plat);
            if !path.is_file() {
                return Ok(Vec::new());
            }
            let doc = read_json(&path)?;
            Ok(extract_claude_bindings(&doc, script_filename))
        }
        PlatformId::Hermes => read_hermes_bindings(script_filename),
    }
}

/// 跨平台拷贝 hook：脚本文件 + 目标平台配置 merge（与 skill 平台镜像下发同类）。
pub fn deploy_between_platforms(
    deploy_base: &Utf8Path,
    script_filename: &str,
    from_plat: PlatformId,
    to_plat: PlatformId,
) -> Result<String, CoreError> {
    if from_plat == to_plat {
        return Err(CoreError::InvalidPath("来源平台与目标平台不能相同".into()));
    }
    let from_script = platform_script_path(deploy_base, from_plat, script_filename);
    if !from_script.is_file() {
        return Err(CoreError::AssetNotFound {
            kind: crate::model::AssetKind::Hook,
            name: script_filename.into(),
            hint: format!(
                "平台 `{}` 上找不到脚本 `{script_filename}`",
                platform::platform_label(from_plat)
            ),
        });
    }
    let bindings = read_platform_bindings(deploy_base, from_plat, script_filename)?;
    if bindings.is_empty() {
        return Err(CoreError::InvalidPath(format!(
            "平台 `{}` 的 hooks 配置中找不到 `{script_filename}` 的绑定",
            platform::platform_label(from_plat)
        )));
    }
    let content = fs::read_to_string(from_script.as_std_path()).map_err(CoreError::Io)?;
    let spec = HookScriptSpec {
        filename: script_filename.to_string(),
        script_path: from_script,
        description: hook::parse_script_description(&content),
        bindings,
    };
    copy_script(&spec, deploy_base, to_plat)?;
    match to_plat {
        PlatformId::Cursor | PlatformId::Codex => {
            merge_cursor_codex_hooks(deploy_base, to_plat, &spec)?;
        }
        PlatformId::Claude => merge_claude_settings(deploy_base, &spec)?,
        PlatformId::Hermes => merge_hermes_config(&spec)?,
        PlatformId::AiConfig => {}
    }
    Ok(format!(
        "Hook `{script_filename}`: {} → {} OK",
        platform::platform_label(from_plat),
        platform::platform_label(to_plat)
    ))
}

/// 从平台导入到 ai-config 源：拷贝脚本并合并 `hooks.json`。
pub fn import_to_source(
    asset_root: &Utf8Path,
    deploy_base: &Utf8Path,
    script_filename: &str,
    from_plat: PlatformId,
) -> Result<Utf8PathBuf, CoreError> {
    let from_script = platform_script_path(deploy_base, from_plat, script_filename);
    if !from_script.is_file() {
        return Err(CoreError::AssetNotFound {
            kind: crate::model::AssetKind::Hook,
            name: script_filename.into(),
            hint: format!("平台 `{}` 无脚本", platform::platform_label(from_plat)),
        });
    }
    let bindings = read_platform_bindings(deploy_base, from_plat, script_filename)?;
    let dest = hook::script_path(asset_root, script_filename);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent.as_std_path()).map_err(CoreError::Io)?;
    }
    fs::copy(from_script.as_std_path(), dest.as_std_path()).map_err(CoreError::Io)?;
    set_executable(&dest)?;
    hook::merge_bindings_into_manifest(asset_root, script_filename, &bindings)?;
    Ok(dest)
}

/// 在 IDE 平台配置中为某脚本开关单个生命周期绑定。
pub fn toggle_platform_lifecycle(
    asset_root: &Utf8Path,
    deploy_base: &Utf8Path,
    plat: PlatformId,
    script_filename: &str,
    lifecycle: &str,
    enabled: bool,
) -> Result<(), CoreError> {
    if plat == PlatformId::AiConfig {
        return Err(CoreError::InvalidPath("源视图请切换 ai-config 平台".into()));
    }
    if !platform::supports_at_scope(plat, crate::model::AssetKind::Hook, deploy_base) {
        return Err(CoreError::UnsupportedAsset {
            platform: plat,
            asset: crate::model::AssetKind::Hook,
            hint: platform::capability_skip_reason(plat, crate::model::AssetKind::Hook),
        });
    }
    if enabled {
        let spec = hook::load_spec(asset_root, script_filename)?;
        copy_script(&spec, deploy_base, plat)?;
        let source_command =
            hook::infer_binding_command(script_filename, lifecycle, &spec.bindings);
        let platform_lifecycle = platform_lifecycle_key(plat, lifecycle);
        let binding = hook::HookBinding {
            lifecycle: platform_lifecycle,
            matcher: spec
                .bindings
                .iter()
                .find(|b| {
                    hook::canonical_lifecycle_id(&b.lifecycle, b.matcher.as_deref(), plat)
                        == lifecycle
                })
                .and_then(|b| b.matcher.clone())
                .or_else(|| default_matcher(plat, canonical_event_key(lifecycle))),
            command: platform_command(
                plat,
                deploy_base,
                script_filename,
                lifecycle,
                &source_command,
            ),
        };
        upsert_platform_binding(deploy_base, plat, script_filename, &binding)?;
    } else {
        remove_platform_binding(deploy_base, plat, script_filename, lifecycle)?;
    }
    Ok(())
}

fn platform_lifecycle_key(plat: PlatformId, lifecycle: &str) -> String {
    let normalized = hook::normalize_lifecycle(lifecycle);
    if plat == PlatformId::Cursor {
        // Cursor 平台的 key 就是 canonical lifecycle 本身（如 `beforeTabFileRead`），不应回落到 PostToolUse。
        return normalized;
    }
    platform_event(plat, canonical_event_key(&normalized)).to_string()
}

fn platform_command(
    plat: PlatformId,
    deploy_base: &Utf8Path,
    script_filename: &str,
    _lifecycle: &str,
    source_command: &str,
) -> String {
    let args: String = source_command
        .split_whitespace()
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ");
    match plat {
        PlatformId::Cursor | PlatformId::Codex => {
            if plat == PlatformId::Cursor {
                if args.is_empty() {
                    cursor_command_path(deploy_base, script_filename)
                } else {
                    format!(
                        "{} {args}",
                        cursor_command_path(deploy_base, script_filename)
                    )
                }
            } else if args.is_empty() {
                codex_command_path(deploy_base, script_filename)
            } else {
                format!(
                    "{} {args}",
                    codex_command_path(deploy_base, script_filename)
                )
            }
        }
        PlatformId::Claude | PlatformId::Hermes => {
            if args.is_empty() {
                if plat == PlatformId::Claude {
                    claude_command_path(deploy_base, script_filename)
                } else {
                    absolute_command_path(deploy_base, plat, script_filename)
                }
            } else {
                format!(
                    "{} {args}",
                    if plat == PlatformId::Claude {
                        claude_command_path(deploy_base, script_filename)
                    } else {
                        absolute_command_path(deploy_base, plat, script_filename)
                    }
                )
            }
        }
        PlatformId::AiConfig => source_command.to_string(),
    }
}

fn upsert_platform_binding(
    deploy_base: &Utf8Path,
    plat: PlatformId,
    script_filename: &str,
    binding: &hook::HookBinding,
) -> Result<(), CoreError> {
    match plat {
        PlatformId::Cursor | PlatformId::Codex => {
            let path = platform_config_path(deploy_base, plat);
            let mut doc = if path.is_file() {
                read_json(&path)?
            } else {
                json!({ "version": 1, "hooks": {} })
            };
            let hooks = doc
                .as_object_mut()
                .unwrap()
                .entry("hooks")
                .or_insert_with(|| json!({}));
            let arr = hooks
                .as_object_mut()
                .unwrap()
                .entry(binding.lifecycle.clone())
                .or_insert_with(|| json!([]));
            let items = arr.as_array_mut().unwrap();
            items.retain(|e| {
                e.get("command")
                    .and_then(|v| v.as_str())
                    .and_then(hook::filename_from_command)
                    .as_deref()
                    != Some(script_filename)
            });
            let mut item = json!({ "command": binding.command });
            if let Some(m) = &binding.matcher {
                item.as_object_mut()
                    .unwrap()
                    .insert("matcher".into(), json!(m));
            }
            items.push(item);
            write_json_atomic(&path, &doc)?;
        }
        PlatformId::Claude => merge_claude_single_binding(deploy_base, script_filename, binding)?,
        PlatformId::Hermes => merge_hermes_single_binding(script_filename, binding)?,
        PlatformId::AiConfig => {}
    }
    Ok(())
}

fn remove_platform_binding(
    deploy_base: &Utf8Path,
    plat: PlatformId,
    script_filename: &str,
    lifecycle: &str,
) -> Result<(), CoreError> {
    let platform_lifecycle = platform_lifecycle_key(plat, lifecycle);
    match plat {
        PlatformId::Cursor | PlatformId::Codex => {
            let path = platform_config_path(deploy_base, plat);
            if !path.is_file() {
                return Ok(());
            }
            let mut doc = read_json(&path)?;
            let Some(hooks) = doc.get_mut("hooks").and_then(|v| v.as_object_mut()) else {
                return Ok(());
            };
            let Some(items) = hooks
                .get_mut(&platform_lifecycle)
                .and_then(|v| v.as_array_mut())
            else {
                return Ok(());
            };
            items.retain(|e| {
                e.get("command")
                    .and_then(|v| v.as_str())
                    .and_then(hook::filename_from_command)
                    .as_deref()
                    != Some(script_filename)
            });
            if items.is_empty() {
                hooks.remove(&platform_lifecycle);
            }
            write_json_atomic(&path, &doc)?;
        }
        PlatformId::Claude => {
            remove_claude_binding(deploy_base, script_filename, &platform_lifecycle)?
        }
        PlatformId::Hermes => remove_hermes_binding(script_filename, &platform_lifecycle)?,
        PlatformId::AiConfig => {}
    }
    Ok(())
}

fn merge_claude_single_binding(
    deploy_base: &Utf8Path,
    script_filename: &str,
    binding: &hook::HookBinding,
) -> Result<(), CoreError> {
    let path = platform_config_path(deploy_base, PlatformId::Claude);
    let mut doc = if path.is_file() {
        read_json(&path)?
    } else {
        json!({})
    };
    let hooks = doc
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| json!({}));
    let arr = hooks
        .as_object_mut()
        .unwrap()
        .entry(binding.lifecycle.clone())
        .or_insert_with(|| json!([]));
    let items = arr.as_array_mut().unwrap();
    items.retain(|e| {
        e.get("hooks")
            .and_then(|v| v.as_array())
            .map(|arr| {
                !arr.iter().any(|h| {
                    h.get("command")
                        .and_then(|v| v.as_str())
                        .and_then(hook::filename_from_command)
                        .as_deref()
                        == Some(script_filename)
                })
            })
            .unwrap_or(true)
    });
    let inner = json!({
        "type": "command",
        "command": binding.command,
        "timeout": 10,
    });
    items.push(json!({ "hooks": [inner] }));
    write_json_atomic(&path, &doc)
}

fn remove_claude_binding(
    deploy_base: &Utf8Path,
    script_filename: &str,
    lifecycle: &str,
) -> Result<(), CoreError> {
    let path = platform_config_path(deploy_base, PlatformId::Claude);
    if !path.is_file() {
        return Ok(());
    }
    let mut doc = read_json(&path)?;
    let Some(hooks) = doc.get_mut("hooks").and_then(|v| v.as_object_mut()) else {
        return Ok(());
    };
    let Some(items) = hooks.get_mut(lifecycle).and_then(|v| v.as_array_mut()) else {
        return Ok(());
    };
    items.retain(|group| {
        group
            .get("hooks")
            .and_then(|v| v.as_array())
            .map(|arr| {
                !arr.iter().any(|h| {
                    h.get("command")
                        .and_then(|v| v.as_str())
                        .and_then(hook::filename_from_command)
                        .as_deref()
                        == Some(script_filename)
                })
            })
            .unwrap_or(true)
    });
    if items.is_empty() {
        hooks.remove(lifecycle);
    }
    write_json_atomic(&path, &doc)
}

fn merge_hermes_single_binding(
    script_filename: &str,
    binding: &hook::HookBinding,
) -> Result<(), CoreError> {
    let path = paths::home_dir().join(".hermes/config.yaml");
    let mut doc = read_yaml(&path)?.unwrap_or_else(|| YamlValue::Mapping(Default::default()));
    let YamlValue::Mapping(ref mut root) = doc else {
        return Ok(());
    };
    let hooks_entry = root
        .entry(YamlValue::String("hooks".into()))
        .or_insert_with(|| YamlValue::Mapping(Default::default()));
    let YamlValue::Mapping(ref mut hooks) = hooks_entry else {
        return Ok(());
    };
    let seq_entry = hooks
        .entry(YamlValue::String(binding.lifecycle.clone()))
        .or_insert_with(|| YamlValue::Sequence(vec![]));
    let YamlValue::Sequence(ref mut seq) = seq_entry else {
        return Ok(());
    };
    seq.retain(|entry| {
        let YamlValue::Mapping(m) = entry else {
            return true;
        };
        let cmd = m
            .get(YamlValue::String("command".into()))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        hook::filename_from_command(cmd).as_deref() != Some(script_filename)
    });
    let mut m = serde_yaml::Mapping::new();
    m.insert(
        YamlValue::String("command".into()),
        YamlValue::String(binding.command.clone()),
    );
    if let Some(matcher) = &binding.matcher {
        m.insert(
            YamlValue::String("matcher".into()),
            YamlValue::String(matcher.clone()),
        );
    }
    seq.push(YamlValue::Mapping(m));
    write_yaml_atomic(&path, &doc)
}

fn remove_hermes_binding(script_filename: &str, lifecycle: &str) -> Result<(), CoreError> {
    let path = paths::home_dir().join(".hermes/config.yaml");
    let Some(mut doc) = read_yaml(&path)? else {
        return Ok(());
    };
    let YamlValue::Mapping(ref mut root) = doc else {
        return Ok(());
    };
    let Some(YamlValue::Mapping(ref mut hooks)) = root.get_mut(YamlValue::String("hooks".into()))
    else {
        return Ok(());
    };
    let Some(YamlValue::Sequence(ref mut seq)) =
        hooks.get_mut(YamlValue::String(lifecycle.to_string()))
    else {
        return Ok(());
    };
    seq.retain(|entry| {
        let YamlValue::Mapping(m) = entry else {
            return true;
        };
        let cmd = m
            .get(YamlValue::String("command".into()))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        hook::filename_from_command(cmd).as_deref() != Some(script_filename)
    });
    if seq.is_empty() {
        hooks.remove(YamlValue::String(lifecycle.to_string()));
    }
    write_yaml_atomic(&path, &doc)
}

fn extract_json_bindings(
    doc: &Value,
    script_filename: &str,
    cursor_flat: bool,
) -> Vec<hook::HookBinding> {
    let mut out = Vec::new();
    let Some(hooks) = doc.get("hooks").and_then(|v| v.as_object()) else {
        return out;
    };
    for (lifecycle, entries) in hooks {
        let Some(arr) = entries.as_array() else {
            continue;
        };
        for entry in arr {
            if cursor_flat {
                push_binding_if_matches(&mut out, lifecycle, entry, script_filename, None);
            } else if let Some(inner) = entry.get("hooks").and_then(|v| v.as_array()) {
                let group_matcher = entry
                    .get("matcher")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                for h in inner {
                    push_binding_if_matches(
                        &mut out,
                        lifecycle,
                        h,
                        script_filename,
                        group_matcher.as_deref(),
                    );
                }
            }
        }
    }
    out
}

fn extract_claude_bindings(doc: &Value, script_filename: &str) -> Vec<hook::HookBinding> {
    extract_json_bindings(doc, script_filename, false)
}

fn push_binding_if_matches(
    out: &mut Vec<hook::HookBinding>,
    lifecycle: &str,
    entry: &Value,
    script_filename: &str,
    inherited_matcher: Option<&str>,
) {
    let Some(command) = entry.get("command").and_then(|v| v.as_str()) else {
        return;
    };
    if hook::filename_from_command(command).as_deref() != Some(script_filename) {
        return;
    }
    out.push(hook::HookBinding {
        lifecycle: lifecycle.to_string(),
        matcher: entry
            .get("matcher")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| inherited_matcher.map(str::to_string)),
        command: command.to_string(),
    });
}

fn read_hermes_bindings(script_filename: &str) -> Result<Vec<hook::HookBinding>, CoreError> {
    let path = paths::home_dir().join(".hermes/config.yaml");
    let Some(doc) = read_yaml(&path)? else {
        return Ok(Vec::new());
    };
    let YamlValue::Mapping(root) = doc else {
        return Ok(Vec::new());
    };
    let Some(YamlValue::Mapping(hooks)) = root.get(YamlValue::String("hooks".into())) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (lifecycle, entries) in hooks {
        let Some(lifecycle) = lifecycle.as_str() else {
            continue;
        };
        let YamlValue::Sequence(seq) = entries else {
            continue;
        };
        for entry in seq {
            let YamlValue::Mapping(m) = entry else {
                continue;
            };
            let cmd = m
                .get(YamlValue::String("command".into()))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if hook::filename_from_command(cmd).as_deref() != Some(script_filename) {
                continue;
            }
            let matcher = m
                .get(YamlValue::String("matcher".into()))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            out.push(hook::HookBinding {
                lifecycle: lifecycle.to_string(),
                matcher,
                command: cmd.to_string(),
            });
        }
    }
    Ok(out)
}

fn config_contains_token(path: &Utf8Path, token: &str) -> bool {
    let Ok(existing) = read_json(path) else {
        return false;
    };
    let script = token.strip_prefix("/hooks/").unwrap_or(token);
    let cursor_flat = path.as_str().contains(".cursor/hooks.json");
    if cursor_flat {
        let Some(hooks) = existing.get("hooks").and_then(|v| v.as_object()) else {
            return false;
        };
        return hooks.values().any(|entries| {
            entries
                .as_array()
                .is_some_and(|arr| arr.iter().any(|e| entry_matches_token(e, token, true)))
        });
    }
    if let Some(hooks) = existing.get("hooks").and_then(|v| v.as_object()) {
        return hooks.values().any(|entries| {
            entries
                .as_array()
                .is_some_and(|arr| arr.iter().any(|e| entry_matches_token(e, token, false)))
        });
    }
    let _ = script;
    false
}

fn hermes_contains_hook(hook_name: &str) -> bool {
    let path = paths::home_dir().join(".hermes/config.yaml");
    let Ok(Some(doc)) = read_yaml(&path) else {
        return false;
    };
    let YamlValue::Mapping(root) = &doc else {
        return false;
    };
    let Some(YamlValue::Mapping(hooks)) = root.get(YamlValue::String("hooks".into())) else {
        return false;
    };
    hooks.values().any(|v| {
        v.as_sequence()
            .is_some_and(|seq| seq.iter().any(|e| yaml_entry_is_managed(e, hook_name)))
    })
}

fn copy_script(
    spec: &HookScriptSpec,
    deploy_base: &Utf8Path,
    plat: PlatformId,
) -> Result<(), CoreError> {
    let dest = platform_script_path(deploy_base, plat, &spec.filename);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent.as_std_path()).map_err(CoreError::Io)?;
    }
    fs::copy(spec.script_path.as_std_path(), dest.as_std_path()).map_err(CoreError::Io)?;
    set_executable(&dest)?;
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Utf8Path) -> Result<(), CoreError> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path.as_std_path())
        .map_err(CoreError::Io)?
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path.as_std_path(), perms).map_err(CoreError::Io)
}

#[cfg(not(unix))]
fn set_executable(_path: &Utf8Path) -> Result<(), CoreError> {
    Ok(())
}

/// Cursor `command` 路径：项目 hook 从仓库根执行 → `.cursor/hooks/`；全局 `~/.cursor` hook → `./hooks/`。
fn cursor_command_path(deploy_base: &Utf8Path, script_filename: &str) -> String {
    if deploy_base == paths::home_dir() {
        format!("./hooks/{script_filename}")
    } else {
        format!(".cursor/hooks/{script_filename}")
    }
}

/// Codex `command` 路径：项目下发尽量使用相对路径，避免机器绝对路径绑定。
fn codex_command_path(deploy_base: &Utf8Path, script_filename: &str) -> String {
    if deploy_base == paths::home_dir() {
        absolute_command_path(deploy_base, PlatformId::Codex, script_filename)
    } else {
        format!(".codex/hooks/{script_filename}")
    }
}

/// Claude `command` 路径：项目下发使用 `${CLAUDE_PROJECT_DIR}` 保持 cwd 无关。
fn claude_command_path(deploy_base: &Utf8Path, script_filename: &str) -> String {
    if deploy_base == paths::home_dir() {
        absolute_command_path(deploy_base, PlatformId::Claude, script_filename)
    } else if project_uses_shared_cursor_hooks(deploy_base) {
        format!(".cursor/hooks/{script_filename}")
    } else {
        format!("${{CLAUDE_PROJECT_DIR}}/.claude/hooks/{script_filename}")
    }
}

fn absolute_command_path(
    deploy_base: &Utf8Path,
    plat: PlatformId,
    script_filename: &str,
) -> String {
    platform_script_path(deploy_base, plat, script_filename)
        .as_str()
        .to_string()
}

fn platform_event_from_binding(plat: PlatformId, lifecycle: &str) -> String {
    let normalized = hook::normalize_lifecycle(lifecycle);
    if plat == PlatformId::Cursor {
        return normalized;
    }
    platform_event(plat, canonical_event_key(&normalized)).to_string()
}

/// 把 Cursor 风格的 lifecycle 收敛到 canonical key，再由 `platform_event` 派发到平台事件名。
///
/// - `postToolUse`/`preToolUse`（非 Shell 的通用工具生命周期）必须与 `afterShellExecution`/`beforeShellExecution` 区分；
///   否则在 Codex/Claude 上会被错误地套上 Bash matcher。
/// - `PostToolUse`/`PreToolUse` 是平台原生 Pascal 命名（写入 .claude/.codex hooks.json 时），需要先回到 canonical。
fn canonical_event_key(lifecycle: &str) -> &str {
    match lifecycle {
        "afterShellExecution" => "afterShellExecution",
        "beforeShellExecution" => "beforeShellExecution",
        "postToolUse" => "postToolUse",
        "preToolUse" => "preToolUse",
        "postToolUseFailure" => "postToolUseFailure",
        "PostToolUse" => "postToolUse",
        "PreToolUse" => "preToolUse",
        "PostToolUseFailure" => "postToolUseFailure",
        "post_tool_call" => "afterShellExecution",
        "pre_tool_call" => "beforeShellExecution",
        other => other,
    }
}

fn platform_event(plat: PlatformId, canonical: &str) -> &'static str {
    match (plat, canonical) {
        (PlatformId::Cursor, "afterShellExecution") => "afterShellExecution",
        (PlatformId::Cursor, "beforeShellExecution") => "beforeShellExecution",
        (PlatformId::Cursor, "postToolUse") => "postToolUse",
        (PlatformId::Cursor, "preToolUse") => "preToolUse",
        (PlatformId::Cursor, "postToolUseFailure") => "postToolUseFailure",
        (PlatformId::Codex, "sessionStart") | (PlatformId::Claude, "sessionStart") => {
            "SessionStart"
        }
        (PlatformId::Codex, "sessionEnd") | (PlatformId::Claude, "sessionEnd") => "SessionEnd",
        (PlatformId::Codex, "beforeSubmitPrompt") | (PlatformId::Claude, "beforeSubmitPrompt") => {
            "UserPromptSubmit"
        }
        (PlatformId::Codex, "preToolUse") | (PlatformId::Claude, "preToolUse") => "PreToolUse",
        (PlatformId::Codex, "postToolUse")
        | (PlatformId::Claude, "postToolUse")
        | (PlatformId::Claude, "postToolUseFailure") => "PostToolUse",
        (PlatformId::Codex, "beforeShellExecution")
        | (PlatformId::Claude, "beforeShellExecution") => "PreToolUse",
        (PlatformId::Codex, "afterShellExecution")
        | (PlatformId::Claude, "afterShellExecution") => "PostToolUse",
        (PlatformId::Codex, "subagentStart") | (PlatformId::Claude, "subagentStart") => {
            "SubagentStart"
        }
        (PlatformId::Codex, "subagentStop") | (PlatformId::Claude, "subagentStop") => {
            "SubagentStop"
        }
        (PlatformId::Codex, "stop") | (PlatformId::Claude, "stop") => "Stop",
        (PlatformId::Codex, "preCompact") | (PlatformId::Claude, "preCompact") => "PreCompact",
        (PlatformId::Codex, "postCompact") | (PlatformId::Claude, "postCompact") => "PostCompact",
        (PlatformId::Hermes, "afterShellExecution") => "post_tool_call",
        (PlatformId::Hermes, "beforeShellExecution") => "pre_tool_call",
        (PlatformId::Hermes, _) => "PostToolUse",
        _ => "PostToolUse",
    }
}

fn default_matcher(plat: PlatformId, canonical: &str) -> Option<String> {
    match (plat, canonical) {
        (PlatformId::Cursor, "afterShellExecution" | "beforeShellExecution") => {
            Some(r"ai-config\b".into())
        }
        (PlatformId::Codex | PlatformId::Claude, "afterFileEdit" | "afterTabFileEdit") => {
            Some("Edit|Write".into())
        }
        (PlatformId::Codex | PlatformId::Claude, "beforeMCPExecution" | "afterMCPExecution") => {
            Some("mcp__.*".into())
        }
        (
            PlatformId::Codex | PlatformId::Claude,
            "afterShellExecution" | "beforeShellExecution",
        ) => Some("Bash".into()),
        (PlatformId::Hermes, _) => Some("terminal".into()),
        _ => None,
    }
}

fn build_cursor_entries(deploy_base: &Utf8Path, spec: &HookScriptSpec) -> Map<String, Value> {
    let mut out = Map::new();
    for binding in &spec.bindings {
        let event = platform_event_from_binding(PlatformId::Cursor, &binding.lifecycle);
        let cmd = platform_command(
            PlatformId::Cursor,
            deploy_base,
            &spec.filename,
            &binding.lifecycle,
            &binding.command,
        );
        let matcher = binding.matcher.clone().or_else(|| {
            default_matcher(PlatformId::Cursor, canonical_event_key(&binding.lifecycle))
        });
        let mut item = json!({
            "command": cmd,
            "managedBy": MANAGED_BY,
            "hook": spec.filename,
        });
        if let Some(m) = matcher {
            item.as_object_mut()
                .unwrap()
                .insert("matcher".into(), json!(m));
        }
        out.entry(event)
            .or_insert_with(|| Value::Array(vec![]))
            .as_array_mut()
            .unwrap()
            .push(item);
    }
    out
}

fn build_codex_claude_group(
    spec: &HookScriptSpec,
    deploy_base: &Utf8Path,
    plat: PlatformId,
) -> Map<String, Value> {
    let mut out = Map::new();
    for binding in &spec.bindings {
        let event = platform_event_from_binding(plat, &binding.lifecycle);
        let cmd = platform_command(
            plat,
            deploy_base,
            &spec.filename,
            &binding.lifecycle,
            &binding.command,
        );
        let matcher = binding
            .matcher
            .clone()
            .or_else(|| default_matcher(plat, canonical_event_key(&binding.lifecycle)));
        let mut inner = json!({
            "type": "command",
            "command": cmd,
        });
        if plat == PlatformId::Codex {
            inner
                .as_object_mut()
                .unwrap()
                .insert("managedBy".into(), json!(MANAGED_BY));
            inner
                .as_object_mut()
                .unwrap()
                .insert("hook".into(), json!(spec.filename));
        }
        let mut group = json!({ "hooks": [inner] });
        if let Some(m) = matcher {
            group
                .as_object_mut()
                .unwrap()
                .insert("matcher".into(), json!(m));
        }
        out.entry(event)
            .or_insert_with(|| Value::Array(vec![]))
            .as_array_mut()
            .unwrap()
            .push(group);
    }
    out
}

fn merge_cursor_codex_hooks(
    deploy_base: &Utf8Path,
    plat: PlatformId,
    spec: &HookScriptSpec,
) -> Result<(), CoreError> {
    let path = platform_config_path(deploy_base, plat);
    let existing = if path.is_file() {
        Some(read_json(&path)?)
    } else {
        None
    };
    let token = hook::managed_command_token(&spec.filename);
    let src_entries = if plat == PlatformId::Cursor {
        build_cursor_entries(deploy_base, spec)
    } else {
        build_codex_claude_group(spec, deploy_base, plat)
    };
    let merged = merge_json_hook_events(existing, &token, src_entries, plat == PlatformId::Cursor)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent.as_std_path()).map_err(CoreError::Io)?;
    }
    atomic_write_json(&path, &merged)
}

fn merge_json_hook_events(
    existing: Option<Value>,
    token: &str,
    src_entries: Map<String, Value>,
    cursor_flat: bool,
) -> Result<Value, CoreError> {
    let mut base = existing.unwrap_or_else(|| json!({ "version": 1, "hooks": {} }));
    if base.get("hooks").is_none() {
        base.as_object_mut()
            .unwrap()
            .insert("hooks".into(), json!({}));
    }
    let hooks = base
        .as_object_mut()
        .unwrap()
        .get_mut("hooks")
        .unwrap()
        .as_object_mut()
        .unwrap();

    // 先全量清理旧的托管条目（覆盖事件迁移与纯 retract 场景）。
    let mut empty_events = Vec::new();
    for (event, entries) in hooks.iter_mut() {
        if let Some(arr) = entries.as_array_mut() {
            arr.retain(|e| !entry_matches_token(e, token, cursor_flat));
            if arr.is_empty() {
                empty_events.push(event.clone());
            }
        }
    }
    for event in empty_events {
        hooks.remove(&event);
    }

    for (event, new_items) in src_entries {
        let arr = hooks.entry(event).or_insert_with(|| Value::Array(vec![]));
        let dest_arr = arr.as_array_mut().unwrap();
        dest_arr.extend(new_items.as_array().cloned().unwrap_or_default());
    }
    if base.get("version").is_none() {
        base.as_object_mut()
            .unwrap()
            .insert("version".into(), json!(1));
    }
    Ok(base)
}

fn entry_matches_token(entry: &Value, token: &str, cursor_flat: bool) -> bool {
    let script = token.strip_prefix("/hooks/").unwrap_or(token);
    if cursor_flat {
        if entry.get("managedBy").and_then(|v| v.as_str()) != Some(MANAGED_BY) {
            return false;
        }
        return entry.get("hook").and_then(|v| v.as_str()) == Some(script)
            || entry
                .get("command")
                .and_then(|v| v.as_str())
                .is_some_and(|c| hook::filename_from_command(c).as_deref() == Some(script));
    }
    if let Some(arr) = entry.get("hooks").and_then(|v| v.as_array()) {
        return arr.iter().any(|h| hook_entry_matches_script(h, script));
    }
    hook_entry_matches_script(entry, script)
}

fn hook_entry_matches_script(entry: &Value, script: &str) -> bool {
    if entry.get("managedBy").and_then(|v| v.as_str()) != Some(MANAGED_BY) {
        return false;
    }
    entry.get("hook").and_then(|v| v.as_str()) == Some(script)
        || entry
            .get("command")
            .and_then(|v| v.as_str())
            .is_some_and(|c| hook::filename_from_command(c).as_deref() == Some(script))
}

fn remove_cursor_codex_managed(existing: Value, token: &str) -> Result<Value, CoreError> {
    merge_json_hook_events(Some(existing), token, Map::new(), true)
}

fn merge_claude_settings(deploy_base: &Utf8Path, spec: &HookScriptSpec) -> Result<(), CoreError> {
    let path = platform_config_path(deploy_base, PlatformId::Claude);
    let existing = if path.is_file() {
        Some(read_json(&path)?)
    } else {
        None
    };
    let mcp_servers = existing.as_ref().and_then(|e| e.get("mcpServers")).cloned();
    let token = hook::managed_command_token(&spec.filename);
    let src = build_codex_claude_group(spec, deploy_base, PlatformId::Claude);
    let mut merged = merge_json_hook_events(existing, &token, src, false)?;
    if let Some(mcp) = mcp_servers {
        merged
            .as_object_mut()
            .ok_or_else(|| CoreError::InvalidPath("settings.json 顶层须为 object".into()))?
            .insert("mcpServers".into(), mcp);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent.as_std_path()).map_err(CoreError::Io)?;
    }
    atomic_write_json(&path, &merged)
}

fn remove_claude_managed(existing: Value, token: &str) -> Result<Value, CoreError> {
    merge_json_hook_events(Some(existing), token, Map::new(), false)
}

fn merge_hermes_config(spec: &HookScriptSpec) -> Result<(), CoreError> {
    let path = paths::home_dir().join(".hermes/config.yaml");
    let mut root = read_yaml(&path)?.unwrap_or(YamlValue::Mapping(Mapping::new()));
    let mapping = match &mut root {
        YamlValue::Mapping(m) => m,
        _ => {
            root = YamlValue::Mapping(Mapping::new());
            root.as_mapping_mut().unwrap()
        }
    };
    let hooks_key = YamlValue::String("hooks".into());
    let hooks_map = match mapping
        .entry(hooks_key)
        .or_insert_with(|| YamlValue::Mapping(Mapping::new()))
    {
        YamlValue::Mapping(m) => m,
        _ => {
            let m = Mapping::new();
            mapping.insert(
                YamlValue::String("hooks".into()),
                YamlValue::Mapping(m.clone()),
            );
            mapping
                .get_mut(YamlValue::String("hooks".into()))
                .unwrap()
                .as_mapping_mut()
                .unwrap()
        }
    };

    let filename = &spec.filename;
    let cmd = absolute_command_path(&paths::home_dir(), PlatformId::Hermes, filename);
    let managed_key = YamlValue::String(format!("{MANAGED_BY}:{filename}"));

    for binding in &spec.bindings {
        let event_key = YamlValue::String(platform_event_from_binding(
            PlatformId::Hermes,
            &binding.lifecycle,
        ));
        let matcher = binding.matcher.clone().or_else(|| {
            default_matcher(PlatformId::Hermes, canonical_event_key(&binding.lifecycle))
        });
        let mut item = Mapping::new();
        item.insert(
            YamlValue::String("command".into()),
            YamlValue::String(cmd.clone()),
        );
        item.insert(
            YamlValue::String("managedBy".into()),
            YamlValue::String(MANAGED_BY.into()),
        );
        item.insert(
            YamlValue::String("hook".into()),
            YamlValue::String(filename.clone()),
        );
        if let Some(m) = matcher {
            item.insert(YamlValue::String("matcher".into()), YamlValue::String(m));
        }
        let list = hooks_map
            .entry(event_key)
            .or_insert_with(|| YamlValue::Sequence(vec![]));
        if let YamlValue::Sequence(seq) = list {
            seq.retain(|entry| !yaml_entry_is_managed(entry, filename));
            seq.push(YamlValue::Mapping(item));
        }
    }
    hooks_map.remove(&managed_key);
    write_yaml(&path, &root)
}

fn yaml_entry_is_managed(entry: &YamlValue, script_filename: &str) -> bool {
    let Some(m) = entry.as_mapping() else {
        return false;
    };
    m.get(YamlValue::String("hook".into()))
        .and_then(|v| v.as_str())
        == Some(script_filename)
}

fn remove_hermes_managed(script_filename: &str) -> Result<(), CoreError> {
    let path = paths::home_dir().join(".hermes/config.yaml");
    let Some(mut root) = read_yaml(&path)? else {
        return Ok(());
    };
    let YamlValue::Mapping(ref mut mapping) = root else {
        return Ok(());
    };
    let Some(YamlValue::Mapping(hooks)) = mapping.get_mut(YamlValue::String("hooks".into())) else {
        return Ok(());
    };
    for (_k, v) in hooks.iter_mut() {
        if let YamlValue::Sequence(seq) = v {
            seq.retain(|entry| !yaml_entry_is_managed(entry, script_filename));
        }
    }
    write_yaml(&path, &root)
}

fn read_json(path: &Utf8Path) -> Result<Value, CoreError> {
    let raw = fs::read_to_string(path.as_std_path()).map_err(CoreError::Io)?;
    serde_json::from_str(&raw).map_err(|e| CoreError::TemplateRender {
        template: path.as_str().to_owned(),
        reason: e.to_string(),
        hint: "JSON 解析失败".into(),
    })
}

fn read_yaml(path: &Utf8Path) -> Result<Option<YamlValue>, CoreError> {
    match fs::read_to_string(path.as_std_path()) {
        Ok(raw) => {
            let v = serde_yaml::from_str(&raw).map_err(|e| CoreError::TemplateRender {
                template: path.as_str().to_owned(),
                reason: e.to_string(),
                hint: "YAML 解析失败".into(),
            })?;
            Ok(Some(v))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CoreError::Io(e)),
    }
}

fn write_yaml(path: &Utf8Path, doc: &YamlValue) -> Result<(), CoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent.as_std_path()).map_err(CoreError::Io)?;
    }
    let raw = serde_yaml::to_string(doc).map_err(|e| CoreError::TemplateRender {
        template: path.as_str().to_owned(),
        reason: e.to_string(),
        hint: "YAML 序列化失败".into(),
    })?;
    fs::write(path.as_std_path(), raw).map_err(CoreError::Io)
}

fn write_json_atomic(path: &Utf8Path, doc: &Value) -> Result<(), CoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent.as_std_path()).map_err(CoreError::Io)?;
    }
    let out = serde_json::to_string_pretty(doc).map_err(|e| CoreError::Io(e.into()))?;
    fs::write(path.as_std_path(), out).map_err(CoreError::Io)
}

fn write_yaml_atomic(path: &Utf8Path, doc: &YamlValue) -> Result<(), CoreError> {
    write_yaml(path, doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::EnvGuard;
    use std::fs;
    use tempfile::TempDir;

    fn sample_spec(asset: &Utf8Path) -> HookScriptSpec {
        HookScriptSpec {
            filename: "run.sh".into(),
            script_path: asset.join("hooks/run.sh"),
            description: "d".into(),
            bindings: vec![hook::HookBinding {
                lifecycle: "afterShellExecution".into(),
                matcher: Some(r"ai-config\b".into()),
                command: "./hooks/run.sh".into(),
            }],
        }
    }

    #[test]
    fn cursor_merge_keeps_third_party() {
        let existing = json!({
            "version": 1,
            "hooks": {
                "afterShellExecution": [
                    { "command": "./hooks/other.sh" },
                    { "command": "./hooks/run.sh", "managedBy": "ai-config", "hook": "run.sh" }
                ]
            }
        });
        let tmp = TempDir::new().unwrap();
        let asset = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let spec = sample_spec(&asset);
        let src = build_cursor_entries(&asset, &spec);
        let merged = merge_json_hook_events(Some(existing), "/hooks/run.sh", src, true).unwrap();
        let arr = merged["hooks"]["afterShellExecution"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["command"], "./hooks/other.sh");
    }

    #[test]
    fn cursor_retract_keeps_other_managed_script() {
        let existing = json!({
            "version": 1,
            "hooks": {
                "beforeSubmitPrompt": [
                    { "command": ".cursor/hooks/speak-lifecycle.py beforeSubmitPrompt", "managedBy": "ai-config", "hook": "speak-lifecycle.py" },
                    { "command": ".cursor/hooks/test-prompt-hook.sh", "managedBy": "ai-config", "hook": "test-prompt-hook.sh" }
                ]
            }
        });
        let merged = merge_json_hook_events(
            Some(existing),
            "/hooks/test-prompt-hook.sh",
            Map::new(),
            true,
        )
        .unwrap();
        let arr = merged["hooks"]["beforeSubmitPrompt"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["hook"], "speak-lifecycle.py");
    }

    #[test]
    fn deploy_cursor_writes_hooks_json() {
        let tmp = TempDir::new().unwrap();
        let asset = tmp.path().join("asset");
        let hooks = asset.join("hooks");
        fs::create_dir_all(&hooks).unwrap();
        fs::write(
            asset.join("hooks.json"),
            r#"{"version":1,"hooks":{"afterShellExecution":[{"command":"./hooks/run.sh","matcher":"ai-config"}]}}"#,
        )
        .unwrap();
        fs::write(hooks.join("run.sh"), "#!/bin/sh\n").unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let asset_u = Utf8PathBuf::from_path_buf(asset).unwrap();
        let home_u = Utf8PathBuf::from_path_buf(home.clone()).unwrap();
        deploy(&asset_u, &home_u, "run.sh", PlatformId::Cursor).unwrap();
        assert!(home.join(".cursor/hooks.json").is_file());
        assert!(home.join(".cursor/hooks/run.sh").is_file());
        let doc: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(home.join(".cursor/hooks.json")).unwrap())
                .unwrap();
        assert_eq!(
            doc["hooks"]["afterShellExecution"][0]["command"],
            cursor_command_path(&home_u, "run.sh")
        );
    }

    #[test]
    fn deploy_cursor_project_uses_dot_cursor_hooks_path() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        let asset = repo.join(".ai-config");
        let hooks = asset.join("hooks");
        fs::create_dir_all(&hooks).unwrap();
        fs::write(
            asset.join("hooks.json"),
            r#"{"version":1,"hooks":{"sessionStart":[{"command":"./hooks/run.py sessionStart"}]}}"#,
        )
        .unwrap();
        fs::write(hooks.join("run.py"), "#!/usr/bin/env python3\n").unwrap();
        let repo_u = Utf8PathBuf::from_path_buf(repo).unwrap();
        let asset_u = Utf8PathBuf::from_path_buf(asset).unwrap();
        deploy(&asset_u, &repo_u, "run.py", PlatformId::Cursor).unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(repo_u.join(".cursor/hooks.json")).unwrap())
                .unwrap();
        assert_eq!(
            doc["hooks"]["sessionStart"][0]["command"],
            ".cursor/hooks/run.py sessionStart"
        );
    }

    #[test]
    fn deploy_codex_writes_hooks_json_and_script() {
        let tmp = TempDir::new().unwrap();
        let asset = tmp.path().join("asset");
        let hooks = asset.join("hooks");
        fs::create_dir_all(&hooks).unwrap();
        fs::write(
            asset.join("hooks.json"),
            r#"{"version":1,"hooks":{"afterShellExecution":[{"command":"./hooks/run.sh","matcher":"ai-config"}]}}"#,
        )
        .unwrap();
        fs::write(hooks.join("run.sh"), "#!/bin/sh\necho codex\n").unwrap();

        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let asset_u = Utf8PathBuf::from_path_buf(asset).unwrap();
        let home_u = Utf8PathBuf::from_path_buf(home.clone()).unwrap();

        deploy(&asset_u, &home_u, "run.sh", PlatformId::Codex).unwrap();

        let hooks_json = home.join(".codex/hooks.json");
        assert!(hooks_json.is_file());
        assert!(home.join(".codex/hooks/run.sh").is_file());

        let doc: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(hooks_json).unwrap()).unwrap();
        let arr = doc["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        let group = &arr[0];
        let inner = &group["hooks"][0];
        assert_eq!(inner["command"].as_str().unwrap(), ".codex/hooks/run.sh");
        assert_eq!(inner["managedBy"], MANAGED_BY);
        assert_eq!(inner["hook"], "run.sh");
    }

    #[test]
    fn deploy_claude_writes_settings_json_and_script() {
        let tmp = TempDir::new().unwrap();
        let asset = tmp.path().join("asset");
        let hooks = asset.join("hooks");
        fs::create_dir_all(&hooks).unwrap();
        fs::write(
            asset.join("hooks.json"),
            r#"{"version":1,"hooks":{"afterShellExecution":[{"command":"./hooks/run.sh","matcher":"ai-config"}]}}"#,
        )
        .unwrap();
        fs::write(hooks.join("run.sh"), "#!/bin/sh\necho claude\n").unwrap();

        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        let asset_u = Utf8PathBuf::from_path_buf(asset).unwrap();
        let home_u = Utf8PathBuf::from_path_buf(home.clone()).unwrap();

        deploy(&asset_u, &home_u, "run.sh", PlatformId::Claude).unwrap();

        let settings_json = home.join(".claude/settings.json");
        assert!(settings_json.is_file());
        assert!(home.join(".claude/hooks/run.sh").is_file());

        let doc: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(settings_json).unwrap()).unwrap();
        let arr = doc["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        let group = &arr[0];
        let inner = &group["hooks"][0];
        assert_eq!(
            inner["command"].as_str().unwrap(),
            "${CLAUDE_PROJECT_DIR}/.claude/hooks/run.sh"
        );
        assert_eq!(inner["type"], "command");
        assert!(inner.get("managedBy").is_none());
        assert!(inner.get("hook").is_none());
    }

    #[test]
    fn deploy_claude_preserves_command_args() {
        let tmp = TempDir::new().unwrap();
        let asset = tmp.path().join("asset");
        let hooks = asset.join("hooks");
        fs::create_dir_all(&hooks).unwrap();
        fs::write(
            asset.join("hooks.json"),
            r#"{"version":1,"hooks":{"afterShellExecution":[{"command":"./hooks/speak-lifecycle.py afterShellExecution","matcher":"ai-config"}]}}"#,
        )
        .unwrap();
        fs::write(
            hooks.join("speak-lifecycle.py"),
            "#!/usr/bin/env python3\nprint('{}')\n",
        )
        .unwrap();

        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let asset_u = Utf8PathBuf::from_path_buf(asset).unwrap();
        let home_u = Utf8PathBuf::from_path_buf(home).unwrap();

        deploy(&asset_u, &home_u, "speak-lifecycle.py", PlatformId::Claude).unwrap();

        let settings = home_u.join(".claude/settings.json");
        let doc: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(settings.as_std_path()).unwrap()).unwrap();
        let cmd = doc["hooks"]["PostToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(
            cmd == "${CLAUDE_PROJECT_DIR}/.claude/hooks/speak-lifecycle.py afterShellExecution",
            "unexpected command: {cmd}"
        );
    }

    #[test]
    fn deploy_hermes_writes_config_and_agent_hook() {
        let tmp = TempDir::new().unwrap();
        // 让 Hermes 使用临时 HOME，避免污染真实配置与并发测试互相影响。
        let _home_guard = EnvGuard::set("HOME", tmp.path().to_string_lossy().as_ref());

        let asset = tmp.path().join("asset");
        let hooks = asset.join("hooks");
        fs::create_dir_all(&hooks).unwrap();
        fs::write(
            asset.join("hooks.json"),
            r#"{"version":1,"hooks":{"afterShellExecution":[{"command":"./hooks/run.sh","matcher":"ai-config"}]}}"#,
        )
        .unwrap();
        fs::write(hooks.join("run.sh"), "#!/bin/sh\necho hermes\n").unwrap();

        let deploy_base = tmp.path().join("project");
        fs::create_dir_all(&deploy_base).unwrap();

        let asset_u = Utf8PathBuf::from_path_buf(asset).unwrap();
        let deploy_u = Utf8PathBuf::from_path_buf(deploy_base.clone()).unwrap();

        deploy(&asset_u, &deploy_u, "run.sh", PlatformId::Hermes).unwrap();

        let home_u = paths::home_dir();
        let config_yaml = home_u.join(".hermes/config.yaml");
        let script_path = home_u.join(".hermes/agent-hooks/run.sh");
        assert!(config_yaml.is_file());
        assert!(script_path.is_file());

        let raw = std::fs::read_to_string(config_yaml.as_std_path()).unwrap();
        let doc: YamlValue = serde_yaml::from_str(&raw).unwrap();
        let YamlValue::Mapping(root) = doc else {
            panic!("root not mapping");
        };
        let hooks = root
            .get(YamlValue::String("hooks".into()))
            .and_then(|v| v.as_mapping())
            .expect("hooks entry");
        let list = hooks
            .get(YamlValue::String("post_tool_call".into()))
            .and_then(|v| v.as_sequence())
            .expect("post_tool_call list");
        assert_eq!(list.len(), 1);
        let YamlValue::Mapping(item) = &list[0] else {
            panic!("item not mapping");
        };
        assert_eq!(
            item.get(YamlValue::String("command".into()))
                .and_then(|v| v.as_str())
                .unwrap(),
            absolute_command_path(&home_u, PlatformId::Hermes, "run.sh")
        );
        assert_eq!(
            item.get(YamlValue::String("hook".into()))
                .and_then(|v| v.as_str())
                .unwrap(),
            "run.sh"
        );
    }

    #[test]
    fn codex_merge_keeps_third_party_groups() {
        let existing = json!({
            "version": 1,
            "hooks": {
                "PostToolUse": [
                    { "matcher": "Bash", "hooks": [{ "type": "command", "command": "/tmp/other.sh" }] },
                    { "matcher": "Bash", "hooks": [{ "type": "command", "command": "/tmp/.codex/hooks/run.sh", "managedBy": "ai-config", "hook": "run.sh" }] }
                ]
            }
        });
        let tmp = TempDir::new().unwrap();
        let asset = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let spec = sample_spec(&asset);
        let src = build_codex_claude_group(&spec, &asset, PlatformId::Codex);
        let merged = merge_json_hook_events(Some(existing), "/hooks/run.sh", src, false).unwrap();
        let arr = merged["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["hooks"][0]["command"], "/tmp/other.sh");
        assert_eq!(arr[1]["hooks"][0]["command"], ".codex/hooks/run.sh");
    }

    #[test]
    fn is_deployed_reports_across_platforms() {
        let tmp = TempDir::new().unwrap();
        let repo = Utf8PathBuf::from_path_buf(tmp.path().join("repo")).unwrap();
        let cursor_cfg = repo.join(".cursor/hooks.json");
        let codex_cfg = repo.join(".codex/hooks.json");
        let claude_cfg = repo.join(".claude/settings.json");

        std::fs::create_dir_all(cursor_cfg.parent().unwrap().as_std_path()).unwrap();
        std::fs::create_dir_all(codex_cfg.parent().unwrap().as_std_path()).unwrap();
        std::fs::create_dir_all(claude_cfg.parent().unwrap().as_std_path()).unwrap();

        std::fs::write(
            cursor_cfg.as_std_path(),
            r#"{"version":1,"hooks":{"afterShellExecution":[{"command":".cursor/hooks/run.sh","managedBy":"ai-config","hook":"run.sh"}]}}"#,
        )
        .unwrap();
        std::fs::write(
            codex_cfg.as_std_path(),
            r#"{"version":1,"hooks":{"PostToolUse":[{"hooks":[{"type":"command","command":"/tmp/.codex/hooks/run.sh","managedBy":"ai-config","hook":"run.sh"}]}]}}"#,
        )
        .unwrap();
        std::fs::write(
            claude_cfg.as_std_path(),
            r#"{"hooks":{"PostToolUse":[{"hooks":[{"type":"command","command":"/tmp/.claude/hooks/run.sh","managedBy":"ai-config","hook":"run.sh"}]}]}}"#,
        )
        .unwrap();

        assert!(is_deployed(&repo, PlatformId::Cursor, "run.sh"));
        assert!(is_deployed(&repo, PlatformId::Codex, "run.sh"));
        assert!(is_deployed(&repo, PlatformId::Claude, "run.sh"));
        assert!(!is_deployed(&repo, PlatformId::Cursor, "other.sh"));
    }

    #[test]
    fn claude_retract_keeps_third_party_groups() {
        let existing = json!({
            "hooks": {
                "PostToolUse": [
                    { "matcher": "Write", "hooks": [{ "type": "command", "command": "/tmp/third-party.sh" }] },
                    { "matcher": "Bash", "hooks": [{ "type": "command", "command": "/tmp/.claude/hooks/run.sh", "managedBy": "ai-config", "hook": "run.sh" }] }
                ]
            }
        });
        let merged = remove_claude_managed(existing, "/hooks/run.sh").unwrap();
        let arr = merged["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["hooks"][0]["command"], "/tmp/third-party.sh");
    }

    #[test]
    fn claude_retract_works_without_managed_metadata() {
        let existing = json!({
            "hooks": {
                "PostToolUse": [
                    { "hooks": [{ "type": "command", "command": "/tmp/third-party.sh" }] },
                    { "hooks": [{ "type": "command", "command": "/tmp/.claude/hooks/run.sh" }] }
                ]
            }
        });
        let merged = remove_claude_managed(existing, "/hooks/run.sh").unwrap();
        let arr = merged["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 2, "无 managedBy 的第三方条目不得被误删");
    }

    #[test]
    fn hermes_event_matcher_and_paths_are_stable() {
        assert_eq!(
            platform_event(PlatformId::Hermes, "afterShellExecution"),
            "post_tool_call"
        );
        assert_eq!(
            platform_event(PlatformId::Hermes, "beforeShellExecution"),
            "pre_tool_call"
        );
        assert_eq!(
            default_matcher(PlatformId::Hermes, "afterShellExecution"),
            Some("terminal".into())
        );
        let home = paths::home_dir();
        assert_eq!(
            platform_hooks_dir(Utf8Path::new("/tmp/repo"), PlatformId::Hermes),
            home.join(".hermes/agent-hooks")
        );
    }

    #[test]
    fn codex_and_claude_event_mapping_prefers_native_lifecycle_events() {
        assert_eq!(
            platform_event(PlatformId::Codex, canonical_event_key("sessionStart")),
            "SessionStart"
        );
        assert_eq!(
            platform_event(PlatformId::Codex, canonical_event_key("beforeSubmitPrompt")),
            "UserPromptSubmit"
        );
        assert_eq!(
            platform_event(PlatformId::Claude, canonical_event_key("subagentStop")),
            "SubagentStop"
        );
        assert_eq!(
            platform_event(PlatformId::Claude, canonical_event_key("preCompact")),
            "PreCompact"
        );
    }

    #[test]
    fn codex_and_claude_default_matchers_cover_edit_and_mcp() {
        assert_eq!(
            default_matcher(PlatformId::Codex, canonical_event_key("afterFileEdit")),
            Some("Edit|Write".into())
        );
        assert_eq!(
            default_matcher(
                PlatformId::Claude,
                canonical_event_key("beforeMCPExecution")
            ),
            Some("mcp__.*".into())
        );
        assert_eq!(
            default_matcher(
                PlatformId::Codex,
                canonical_event_key("afterShellExecution")
            ),
            Some("Bash".into())
        );
    }

    #[test]
    fn codex_and_claude_command_paths_are_portable() {
        let repo = Utf8Path::new("/tmp/repo");
        let home = paths::home_dir();
        assert_eq!(codex_command_path(repo, "run.sh"), ".codex/hooks/run.sh");
        assert_eq!(
            claude_command_path(repo, "run.sh"),
            "${CLAUDE_PROJECT_DIR}/.claude/hooks/run.sh"
        );
        assert!(codex_command_path(&home, "run.sh").contains(".codex/hooks/run.sh"));
        assert!(claude_command_path(&home, "run.sh").contains(".claude/hooks/run.sh"));
    }

    // ── 三端统一映射回归（plan: Hooks 三端统一完善） ─────────────────

    /// `postToolUse`/`preToolUse` 与 `*ShellExecution` 必须落到不同 canonical key，
    /// 避免在 Codex/Claude 上错误匹配 Bash。
    #[test]
    fn canonical_event_key_keeps_posttooluse_separate_from_shell() {
        assert_eq!(canonical_event_key("postToolUse"), "postToolUse");
        assert_eq!(canonical_event_key("preToolUse"), "preToolUse");
        assert_eq!(
            canonical_event_key("postToolUseFailure"),
            "postToolUseFailure"
        );
        assert_eq!(
            canonical_event_key("PostToolUse"),
            "postToolUse",
            "Pascal 写法应回到 canonical 通用工具事件"
        );
        assert_eq!(
            canonical_event_key("afterShellExecution"),
            "afterShellExecution",
            "Shell 事件不再被折回 after_shell"
        );
    }

    /// Codex/Claude 上通用工具事件应各自独立到 `PreToolUse`/`PostToolUse` 而非
    /// 把所有事件塞进 `PostToolUse`。
    #[test]
    fn platform_event_distinguishes_posttooluse_from_shell() {
        assert_eq!(
            platform_event(PlatformId::Codex, "postToolUse"),
            "PostToolUse"
        );
        assert_eq!(
            platform_event(PlatformId::Claude, "postToolUse"),
            "PostToolUse"
        );
        assert_eq!(
            platform_event(PlatformId::Codex, "preToolUse"),
            "PreToolUse"
        );
        assert_eq!(
            platform_event(PlatformId::Claude, "afterShellExecution"),
            "PostToolUse"
        );
        assert_eq!(
            platform_event(PlatformId::Claude, "postToolUseFailure"),
            "PostToolUse",
            "Claude 文档：postToolUseFailure 落在 PostToolUse 顶层组"
        );
    }

    /// Codex/Claude 上通用工具事件默认 matcher 应为 None（不强行套 Bash），
    /// 仅 Shell 事件才默认 `Bash`。
    #[test]
    fn default_matcher_only_for_shell_or_special() {
        assert_eq!(
            default_matcher(PlatformId::Codex, "postToolUse"),
            None,
            "Codex PostToolUse 不应默认 Bash"
        );
        assert_eq!(default_matcher(PlatformId::Claude, "preToolUse"), None);
        assert_eq!(
            default_matcher(PlatformId::Codex, "afterShellExecution"),
            Some("Bash".into())
        );
        assert_eq!(
            default_matcher(PlatformId::Claude, "beforeShellExecution"),
            Some("Bash".into())
        );
    }

    /// 顶层 lifecycle → Claude 平台事件映射需覆盖 6 个独立键。
    #[test]
    fn claude_top_level_event_keys_cover_lifecycle_variants() {
        for (canonical, expected) in [
            ("sessionStart", "SessionStart"),
            ("sessionEnd", "SessionEnd"),
            ("beforeSubmitPrompt", "UserPromptSubmit"),
            ("preToolUse", "PreToolUse"),
            ("postToolUse", "PostToolUse"),
            ("subagentStart", "SubagentStart"),
            ("subagentStop", "SubagentStop"),
            ("stop", "Stop"),
            ("preCompact", "PreCompact"),
            ("postCompact", "PostCompact"),
        ] {
            assert_eq!(
                platform_event(PlatformId::Claude, canonical),
                expected,
                "canonical={canonical}"
            );
        }
    }

    /// 端到端：源 `hooks.json` 含 sessionStart/sessionEnd/stop/preCompact 等多个 lifecycle
    /// 部署到 Claude 后，`.claude/settings.json` 顶层键必须分键。
    #[test]
    fn deploy_claude_splits_top_level_event_keys() {
        let tmp = TempDir::new().unwrap();
        let asset = tmp.path().join("asset");
        let hooks = asset.join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        std::fs::write(
            asset.join("hooks.json"),
            r#"{
  "version": 1,
  "hooks": {
    "sessionStart": [
      { "command": "./hooks/speak-lifecycle.py sessionStart" }
    ],
    "sessionEnd": [
      { "command": "./hooks/speak-lifecycle.py sessionEnd" }
    ],
    "stop": [
      { "command": "./hooks/speak-lifecycle.py stop" }
    ],
    "preCompact": [
      { "command": "./hooks/speak-lifecycle.py preCompact" }
    ],
    "subagentStart": [
      { "command": "./hooks/speak-lifecycle.py subagentStart" }
    ],
    "subagentStop": [
      { "command": "./hooks/speak-lifecycle.py subagentStop" }
    ],
    "beforeSubmitPrompt": [
      { "command": "./hooks/test-prompt-hook.sh" }
    ],
    "beforeShellExecution": [
      { "command": "./hooks/speak-lifecycle.py beforeShellExecution", "matcher": "ai-config" }
    ]
  }
}"#,
        )
        .unwrap();
        std::fs::write(
            hooks.join("speak-lifecycle.py"),
            "#!/usr/bin/env python3\nprint('ok')\n",
        )
        .unwrap();
        std::fs::write(
            hooks.join("test-prompt-hook.sh"),
            "#!/usr/bin/env bash\necho ok\n",
        )
        .unwrap();

        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let asset_u = Utf8PathBuf::from_path_buf(asset).unwrap();
        let home_u = Utf8PathBuf::from_path_buf(home.clone()).unwrap();

        // 部署 speak-lifecycle 与 test-prompt-hook 两个脚本
        deploy(&asset_u, &home_u, "speak-lifecycle.py", PlatformId::Claude).unwrap();
        deploy(&asset_u, &home_u, "test-prompt-hook.sh", PlatformId::Claude).unwrap();

        let settings = std::fs::read_to_string(home.join(".claude/settings.json")).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&settings).unwrap();
        let hooks_obj = doc.get("hooks").and_then(|v| v.as_object()).unwrap();

        // 这些 lifecycle 必须分键（不能全部塞进 PostToolUse）
        for key in [
            "SessionStart",
            "SessionEnd",
            "Stop",
            "PreCompact",
            "SubagentStart",
            "SubagentStop",
            "UserPromptSubmit",
        ] {
            assert!(
                hooks_obj.contains_key(key),
                "settings.json 顶层应包含 {key}，实际键: {:?}",
                hooks_obj.keys().collect::<Vec<_>>()
            );
        }
        // Shell 事件应当出现在 PreToolUse 下而非 SessionStart；source 里指定的 matcher
        // （这里是 `ai-config`）应被原样保留（不会被默认 Bash 覆盖）。
        let pre_tool_use = hooks_obj
            .get("PreToolUse")
            .and_then(|v| v.as_array())
            .unwrap();
        let has_speak = pre_tool_use.iter().any(|group| {
            group
                .get("hooks")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter().any(|h| {
                        h.get("command")
                            .and_then(|v| v.as_str())
                            .map(|c| c.contains("speak-lifecycle.py"))
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
        });
        assert!(
            has_speak,
            "beforeShellExecution 应落到 PreToolUse 组：实际 keys={:?}",
            hooks_obj.keys().collect::<Vec<_>>()
        );
    }

    /// 端到端：Codex 端命令应是相对路径（项目级），符合官方「优先 git-root 相对路径」。
    #[test]
    fn deploy_codex_uses_relative_command_path() {
        let tmp = TempDir::new().unwrap();
        let asset = tmp.path().join("asset");
        let hooks = asset.join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        std::fs::write(
            asset.join("hooks.json"),
            r#"{
  "version": 1,
  "hooks": {
    "sessionStart": [
      { "command": "./hooks/speak-lifecycle.py sessionStart" }
    ],
    "postToolUse": [
      { "command": "./hooks/speak-lifecycle.py postToolUse" }
    ]
  }
}"#,
        )
        .unwrap();
        std::fs::write(
            hooks.join("speak-lifecycle.py"),
            "#!/usr/bin/env python3\nprint('ok')\n",
        )
        .unwrap();

        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let asset_u = Utf8PathBuf::from_path_buf(asset).unwrap();
        let repo_u = Utf8PathBuf::from_path_buf(repo.clone()).unwrap();

        deploy(&asset_u, &repo_u, "speak-lifecycle.py", PlatformId::Codex).unwrap();

        let hooks_json = std::fs::read_to_string(repo.join(".codex/hooks.json")).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&hooks_json).unwrap();

        // 顶层键分键：sessionStart → SessionStart，postToolUse → PostToolUse
        assert!(doc["hooks"].get("SessionStart").is_some());
        assert!(doc["hooks"].get("PostToolUse").is_some());
        // sessionStart 不应再被塞进 PostToolUse
        let post_tool_use = doc["hooks"]["PostToolUse"].as_array().unwrap();
        for group in post_tool_use {
            let inner = group.get("hooks").and_then(|v| v.as_array()).unwrap();
            for h in inner {
                let cmd = h.get("command").and_then(|v| v.as_str()).unwrap();
                assert!(
                    !cmd.contains("sessionStart"),
                    "PostToolUse 组内命令不应携带 sessionStart 事件参数：{cmd}"
                );
            }
        }
        // 路径应为相对路径而非绝对路径
        for event in ["SessionStart", "PostToolUse"] {
            for group in doc["hooks"][event].as_array().unwrap() {
                for h in group.get("hooks").and_then(|v| v.as_array()).unwrap() {
                    let cmd = h.get("command").and_then(|v| v.as_str()).unwrap();
                    assert!(
                        cmd.starts_with(".codex/hooks/"),
                        "Codex 项目级命令应使用相对路径：{cmd}"
                    );
                    assert!(
                        !cmd.starts_with('/'),
                        "Codex 项目级命令不应使用绝对路径：{cmd}"
                    );
                }
            }
        }
    }

    #[test]
    fn merge_cursor_preserves_unmanaged_user_hooks() {
        let tmp = TempDir::new().unwrap();
        let asset = tmp.path().join("asset");
        let hooks = asset.join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        std::fs::write(
            asset.join("hooks.json"),
            r#"{
  "version": 1,
  "hooks": {
    "sessionStart": [
      { "command": "./hooks/speak-lifecycle.py sessionStart" }
    ]
  }
}"#,
        )
        .unwrap();
        std::fs::write(hooks.join("speak-lifecycle.py"), "#!/usr/bin/env python3\n").unwrap();

        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join(".cursor/hooks")).unwrap();
        std::fs::write(
            repo.join(".cursor/hooks.json"),
            r#"{
  "version": 1,
  "hooks": {
    "sessionStart": [
      { "command": ".cursor/hooks/session-branch-check.py", "timeout": 10 }
    ],
    "preToolUse": [
      { "command": ".cursor/hooks/block-edit-protected-branch.py", "timeout": 10 }
    ],
    "beforeShellExecution": [
      { "command": ".cursor/hooks/speak-lifecycle.py beforeShellExecution", "timeout": 3 }
    ]
  }
}"#,
        )
        .unwrap();

        let asset_u = Utf8PathBuf::from_path_buf(asset).unwrap();
        let repo_u = Utf8PathBuf::from_path_buf(repo.clone()).unwrap();
        deploy(&asset_u, &repo_u, "speak-lifecycle.py", PlatformId::Cursor).unwrap();

        let doc: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(repo.join(".cursor/hooks.json")).unwrap(),
        )
        .unwrap();
        let hooks_obj = doc["hooks"].as_object().unwrap();
        assert!(hooks_obj.contains_key("preToolUse"));
        assert!(hooks_obj.contains_key("beforeShellExecution"));
        let pre = hooks_obj["preToolUse"].as_array().unwrap();
        assert!(pre.iter().any(|e| e["command"]
            .as_str()
            .is_some_and(|c| c.contains("block-edit-protected-branch.py"))));
    }

    #[test]
    fn merge_claude_settings_preserves_mcp_servers() {
        let tmp = TempDir::new().unwrap();
        let asset = tmp.path().join("asset");
        let hooks = asset.join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        std::fs::write(
            asset.join("hooks.json"),
            r#"{"version":1,"hooks":{"sessionStart":[{"command":"./hooks/speak-lifecycle.py sessionStart"}]}}"#,
        )
        .unwrap();
        std::fs::write(hooks.join("speak-lifecycle.py"), "#!/usr/bin/env python3\n").unwrap();

        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join(".claude")).unwrap();
        std::fs::write(
            repo.join(".claude/settings.json"),
            r#"{
  "mcpServers": { "gitea": { "command": "npx", "args": ["-y", "gitea-mcp"] } },
  "hooks": { "SessionStart": [] }
}"#,
        )
        .unwrap();

        let asset_u = Utf8PathBuf::from_path_buf(asset).unwrap();
        let repo_u = Utf8PathBuf::from_path_buf(repo.clone()).unwrap();
        deploy(&asset_u, &repo_u, "speak-lifecycle.py", PlatformId::Claude).unwrap();

        let doc: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(repo.join(".claude/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(doc["mcpServers"]["gitea"]["command"], "npx");
    }

    #[test]
    fn claude_shared_cursor_hooks_uses_cursor_path() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join(".cursor/hooks")).unwrap();
        std::fs::create_dir_all(repo.join(".claude")).unwrap();
        std::fs::write(
            repo.join(".claude/settings.json"),
            r#"{"hooks":{"PreToolUse":[{"hooks":[{"command":".cursor/hooks/foo.py"}]}]}}"#,
        )
        .unwrap();
        let repo_u = Utf8PathBuf::from_path_buf(repo).unwrap();
        assert!(project_uses_shared_cursor_hooks(&repo_u));
        assert_eq!(
            platform_hooks_dir(&repo_u, PlatformId::Claude),
            repo_u.join(".cursor/hooks")
        );
    }
}
