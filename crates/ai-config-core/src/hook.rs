//! Hook 源资产：`<asset_root>/hooks.json`（Cursor 格式 canonical）+ `hooks/<脚本文件>`。

use std::collections::HashMap;
use std::fs;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::CoreError;
use crate::model::PlatformId;

pub const HOOKS_SUBDIR: &str = "hooks";
pub const HOOKS_MANIFEST: &str = "hooks.json";

/// Hook 列表项类型（与 Cursor `hooks.json` 条目 `type` 对齐）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HookEntryType {
    Command,
    Prompt,
}

/// GUI 列表一行：合并 `hooks.json` 条目与 `hooks/` 下脚本/目录，按资产名去重。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookCatalogItem {
    /// 列表 title：脚本文件名、目录包名或 prompt 摘要。
    pub name: String,
    pub hook_type: HookEntryType,
    /// command 型：脚本 docstring 首段；prompt 型为空。
    pub description: String,
    pub source_path: Utf8PathBuf,
    /// 生命周期切换 / deploy 用的绑定键（脚本名或 `prompt:<lifecycle>:<index>`）。
    pub binding_key: String,
}

/// `hooks.json` 中一条生命周期绑定（列表一行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookListItem {
    /// 平台原生或 canonical 生命周期键，如 `afterShellExecution`。
    pub lifecycle: String,
    /// 脚本文件名（`command` 路径末段），列表 title / 唯一键。
    pub script_filename: String,
    /// 源侧脚本绝对路径。
    pub script_path: Utf8PathBuf,
    /// 源侧 `hooks.json` 路径。
    pub manifest_path: Utf8PathBuf,
}

/// 下发用的脚本 + 其在 manifest 中的全部绑定。
#[derive(Debug, Clone)]
pub struct HookScriptSpec {
    pub filename: String,
    pub script_path: Utf8PathBuf,
    pub description: String,
    pub bindings: Vec<HookBinding>,
}

#[derive(Debug, Clone)]
pub struct HookBinding {
    pub lifecycle: String,
    pub matcher: Option<String>,
    /// 源 manifest 内原始 `command`（相对路径，如 `./hooks/foo.sh`）。
    pub command: String,
}

#[derive(Debug, Deserialize)]
struct HooksManifestFile {
    #[serde(default, rename = "version")]
    _version: u32,
    hooks: HashMap<String, Vec<HooksManifestEntry>>,
}

