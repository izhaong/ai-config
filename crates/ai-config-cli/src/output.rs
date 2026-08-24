//! 输出模式 + 共享渲染 / 错误出口(PRD §9.1)。
//!
//! 三态:
//! - `Human`:默认,带颜色 / 表格 / 进度
//! - `Json`:`--json`,全程机器可解析,纯结构化输出
//! - `Quiet`:`--quiet`,只输完成/失败一行
//!
//! `--json --quiet` 同时出现时 `Json` 胜出(`--json` 是机器契约,优先)。

use std::io::Write;
use std::process::ExitCode;

/// 输出模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Human,
    Json,
    Quiet,
}

impl OutputMode {
    pub fn from_flags(json: bool, quiet: bool) -> Self {
        if json {
            Self::Json
        } else if quiet {
            Self::Quiet
        } else {
            Self::Human
        }
    }

    pub fn is_json(self) -> bool {
        matches!(self, Self::Json)
    }

    pub fn is_quiet(self) -> bool {
        matches!(self, Self::Quiet)
    }

    pub fn is_human(self) -> bool {
        matches!(self, Self::Human)
    }
}

/// 输出 JSON 值(机器可解析):仅在 `--json` 模式下打印。
pub fn emit_json<T: serde::Serialize>(mode: OutputMode, value: &T) {
    if mode.is_json() {
        match serde_json::to_string_pretty(value) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("[agent-manager] serialize error: {e}"),
        }
    }
}

/// 输出人类友好的"一行结果"。Quiet 模式下不打印。
pub fn emit_line(mode: OutputMode, line: impl AsRef<str>) {
    if !mode.is_quiet() {
        println!("{}", line.as_ref());
    }
}

/// 错误信封(对齐 PRD §9.1):`--json` 走 stdout,其它走 stderr。
pub fn emit_error_envelope(mode: OutputMode, code: u8, msg: &str, hint: Option<&str>) {
    if mode.is_json() {
        let mut payload = serde_json::Map::new();
        payload.insert("code".into(), serde_json::json!(code));
        payload.insert("message".into(), serde_json::json!(msg));
        if let Some(h) = hint {
            payload.insert("hint".into(), serde_json::json!(h));
        }
        let envelope = serde_json::json!({ "error": payload });
        let s = serde_json::to_string(&envelope).unwrap_or_else(|_| {
            r#"{"error":{"code":2,"message":"internal: serialize error"}}"#.to_string()
        });
        let _ = writeln!(std::io::stdout().lock(), "{s}");
    } else if mode.is_quiet() {
        let _ = writeln!(std::io::stderr().lock(), "fail: {msg}");
    } else {
        let _ = writeln!(std::io::stderr().lock(), "error: {msg}");
        if let Some(h) = hint {
            let _ = writeln!(std::io::stderr().lock(), "hint: {h}");
        }
    }
}

/// 把任意 `anyhow::Error` 翻译成退出码(优先看根因是不是 `CoreError`)。
#[allow(dead_code)] // 供未来 subcommand 复用
pub fn report_anyhow(mode: OutputMode, err: anyhow::Error) -> ExitCode {
    if let Some(core) = err.downcast_ref::<ai_config_core::error::CoreError>() {
        let code = core.exit_code();
        emit_error_envelope(mode, code, &core.to_string(), core.hint());
        return ExitCode::from(code);
    }
    let code = ai_config_core::error::exit_code::FS_ERROR;
    emit_error_envelope(mode, code, &err.to_string(), None);
    ExitCode::from(code)
}
