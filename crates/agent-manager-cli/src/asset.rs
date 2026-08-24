//! `agents-manager skill ...` / `rule ...` / `agent ...` 共享子命令(PRD §5.1 必做)。
//!
//! 维度划分(与 §2 / §11.2 一致):
//! - **skill**:`skills/<name>/SKILL.md`(目录)
//! - **rule**:`rules/<name>.mdc` / `.md`(单文件)
//! - **agent**:`agents/<name>/` 或 `agents/<name>.{md,yaml,json}`
//!
//! 子命令语义:
//! - `list`:`<root>/<kind>s/` 下扫到的 entry 列表
//! - `show <name>`:打印 `name` + 源路径 + 描述(若有 frontmatter)
//! - `reveal <name>`:调 `open` 打开源目录或文件所在目录

use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use agent_manager_core::error::{exit_code, CoreError};
use agent_manager_core::source;

use crate::output::{emit_error_envelope, emit_json, emit_line, OutputMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    Skill,
    Rule,
    Agent,
    Command,
}

impl AssetKind {
    pub fn label(self) -> &'static str {
        match self {
            AssetKind::Skill => "skill",
            AssetKind::Rule => "rule",
            AssetKind::Agent => "agent",
            AssetKind::Command => "command",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AssetEntry {
    pub name: String,
    pub source_path: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub enum AssetCmd {
    List,
    Show { name: String },
    Reveal { name: String },
}

impl AssetCmd {
    pub fn run(self, mode: OutputMode, default_root: &Utf8Path, kind: AssetKind) -> ExitCode {
        match self {
            AssetCmd::List => run_list(mode, default_root, kind),
            AssetCmd::Show { name } => run_show(mode, default_root, kind, &name),
            AssetCmd::Reveal { name } => run_reveal(mode, default_root, kind, &name),
        }
    }
}

fn run_list(mode: OutputMode, root: &Utf8Path, kind: AssetKind) -> ExitCode {
    let entries = match scan_assets(root, kind) {
        Ok(e) => e,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            return ExitCode::from(e.exit_code());
        }
    };
    if mode.is_json() {
        emit_json(mode, &serde_json::json!({ "items": entries }));
    } else {
        for e in &entries {
            println!("{}\t{}", e.name, e.source_path);
        }
    }
    ExitCode::SUCCESS
}

fn run_show(mode: OutputMode, root: &Utf8Path, kind: AssetKind, name: &str) -> ExitCode {
    let entry = match find_by_name(root, kind, name) {
        Ok(e) => e,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            return ExitCode::from(e.exit_code());
        }
    };
    let Some(e) = entry else {
        let msg = format!("{} `{name}` 找不到", kind.label());
        let hint = format!("跑 `agents-manager {} list` 看全部", kind.label());
        emit_error_envelope(mode, exit_code::PARTIAL_FAILURE, &msg, Some(&hint));
        return ExitCode::from(exit_code::PARTIAL_FAILURE);
    };
    let read_path = content_path_for_show(kind, &Utf8PathBuf::from(&e.source_path));
    if mode.is_json() {
        emit_json(
            mode,
            &serde_json::json!({
                "name": e.name,
                "source_path": e.source_path,
                "description": e.description,
                "head": read_head(&read_path, 30),
            }),
        );
    } else {
        println!("name: {}", e.name);
        println!("source: {}", e.source_path);
        if let Some(d) = &e.description {
            println!("description: {d}");
        }
        let body = read_head(&read_path, 30);
        if !body.is_empty() {
            println!("---");
            println!("{body}");
        }
    }
    ExitCode::SUCCESS
}

fn run_reveal(mode: OutputMode, root: &Utf8Path, kind: AssetKind, name: &str) -> ExitCode {
    let entry = match find_by_name(root, kind, name) {
        Ok(e) => e,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            return ExitCode::from(e.exit_code());
        }
    };
    let Some(e) = entry else {
        let msg = format!("{} `{name}` 找不到", kind.label());
        let hint = format!("跑 `agents-manager {} list` 看全部", kind.label());
        emit_error_envelope(mode, exit_code::PARTIAL_FAILURE, &msg, Some(&hint));
        return ExitCode::from(exit_code::PARTIAL_FAILURE);
    };
    let src = Utf8PathBuf::from(&e.source_path);
    let dir_to_open = if src.is_dir() {
        src.clone()
    } else {
        src.parent().map(|p| p.to_path_buf()).unwrap_or(src.clone())
    };
    match open_in_default_app(&dir_to_open) {
        Ok(()) => {
            if mode.is_json() {
                emit_json(
                    mode,
                    &serde_json::json!({
                        "ok": true,
                        "action": "reveal",
                        "name": name,
                        "opened": dir_to_open.as_str(),
                    }),
                );
            } else {
                emit_line(mode, format!("revealed {name} -> {}", dir_to_open));
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            let msg = format!("open 失败: {e}");
            let hint = "检查 `open` / `xdg-open` / `explorer` 是否在 PATH";
            emit_error_envelope(mode, exit_code::FS_ERROR, &msg, Some(hint));
            ExitCode::from(exit_code::FS_ERROR)
        }
    }
}

fn scan_assets(root: &Utf8Path, kind: AssetKind) -> Result<Vec<AssetEntry>, CoreError> {
    let scan = source::scan_project_root(root)?;
    let raw: Vec<Utf8PathBuf> = match kind {
        AssetKind::Skill => scan.skills,
        AssetKind::Rule => scan.rules,
        AssetKind::Agent => scan.agents,
        AssetKind::Command => scan.commands,
    };
    let mut out = Vec::with_capacity(raw.len());
    for path in raw {
        let name = match kind {
            AssetKind::Skill => path
                .parent()
                .and_then(|p| p.file_name())
                .map(|s| s.to_string())
                .unwrap_or_default(),
            AssetKind::Rule => path.file_stem().map(|s| s.to_string()).unwrap_or_default(),
            AssetKind::Command => path.file_stem().map(|s| s.to_string()).unwrap_or_default(),
            AssetKind::Agent => {
                if path.is_dir() {
                    path.file_name().map(|s| s.to_string()).unwrap_or_default()
                } else {
                    path.file_stem().map(|s| s.to_string()).unwrap_or_default()
                }
            }
        };
        let desc_path = content_path_for_show(kind, &path);
        let desc = parse_frontmatter_description(&desc_path);
        out.push(AssetEntry {
            name,
            source_path: path.as_str().to_string(),
            description: desc,
        });
    }
    Ok(out)
}

fn find_by_name(
    root: &Utf8Path,
    kind: AssetKind,
    name: &str,
) -> Result<Option<AssetEntry>, CoreError> {
    let entries = scan_assets(root, kind)?;
    Ok(entries.into_iter().find(|e| e.name == name))
}

/// Agent 目录优先 `AGENT.md`,否则首个 `.md`;其它 kind 用源路径本身。
fn content_path_for_show(kind: AssetKind, src: &Utf8Path) -> Utf8PathBuf {
    if kind != AssetKind::Agent {
        return src.to_path_buf();
    }
    if !src.is_dir() {
        return src.to_path_buf();
    }
    let agent_md = src.join("AGENT.md");
    if agent_md.is_file() {
        return agent_md;
    }
    if let Ok(entries) = std::fs::read_dir(src.as_std_path()) {
        for entry in entries.flatten() {
            if let Ok(path) = Utf8PathBuf::from_path_buf(entry.path()) {
                if path.extension().map(|e| e == "md").unwrap_or(false) {
                    return path;
                }
            }
        }
    }
    agent_md
}

/// 极简 frontmatter 解析:读 SKILL.md / .mdc / agent 正文,识别 `description:`。
fn parse_frontmatter_description(path: &Utf8Path) -> Option<String> {
    let content = std::fs::read_to_string(path.as_std_path()).ok()?;
    if !content.starts_with("---") {
        return None;
    }
    let after_first = content[3..].trim_start_matches('\n');
    let end = after_first.find("\n---")?;
    let front = &after_first[..end];
    for line in front.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("description:") {
            return Some(rest.trim().trim_matches(['"', '\'']).to_string());
        }
    }
    None
}

fn read_head(path: &Utf8Path, max_lines: usize) -> String {
    let Ok(content) = std::fs::read_to_string(path.as_std_path()) else {
        return String::new();
    };
    content
        .lines()
        .take(max_lines)
        .collect::<Vec<_>>()
        .join("\n")
}

fn open_in_default_app(path: &Utf8Path) -> Result<(), String> {
    let path_str = path.as_str();
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path_str)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path_str)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(path_str)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
