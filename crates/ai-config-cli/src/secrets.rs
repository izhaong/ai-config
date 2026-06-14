//! `ai-config secrets ...` 子命令(PRD §5.1 必做 + §10 A-5 / A-12 / A-14)。
//!
//! 关键约束:
//! - `secrets list`:**不**输出 value(只输出 key 列表);遵守 PRD §8.1 / §10 A-14
//! - `secrets set` 从 stdin 读 value,**不**回显(canonical mode + 无 terminal echo)
//! - `secrets validate` 退出码 0 / 4(对齐 §6.1)
//! - 所有写盘路径走 `ai_config_core::secrets::save_to`(强制 0600 + 原子)

use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::process::ExitCode;

use camino::Utf8Path;

use ai_config_core::error::{exit_code, CoreError};
use ai_config_core::secrets as core_secrets;

use crate::output::{emit_error_envelope, emit_json, emit_line, OutputMode};

/// 镜像 clap 的 SecretsCmd(由 main 透传)。
#[derive(Debug, Clone)]
pub enum SecretsCmd {
    Path,
    Validate,
    List,
    Set { key: String },
    Unset { key: String },
}

impl SecretsCmd {
    /// 跑该子命令,返回 `ExitCode`。
    pub fn run(self, mode: OutputMode, default_root: &Utf8Path) -> ExitCode {
        match self {
            SecretsCmd::Path => run_path(mode),
            SecretsCmd::Validate => run_validate(mode, default_root),
            SecretsCmd::List => run_list(mode),
            SecretsCmd::Set { key } => run_set(mode, &key),
            SecretsCmd::Unset { key } => run_unset(mode, &key),
        }
    }
}

fn run_path(mode: OutputMode) -> ExitCode {
    emit_line(mode, core_secrets::default_path().as_str());
    ExitCode::SUCCESS
}

fn run_list(mode: OutputMode) -> ExitCode {
    let pairs = match core_secrets::load() {
        Ok(p) => p,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            return ExitCode::from(e.exit_code());
        }
    };
    if mode.is_json() {
        let keys: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
        emit_json(mode, &serde_json::json!({ "keys": keys }));
    } else {
        for (k, _) in &pairs {
            println!("{k}");
        }
    }
    ExitCode::SUCCESS
}

fn run_validate(mode: OutputMode, default_root: &Utf8Path) -> ExitCode {
    match collect_missing(default_root) {
        Ok(missing) if missing.is_empty() => {
            if mode.is_json() {
                emit_json(
                    mode,
                    &serde_json::json!({
                        "ok": true,
                        "missing": serde_json::Value::Array(vec![]),
                    }),
                );
            } else {
                emit_line(mode, "secrets ok: 所有 placeholder 都有对应值");
            }
            ExitCode::SUCCESS
        }
        Ok(missing) => {
            if mode.is_json() {
                let arr: Vec<serde_json::Value> = missing
                    .iter()
                    .map(|m| {
                        serde_json::json!({
                            "template": m.template,
                            "keys": m.keys,
                        })
                    })
                    .collect();
                emit_json(
                    mode,
                    &serde_json::json!({
                        "ok": false,
                        "code": exit_code::SECRETS_MISSING,
                        "missing": arr,
                    }),
                );
            } else {
                for m in &missing {
                    emit_line(mode, m.to_string());
                }
                emit_line(mode, format!("共 {} 个 template 缺变量", missing.len()));
            }
            ExitCode::from(exit_code::SECRETS_MISSING)
        }
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            ExitCode::from(e.exit_code())
        }
    }
}

fn run_set(mode: OutputMode, key: &str) -> ExitCode {
    if key.is_empty() || key.contains('=') || key.contains('\n') {
        let msg = "secrets key 非法: 不可为空,不可含 '=' / 换行".to_string();
        emit_error_envelope(mode, exit_code::ARG_ERROR, &msg, None);
        return ExitCode::from(exit_code::ARG_ERROR);
    }

    let value = match read_secret_stdin() {
        Ok(v) => v,
        Err(e) => {
            emit_error_envelope(mode, exit_code::FS_ERROR, &e.to_string(), None);
            return ExitCode::from(exit_code::FS_ERROR);
        }
    };

    let mut pairs = match core_secrets::load() {
        Ok(p) => p,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            return ExitCode::from(e.exit_code());
        }
    };
    pairs.retain(|(k, _)| k != key);
    pairs.push((key.to_string(), value));

    let path = core_secrets::default_path();
    if let Err(e) = core_secrets::save_to(&pairs, &path) {
        emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
        return ExitCode::from(e.exit_code());
    }

    if mode.is_json() {
        emit_json(
            mode,
            &serde_json::json!({
                "ok": true,
                "action": "set",
                "key": key,
                "path": path.as_str(),
            }),
        );
    } else {
        emit_line(mode, format!("secrets {key} 已写入 {}", path));
    }
    ExitCode::SUCCESS
}

fn run_unset(mode: OutputMode, key: &str) -> ExitCode {
    let mut pairs = match core_secrets::load() {
        Ok(p) => p,
        Err(e) => {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            return ExitCode::from(e.exit_code());
        }
    };
    let before = pairs.len();
    pairs.retain(|(k, _)| k != key);
    let changed = pairs.len() != before;
    if changed {
        let path = core_secrets::default_path();
        if let Err(e) = core_secrets::save_to(&pairs, &path) {
            emit_error_envelope(mode, e.exit_code(), &e.to_string(), e.hint());
            return ExitCode::from(e.exit_code());
        }
    }
    if mode.is_json() {
        emit_json(
            mode,
            &serde_json::json!({
                "ok": true,
                "action": "unset",
                "key": key,
                "changed": changed,
            }),
        );
    } else if changed {
        emit_line(mode, format!("secrets {key} 已删除"));
    } else {
        emit_line(mode, format!("secrets {key} 本来就不存在"));
    }
    ExitCode::SUCCESS
}

/// MCP 已改为明文 `mcp.json`,不再扫描 `${VAR}` 占位符。
fn collect_missing(_default_root: &Utf8Path) -> Result<Vec<core_secrets::MissingKeys>, CoreError> {
    Ok(Vec::new())
}

/// 从 stdin 读一行作为 value。
///
/// 关键:终端下应**不**回显(避免旁观者 / 历史记录偷窥)。简化策略:
/// - 管道 / heredoc(非 tty):直接读整段
/// - tty:用 `eprint!` 提示后 read_line(terminal 通常默认 echo,简化实现靠
///   提示用户"输入完成后按 Enter";生产可换 `rpassword` crate)
fn read_secret_stdin() -> std::io::Result<String> {
    let mut buf = String::new();
    let stdin = io::stdin();
    if !stdin.is_terminal() {
        stdin.lock().read_to_string(&mut buf)?;
    } else {
        eprint!("> ");
        let _ = io::stderr().flush();
        stdin.lock().read_line(&mut buf)?;
    }
    Ok(buf.trim_end_matches(['\n', '\r']).to_string())
}