#[derive(Debug, Deserialize)]
struct HooksManifestEntry {
    #[serde(default)]
    command: String,
    #[serde(default)]
    matcher: Option<String>,
    #[serde(default)]
    r#type: Option<String>,
    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

pub fn hooks_dir(asset_root: &Utf8Path) -> Utf8PathBuf {
    asset_root.join(HOOKS_SUBDIR)
}

pub fn manifest_path(asset_root: &Utf8Path) -> Utf8PathBuf {
    asset_root.join(HOOKS_MANIFEST)
}

pub fn script_path(asset_root: &Utf8Path, filename: &str) -> Utf8PathBuf {
    hooks_dir(asset_root).join(filename)
}

/// 源侧 hook 资产路径（单文件或 bundle 目录）。
pub fn source_entry_path(asset_root: &Utf8Path, asset_name: &str) -> Utf8PathBuf {
    script_path(asset_root, asset_name)
}

fn binding_matches_asset(command: &str, asset_name: &str, asset_root: &Utf8Path) -> bool {
    asset_key_from_command(command, asset_root).as_deref() == Some(asset_name)
        || filename_from_command(command).as_deref() == Some(asset_name)
}

/// 从 `command` 字段提取脚本文件名（忽略参数；支持 `./hooks/x.sh` 与 `.cursor/hooks/x.py`）。
pub fn filename_from_command(command: &str) -> Option<String> {
    let executable = command.split_whitespace().next()?.trim();
    if executable.is_empty() {
        return None;
    }
    let path = executable.strip_prefix("./").unwrap_or(executable);
    Utf8Path::new(path).file_name().map(str::to_string)
}

pub fn canonical_command_for_source(command: &str, script_filename: &str) -> String {
    let args: String = command
        .split_whitespace()
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ");
    if args.is_empty() {
        format!("./hooks/{script_filename}")
    } else {
        format!("./hooks/{script_filename} {args}")
    }
}

/// 为源 `hooks.json` 推断新绑定的 `command`（保留带事件参数的脚本风格）。
pub fn infer_binding_command(
    script_filename: &str,
    lifecycle: &str,
    bindings: &[HookBinding],
) -> String {
    if let Some(existing) = bindings.iter().find(|b| b.lifecycle == lifecycle) {
        return canonical_command_for_source(&existing.command, script_filename);
    }
    if script_filename.contains("speak-lifecycle")
        || bindings
            .iter()
            .any(|b| b.command.split_whitespace().count() > 1)
    {
        format!("./hooks/{script_filename} {lifecycle}")
    } else {
        format!("./hooks/{script_filename}")
    }
}

/// 由资产根推导平台下发根（`<repo>` 或 `$HOME`）。
pub fn deploy_base_for_asset_root(asset_root: &Utf8Path) -> Utf8PathBuf {
    asset_root
        .parent()
        .filter(|p| !p.as_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(crate::paths::global_deploy_base)
}

/// 收集脚本的 hooks 绑定：源 `hooks.json` 优先，否则回退读各 IDE 平台配置。
pub fn collect_bindings_for_script(
    asset_root: &Utf8Path,
    deploy_base: &Utf8Path,
    script_filename: &str,
) -> Result<Vec<HookBinding>, CoreError> {
    let mut bindings = Vec::new();
    if script_path(asset_root, script_filename).is_file() {
        bindings = load_spec(asset_root, script_filename)?.bindings;
    }
    if bindings.is_empty() {
        for plat in [
            crate::model::PlatformId::Cursor,
            crate::model::PlatformId::Codex,
            crate::model::PlatformId::Claude,
            crate::model::PlatformId::Hermes,
        ] {
            if let Ok(found) =
                crate::hook_adapter::read_platform_bindings(deploy_base, plat, script_filename)
            {
                if !found.is_empty() {
                    bindings = found;
                    break;
                }
            }
        }
    }
    Ok(bindings)
}

pub(crate) fn is_hook_script_file(name: &str) -> bool {
    if name.starts_with('.') {
        return false;
    }
    matches!(
        name.rsplit('.').next(),
        Some("sh" | "bash" | "zsh" | "py" | "rb" | "pl" | "js" | "ts")
    )
}

/// 脚本开头 `"""..."""` 描述（shebang 之后亦可）。
pub fn parse_script_description(content: &str) -> String {
    let mut lines = content.lines();
    if let Some(first) = lines.next() {
        if first.starts_with("#!") {
            let rest = lines.collect::<Vec<_>>().join("\n");
            return extract_triple_quote(&rest);
        }
        return extract_triple_quote(content);
    }
    String::new()
}

fn extract_triple_quote(content: &str) -> String {
    let trimmed = content.trim_start();
    let Some(rest) = trimmed.strip_prefix("\"\"\"") else {
        return String::new();
    };
    let Some(end) = rest.find("\"\"\"") else {
        return String::new();
    };
    rest[..end].trim().to_string()
}

/// 解析 `hooks.json`，列出每个生命周期下的对象。
pub fn list_from_manifest(
    manifest: &Utf8Path,
    asset_root: &Utf8Path,
) -> Result<Vec<HookListItem>, CoreError> {
    if !manifest.is_file() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(manifest.as_std_path()).map_err(CoreError::Io)?;
    let doc: HooksManifestFile =
        serde_json::from_str(&raw).map_err(|e| CoreError::TemplateRender {
            template: manifest.as_str().to_owned(),
            reason: e.to_string(),
            hint: "hooks.json 格式无效".into(),
        })?;

    let mut out = Vec::new();
    for (lifecycle, entries) in &doc.hooks {
        for entry in entries {
            if entry.command.is_empty() {
                continue;
            }
            let Some(script_filename) = filename_from_command(&entry.command) else {
                continue;
            };
            let script_path = resolve_script_path(asset_root, &entry.command)?;
            if !script_path.is_file() {
                continue;
            }
            out.push(HookListItem {
                lifecycle: lifecycle.clone(),
                script_filename,
                script_path,
                manifest_path: manifest.to_path_buf(),
            });
        }
    }
    out.sort_by(|a, b| (&a.lifecycle, &a.script_filename).cmp(&(&b.lifecycle, &b.script_filename)));
    Ok(out)
}

fn resolve_script_path(asset_root: &Utf8Path, command: &str) -> Result<Utf8PathBuf, CoreError> {
    let executable = command
        .split_whitespace()
        .next()
        .ok_or_else(|| CoreError::InvalidPath(format!("空 command: {command}")))?;
    let rel = executable
        .trim()
        .strip_prefix("./")
        .unwrap_or(executable.trim());
    let path = Utf8Path::new(rel);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    if let Some(name) = path.file_name() {
        return Ok(hooks_dir(asset_root).join(name));
    }
    Ok(hooks_dir(asset_root).join(rel))
}

/// 从 `command` 解析可同步资产名：单文件取文件名；`hooks/<bundle>/…` 且 `<bundle>` 为目录时取目录名。
pub fn asset_key_from_command(command: &str, asset_root: &Utf8Path) -> Option<String> {
    let executable = command.split_whitespace().next()?.trim();
    if executable.is_empty() {
        return None;
    }
    let rel = executable.strip_prefix("./").unwrap_or(executable);
    let path = Utf8Path::new(rel);
    let mut components = path.iter().collect::<Vec<_>>();
    if components.first() == Some(&"hooks") && components.len() >= 3 {
        return Some(components[1].to_string());
    }
    if components.first() == Some(&"hooks") && components.len() >= 2 {
        let bundle = components[1].to_string();
        if hooks_dir(asset_root).join(&bundle).is_dir() {
            return Some(bundle);
        }
        components.remove(0);
    }
    path.file_name().map(str::to_string)
}

pub fn prompt_entry_title(prompt: &str) -> String {
    let line = prompt.lines().next().unwrap_or("prompt").trim();
    if line.chars().count() > 48 {
        let truncated: String = line.chars().take(48).collect();
        format!("{truncated}…")
    } else {
        line.to_string()
    }
}

fn prompt_binding_key(lifecycle: &str, index: usize) -> String {
    format!("prompt:{lifecycle}:{index}")
}

fn read_bundle_description(bundle_dir: &Utf8Path) -> String {
    for name in ["HOOK.md", "hook.yaml", "README.md"] {
        let path = bundle_dir.join(name);
        if path.is_file() {
            if let Ok(content) = fs::read_to_string(path.as_std_path()) {
                let desc = if name.ends_with(".md") {
                    content.lines().next().unwrap_or("").trim().to_string()
                } else {
                    parse_script_description(&content)
                };
                if !desc.is_empty() {
                    return desc;
                }
            }
        }
    }
    String::new()
}

/// 合并 `hooks.json` 与 `hooks/` 目录，按资产名去重后返回 GUI 列表。
pub fn list_hook_catalog(asset_root: &Utf8Path) -> Result<Vec<HookCatalogItem>, CoreError> {
    let manifest = manifest_path(asset_root);
    let hooks_root = hooks_dir(asset_root);
    let mut by_key: HashMap<String, HookCatalogItem> = HashMap::new();

    if hooks_root.is_dir() {
        for entry in fs::read_dir(hooks_root.as_std_path()).map_err(CoreError::Io)? {
            let entry = entry.map_err(CoreError::Io)?;
            let path = Utf8PathBuf::from_path_buf(entry.path()).unwrap_or_default();
            let Some(name) = path.file_name().map(str::to_string) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            if path.is_file() {
                if !is_hook_script_file(&name) {
                    continue;
                }
                let description = fs::read_to_string(path.as_std_path())
                    .ok()
                    .map(|c| parse_script_description(&c))
                    .unwrap_or_default();
                by_key.insert(
                    name.clone(),
                    HookCatalogItem {
                        name: name.clone(),
                        hook_type: HookEntryType::Command,
                        description,
                        source_path: path,
                        binding_key: name,
                    },
                );
            } else if path.is_dir() {
                by_key.insert(
                    name.clone(),
                    HookCatalogItem {
                        name: name.clone(),
                        hook_type: HookEntryType::Command,
                        description: read_bundle_description(&path),
                        source_path: path,
                        binding_key: name,
                    },
                );
            }
        }
    }

    if manifest.is_file() {
        let raw = fs::read_to_string(manifest.as_std_path()).map_err(CoreError::Io)?;
        let doc: HooksManifestFile =
            serde_json::from_str(&raw).map_err(|e| CoreError::TemplateRender {
                template: manifest.as_str().to_owned(),
                reason: e.to_string(),
                hint: "hooks.json 格式无效".into(),
            })?;
        for (lifecycle, entries) in &doc.hooks {
            for (index, entry) in entries.iter().enumerate() {
                let entry_type = entry
                    .r#type
                    .as_deref()
                    .unwrap_or("command")
                    .to_ascii_lowercase();
                let prompt_text = entry
                    .extra
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                if entry_type == "prompt" || (entry.command.is_empty() && prompt_text.is_some()) {
                    let prompt = prompt_text.unwrap_or_default();
                    let key = prompt_binding_key(lifecycle, index);
                    let title = if prompt.is_empty() {
                        format!("prompt@{lifecycle}")
                    } else {
                        prompt_entry_title(&prompt)
                    };
                    by_key.insert(
                        key.clone(),
                        HookCatalogItem {
                            name: title,
                            hook_type: HookEntryType::Prompt,
                            description: String::new(),
                            source_path: manifest.clone(),
                            binding_key: key,
                        },
                    );
                    continue;
                }
                if entry.command.is_empty() {
                    continue;
                }
                let Some(key) = asset_key_from_command(&entry.command, asset_root) else {
                    continue;
                };
                let script_path = resolve_script_path(asset_root, &entry.command)?;
                let description = if script_path.is_file() {
                    fs::read_to_string(script_path.as_std_path())
                        .ok()
                        .map(|c| parse_script_description(&c))
                        .unwrap_or_default()
                } else if script_path.is_dir() {
                    read_bundle_description(&script_path)
                } else {
                    String::new()
                };
                by_key.insert(
                    key.clone(),
                    HookCatalogItem {
                        name: key.clone(),
                        hook_type: HookEntryType::Command,
                        description,
                        source_path: script_path,
                        binding_key: key,
                    },
                );
            }
        }
    }

    let mut out: Vec<HookCatalogItem> = by_key.into_values().collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// 按脚本文件名加载 spec（manifest 中凡指向该文件名的绑定一并纳入）。
pub fn load_spec(
    asset_root: &Utf8Path,
    script_filename: &str,
) -> Result<HookScriptSpec, CoreError> {
    let script_path = script_path(asset_root, script_filename);
    if !script_path.is_file() {
        return Err(CoreError::AssetNotFound {
            kind: crate::model::AssetKind::Hook,
            name: script_filename.into(),
            hint: format!("缺少脚本 {script_path}"),
        });
    }
    let manifest = manifest_path(asset_root);
    let content = fs::read_to_string(script_path.as_std_path()).map_err(CoreError::Io)?;
    let description = parse_script_description(&content);
    let mut bindings = Vec::new();
    if manifest.is_file() {
        let raw = fs::read_to_string(manifest.as_std_path()).map_err(CoreError::Io)?;
        let doc: HooksManifestFile =
            serde_json::from_str(&raw).map_err(|e| CoreError::TemplateRender {
                template: manifest.as_str().to_owned(),
                reason: e.to_string(),
                hint: "hooks.json 格式无效".into(),
            })?;
        for (lifecycle, entries) in &doc.hooks {
            for entry in entries {
                if filename_from_command(&entry.command).as_deref() == Some(script_filename) {
                    bindings.push(HookBinding {
                        lifecycle: lifecycle.clone(),
                        matcher: entry.matcher.clone(),
                        command: entry.command.clone(),
                    });
                }
            }
        }
    }
    Ok(HookScriptSpec {
        filename: script_filename.to_string(),
        script_path,
        description,
        bindings,
    })
}

pub fn managed_command_token(script_filename: &str) -> String {
    format!("/hooks/{script_filename}")
}

/// 内容指纹：文件名 + 正文（判断唯一性）。
pub fn script_identity(path: &Utf8Path) -> Result<String, CoreError> {
    let name = path
        .file_name()
        .ok_or_else(|| CoreError::InvalidPath(format!("无效脚本路径: {path}")))?;
    let content = fs::read_to_string(path.as_std_path()).map_err(CoreError::Io)?;
    Ok(format!("{name}:{content}"))
}

/// canonical 生命周期 ID（跨平台统一为 Cursor `hooks.json` 键名）。
pub fn normalize_lifecycle(lifecycle: &str) -> String {
    match lifecycle {
        "after_shell" | "afterShellExecution" => "afterShellExecution".into(),
        "before_shell" | "beforeShellExecution" => "beforeShellExecution".into(),
        "postToolUse" | "PostToolUse" => "postToolUse".into(),
        "preToolUse" | "PreToolUse" => "preToolUse".into(),
        "postToolUseFailure" | "PostToolUseFailure" => "postToolUseFailure".into(),
        "pre_tool_call" => "beforeShellExecution".into(),
        "post_tool_call" => "afterShellExecution".into(),
        other => other.to_string(),
    }
}

/// 将平台原生事件名映射为 canonical ID；Codex/Claude 的 `PreToolUse`/`PostToolUse` 需结合 matcher 区分工具与 Shell。
pub fn canonical_lifecycle_id(
    native_lifecycle: &str,
    matcher: Option<&str>,
    plat: crate::model::PlatformId,
) -> String {
    use crate::model::PlatformId;
    let matcher_has_bash = matcher
        .map(|m| m.split(['|', ',']).any(|part| part.trim() == "Bash"))
        .unwrap_or(false);
    match plat {
        PlatformId::Codex | PlatformId::Claude => match native_lifecycle {
            "PreToolUse" => {
                if matcher_has_bash {
                    "beforeShellExecution".into()
                } else {
                    "preToolUse".into()
                }
            }
            "PostToolUse" => {
                if matcher_has_bash {
                    "afterShellExecution".into()
                } else {
                    "postToolUse".into()
                }
            }
            other => normalize_lifecycle(other),
        },
        PlatformId::Hermes => match native_lifecycle {
            "pre_tool_call" => "beforeShellExecution".into(),
            "post_tool_call" => "afterShellExecution".into(),
            other => normalize_lifecycle(other),
        },
        _ => normalize_lifecycle(native_lifecycle),
    }
}

/// 从源 `hooks.json` 移除指向该脚本文件名的全部绑定。
pub fn remove_bindings_for_script(
    asset_root: &Utf8Path,
    script_filename: &str,
) -> Result<(), CoreError> {
    let manifest = manifest_path(asset_root);
    if !manifest.is_file() {
        return Ok(());
    }
    let raw = fs::read_to_string(manifest.as_std_path()).map_err(CoreError::Io)?;
    let mut doc: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| CoreError::TemplateRender {
            template: manifest.as_str().to_owned(),
            reason: e.to_string(),
            hint: "hooks.json 格式无效".into(),
        })?;
    let Some(hooks) = doc.get_mut("hooks").and_then(|v| v.as_object_mut()) else {
        return Ok(());
    };
    for entries in hooks.values_mut() {
        if let Some(arr) = entries.as_array_mut() {
            arr.retain(|e| {
                e.get("command")
                    .and_then(|v| v.as_str())
                    .map_or(true, |cmd| {
                        !binding_matches_asset(cmd, script_filename, asset_root)
                    })
            });
        }
    }
    hooks.retain(|_, entries| entries.as_array().is_some_and(|a| !a.is_empty()));
    let out = serde_json::to_string_pretty(&doc).map_err(|e| CoreError::Io(e.into()))?;
    fs::write(manifest.as_std_path(), out).map_err(CoreError::Io)
}

/// 从源 `hooks.json` 移除某脚本在单个生命周期下的绑定。
pub fn remove_binding_for_script_lifecycle(
    asset_root: &Utf8Path,
    script_filename: &str,
    lifecycle: &str,
) -> Result<(), CoreError> {
    let manifest = manifest_path(asset_root);
    if !manifest.is_file() {
        return Ok(());
    }
    let raw = fs::read_to_string(manifest.as_std_path()).map_err(CoreError::Io)?;
    let mut doc: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| CoreError::TemplateRender {
            template: manifest.as_str().to_owned(),
            reason: e.to_string(),
            hint: "hooks.json 格式无效".into(),
        })?;
    let Some(hooks) = doc.get_mut("hooks").and_then(|v| v.as_object_mut()) else {
        return Ok(());
    };
    let Some(entries) = hooks.get_mut(lifecycle).and_then(|v| v.as_array_mut()) else {
        return Ok(());
    };
    entries.retain(|e| {
        e.get("command")
            .and_then(|v| v.as_str())
            .map_or(true, |cmd| {
                !binding_matches_asset(cmd, script_filename, asset_root)
            })
    });
    if entries.is_empty() {
        hooks.remove(lifecycle);
    }
    hooks.retain(|_, entries| entries.as_array().is_some_and(|a| !a.is_empty()));
    let out = serde_json::to_string_pretty(&doc).map_err(|e| CoreError::Io(e.into()))?;
    fs::write(manifest.as_std_path(), out).map_err(CoreError::Io)
}

/// 将绑定合并进源 `hooks.json`（按脚本文件名去重后追加）。
pub fn merge_bindings_into_manifest(
    asset_root: &Utf8Path,
    script_filename: &str,
    bindings: &[HookBinding],
) -> Result<(), CoreError> {
    if bindings.is_empty() {
        return Ok(());
    }
    let manifest = manifest_path(asset_root);
    let mut doc: serde_json::Value = if manifest.is_file() {
        let raw = fs::read_to_string(manifest.as_std_path()).map_err(CoreError::Io)?;
        serde_json::from_str(&raw).map_err(|e| CoreError::TemplateRender {
            template: manifest.as_str().to_owned(),
            reason: e.to_string(),
            hint: "hooks.json 格式无效".into(),
        })?
    } else {
        json!({ "version": 1, "hooks": {} })
    };
    let hooks = doc
        .as_object_mut()
        .ok_or_else(|| CoreError::InvalidPath("hooks.json 根须为对象".into()))?
        .entry("hooks")
        .or_insert_with(|| json!({}));
    let hooks_obj = hooks
        .as_object_mut()
        .ok_or_else(|| CoreError::InvalidPath("hooks.json.hooks 须为对象".into()))?;
    for binding in bindings {
        let arr = hooks_obj
            .entry(binding.lifecycle.clone())
            .or_insert_with(|| json!([]));
        let items = arr.as_array_mut().ok_or_else(|| {
            CoreError::InvalidPath(format!("hooks.{} 须为数组", binding.lifecycle))
        })?;
        items.retain(|e| {
            e.get("command")
                .and_then(|v| v.as_str())
                .and_then(filename_from_command)
                .as_deref()
                != Some(script_filename)
        });
        let mut item = json!({
            "command": canonical_command_for_source(&binding.command, script_filename),
        });
        if let Some(m) = &binding.matcher {
            item.as_object_mut()
                .unwrap()
                .insert("matcher".into(), json!(m));
        }
        items.push(item);
    }
    if doc.get("version").is_none() {
        doc.as_object_mut()
            .unwrap()
            .insert("version".into(), json!(1));
    }
    if let Some(parent) = manifest.parent() {
        fs::create_dir_all(parent.as_std_path()).map_err(CoreError::Io)?;
    }
    fs::create_dir_all(hooks_dir(asset_root).as_std_path()).map_err(CoreError::Io)?;
    let out = serde_json::to_string_pretty(&doc).map_err(|e| CoreError::Io(e.into()))?;
    fs::write(manifest.as_std_path(), out).map_err(CoreError::Io)
}

pub fn enabled_on_platform(_spec: &HookScriptSpec, _plat: PlatformId) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parse_triple_quote_after_shebang() {
        let content = "#!/usr/bin/env bash\n\"\"\"hello hook\"\"\"\necho hi\n";
        assert_eq!(parse_script_description(content), "hello hook");
    }

