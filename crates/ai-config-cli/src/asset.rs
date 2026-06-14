//! `ai-config skill ...` / `ai-config rule ...` 共享子命令(PRD §5.1 必做)。
//!
//! 维度划分(与 §2 / §11.2 一致):
//! - **skill**:`skills/<name>/SKILL.md`(目录)
//! - **rule**:`rules/<name>.mdc` / `.md`(单文件,无 scope,单一源)
//!
//! 子命令语义:
//! - `list`:`<root>/<kind>s/` 下扫到的 entry 列表
//! - `show <name>`:打印 `name` + 源路径 + 描述(若有 frontmatter)
//! - `reveal <name>`:调 `open` 打开 IDE skills / rules 目录(人工查看用)

use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use ai_config_core::error::{exit_code, CoreError};
use ai_config_core::source;

use crate::output::{emit_error_envelope, emit_json, emit_line, OutputMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    Skill,
    Rule,
}

impl AssetKind {
    pub fn label(self) -> &'static str {
        match self {
            AssetKind::Skill => "skill",
            AssetKind::Rule => "rule",
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
        let hint = format!("跑 `ai-config {} list` 看全部", kind.label());
        emit_error_envelope(mode, exit_code::PARTIAL_FAILURE, &msg, Some(&hint));
        return ExitCode::from(exit_code::PARTIAL_FAILURE);
    };
    if mode.is_json() {
        emit_json(
            mode,
            &serde_json::json!({
                "name": e.name,
                "source_path": e.source_path,
                "description": e.description,
                "head": read_head(&Utf8PathBuf::from(&e.source_path), 30),
            }),
        );
    } else {
        println!("name: {}", e.name);
        println!("source: {}", e.source_path);
        if let Some(d) = &e.description {
            println!("description: {d}");
        }
        let body = read_head(&Utf8PathBuf::from(&e.source_path), 30);
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
        let hint = format!("跑 `ai-config {} list` 看全部", kind.label());
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
        };
        let desc = parse_frontmatter_description(&path);
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

/// 极简 frontmatter 解析:读 SKILL.md / .mdc 的开头,识别
/// `---\n...description: ...\n---\n`,只取 `description` 一行。
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