    #[test]
    fn list_from_manifest_reads_cursor_format() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let hooks = hooks_dir(&root);
        fs::create_dir_all(&hooks).unwrap();
        fs::write(
            root.join("hooks.json"),
            r#"{
  "version": 1,
  "hooks": {
    "afterShellExecution": [
      { "command": "./hooks/demo.sh", "matcher": "ai-config" }
    ]
  }
}"#,
        )
        .unwrap();
        fs::write(hooks.join("demo.sh"), "#!/bin/sh\n\"\"\"demo desc\"\"\"\n").unwrap();
        let items = list_from_manifest(&root.join("hooks.json"), &root).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].script_filename, "demo.sh");
        assert_eq!(items[0].lifecycle, "afterShellExecution");
        let spec = load_spec(&root, "demo.sh").unwrap();
        assert_eq!(spec.description, "demo desc");
        assert_eq!(spec.bindings.len(), 1);
    }

    #[test]
    fn filename_from_command_strips_args_and_cursor_path() {
        assert_eq!(
            filename_from_command(".cursor/hooks/speak-lifecycle.py sessionStart").as_deref(),
            Some("speak-lifecycle.py")
        );
        assert_eq!(
            filename_from_command("./hooks/demo.sh").as_deref(),
            Some("demo.sh")
        );
    }

    #[test]
    fn canonical_command_preserves_event_args() {
        assert_eq!(
            canonical_command_for_source(
                ".cursor/hooks/speak-lifecycle.py sessionStart",
                "speak-lifecycle.py"
            ),
            "./hooks/speak-lifecycle.py sessionStart"
        );
    }

    #[test]
    fn canonical_lifecycle_id_maps_platform_aliases() {
        use crate::model::PlatformId;
        assert_eq!(
            canonical_lifecycle_id("PostToolUse", None, PlatformId::Codex),
            "postToolUse"
        );
        assert_eq!(
            canonical_lifecycle_id("PostToolUse", Some("Bash"), PlatformId::Claude),
            "afterShellExecution"
        );
        assert_eq!(
            canonical_lifecycle_id("PreToolUse", Some("Edit|Bash"), PlatformId::Codex),
            "beforeShellExecution"
        );
        assert_eq!(
            canonical_lifecycle_id("pre_tool_call", None, PlatformId::Hermes),
            "beforeShellExecution"
        );
        assert_eq!(normalize_lifecycle("before_shell"), "beforeShellExecution");
    }

    #[test]
    fn list_hook_catalog_dedupes_manifest_and_orphans() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(root.join("hooks")).unwrap();
        fs::write(
            root.join("hooks/speak-lifecycle.py"),
            "#!/usr/bin/env python3\n\"\"\"生命周期 TTS\"\"\"\n",
        )
        .unwrap();
        fs::write(
            root.join("hooks.json"),
            r#"{"version":1,"hooks":{"sessionStart":[{"command":"./hooks/speak-lifecycle.py sessionStart"}]}}"#,
        )
        .unwrap();
        let catalog = list_hook_catalog(&root).unwrap();
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].name, "speak-lifecycle.py");
        assert_eq!(catalog[0].hook_type, HookEntryType::Command);
        assert!(catalog[0].description.contains("生命周期"));
    }

    #[test]
    fn asset_key_from_command_prefers_bundle_dir() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(root.join("hooks/my-bundle")).unwrap();
        assert_eq!(
            asset_key_from_command("./hooks/my-bundle/run.sh", &root).as_deref(),
            Some("my-bundle")
        );
    }

    #[test]
    fn remove_bindings_for_bundle_asset_name() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(root.join("hooks/lifecycle-tts/scripts")).unwrap();
        fs::write(
            root.join("hooks.json"),
            r#"{"version":1,"hooks":{"afterShellExecution":[{"command":"./hooks/lifecycle-tts/scripts/run.sh afterShellExecution"}]}}"#,
        )
        .unwrap();
        remove_bindings_for_script(&root, "lifecycle-tts").unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.join("hooks.json")).unwrap()).unwrap();
        assert!(doc["hooks"].as_object().unwrap().is_empty());
    }
}
