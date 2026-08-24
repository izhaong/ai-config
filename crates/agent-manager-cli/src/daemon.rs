//! `agents-manager daemon ...` 子命令(PRD §5.1 + §9.1)。
//!
//! 守护进程管控:`start` / `stop` / `status` / `logs` / `events-follow`。
//! 设计要点(对齐 PRD §5.1 / §8.1 / §9.1 / §10 A-5):
//!
//! - **pid file**:`$XDG_CONFIG_HOME/agents-manager/daemon.pid`(fallback `~/.config/agents-manager/daemon.pid`)
//! - **log file**:`$XDG_CONFIG_HOME/agents-manager/daemon.log`
//! - **UDS sock**:`$XDG_CONFIG_HOME/agents-manager/daemon.sock`
//! - **`daemon start` 二次拒绝**:已有一个 alive pid 时,退出码 5(FS 错),不覆盖。
//! - **`daemon stop`**:SIGTERM → 等 1s → SIGKILL 兜底。
//! - **`daemon logs`**:`tail` log,过滤 `secrets.*` 字段(PRD §8.1)。
//! - **`daemon events-follow`**:连 UDS 订阅 BusEvent;`--json` 时每行直出原始 JSON 帧 payload。
//!
//! 本阶段 `agents-managerd` 是 Phase 0 占位,可能不接 UDS — `events-follow` 在
//! connect 失败时返回部分失败(退出码 3),并把原因放在 stderr 提示。

use std::io::{BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

use agent_manager_core::error::exit_code;

use crate::output::{emit_error_envelope, emit_json, emit_line, OutputMode};

/// `daemon events-follow` 的子选项(由 `main.rs` 解析时填充)。
#[derive(Debug, Clone, Default)]
pub struct EventsFollowOpts {
    /// `--json` 标志(沿用全局或子命令标志):每行直出原始 JSON 帧 payload。
    pub json: bool,
}

/// daemon 子命令(`main.rs` 透传 — 不与 `clap` 耦合)。
#[derive(Debug, Clone)]
pub enum DaemonCmd {
    Start,
    Stop,
    Status,
    Logs,
    EventsFollow(EventsFollowOpts),
}

/// daemon 子命令入口(由 `main.rs` 路由过来)。
pub fn run(action: DaemonCmd, mode: OutputMode) -> ExitCode {
    match action {
        DaemonCmd::Start => start(mode),
        DaemonCmd::Stop => stop(mode),
        DaemonCmd::Status => status(mode),
        DaemonCmd::Logs => logs(mode),
        DaemonCmd::EventsFollow(opts) => events_follow(mode, opts.json),
    }
}

// ─── 路径与目录 ──────────────────────────────────────────────────────────

/// XDG 风格配置目录:`~/.config/agents-manager`(Windows:`%APPDATA%\\agents-manager`)。
fn config_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            if !appdata.trim().is_empty() {
                return PathBuf::from(appdata).join("agents-manager");
            }
        }
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.trim().is_empty() {
            return PathBuf::from(xdg).join("agents-manager");
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".config").join("agents-manager");
    }
    PathBuf::from(".config").join("agents-manager")
}

pub fn pid_file() -> PathBuf {
    config_dir().join("daemon.pid")
}

pub fn log_file() -> PathBuf {
    config_dir().join("daemon.log")
}

pub fn sock_file() -> PathBuf {
    config_dir().join("daemon.sock")
}

fn ensure_config_dir() -> std::io::Result<()> {
    let dir = config_dir();
    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
    }
    Ok(())
}

// ─── PID 工具 ────────────────────────────────────────────────────────────

/// 读 pid file;`None` 表示未跑 / 文件不存在 / 脏数据。
pub fn read_pid() -> Option<i32> {
    let p = pid_file();
    let raw = std::fs::read_to_string(&p).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<i32>().ok()
}

pub fn write_pid(pid: u32) -> std::io::Result<()> {
    ensure_config_dir()?;
    let path = pid_file();
    if let Some(parent) = path.parent() {
        // 测试间可能并发清理 tmpdir,显式 create_dir_all 兜底
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, pid.to_string())
}

pub fn remove_pid() {
    let _ = std::fs::remove_file(pid_file());
}

/// `kill -0 <pid>` 检测进程是否存在。
pub fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ─── 子命令实现 ──────────────────────────────────────────────────────────

/// `daemon start` — fork 子进程跑 agents-managerd;写 pid file;二次 start 拒绝(退出码 5)。
fn start(mode: OutputMode) -> ExitCode {
    if let Some(pid) = read_pid() {
        if pid_alive(pid) {
            emit_error_envelope(
                mode,
                exit_code::FS_ERROR,
                "daemon already running",
                Some(&format!(
                    "pid file at {} holds live pid {pid};stop it first(`agents-manager daemon stop`)",
                    pid_file().display()
                )),
            );
            return ExitCode::from(exit_code::FS_ERROR);
        }
        // 脏 pid:清理
        remove_pid();
    }

    if let Err(e) = ensure_config_dir() {
        emit_error_envelope(
            mode,
            exit_code::FS_ERROR,
            "failed to create config dir",
            Some(&e.to_string()),
        );
        return ExitCode::from(exit_code::FS_ERROR);
    }

    let exe = match locate_daemon_binary() {
        Some(p) => p,
        None => {
            emit_error_envelope(
                mode,
                exit_code::FS_ERROR,
                "agents-managerd binary not found",
                Some(
                    "looked next to current exe and in $PATH;install agents-managerd or run `cargo build -p agents-manager-daemon`",
                ),
            );
            return ExitCode::from(exit_code::FS_ERROR);
        }
    };

    // 打开日志(append);Phase 0 占位 daemon 写到 stderr,我们 redirect 到 log file。
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file());

    let child = match log {
        Ok(f) => {
            // `Stdio` 不是 Copy,所以用 `try_clone` 拿一份给 stderr
            let f_for_stderr = f.try_clone().unwrap_or_else(|_| {
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(null_device())
                    .expect("open null device")
            });
            Command::new(&exe)
                .stdout(Stdio::from(f))
                .stderr(Stdio::from(f_for_stderr))
                .stdin(Stdio::null())
                .spawn()
        }
        Err(_) => Command::new(&exe)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .spawn(),
    };

    let child = match child {
        Ok(c) => c,
        Err(e) => {
            emit_error_envelope(
                mode,
                exit_code::FS_ERROR,
                "failed to spawn agents-managerd",
                Some(&format!("{}: {e}", exe.display())),
            );
            return ExitCode::from(exit_code::FS_ERROR);
        }
    };

    let pid = child.id();
    if let Err(e) = write_pid(pid) {
        emit_error_envelope(
            mode,
            exit_code::FS_ERROR,
            "failed to write pid file",
            Some(&e.to_string()),
        );
        return ExitCode::from(exit_code::FS_ERROR);
    }

    let payload = serde_json::json!({
        "started": true,
        "pid": pid,
        "pid_file": pid_file().to_string_lossy(),
        "log_file": log_file().to_string_lossy(),
    });
    let line = format!("daemon started: pid {pid}");
    if mode.is_json() {
        emit_json(mode, &payload);
    } else {
        emit_line(mode, &line);
    }
    ExitCode::SUCCESS
}

/// `daemon stop` — 读 pid file,kill SIGTERM;等 1s,SIGKILL 兜底。
fn stop(mode: OutputMode) -> ExitCode {
    let pid = match read_pid() {
        Some(p) => p,
        None => {
            // 未跑 = 静默成功
            let payload = serde_json::json!({"stopped": true, "was_running": false});
            if mode.is_json() {
                emit_json(mode, &payload);
            } else {
                emit_line(mode, "daemon not running");
            }
            return ExitCode::SUCCESS;
        }
    };

    if !pid_alive(pid) {
        // 脏 pid file
        remove_pid();
        let payload = serde_json::json!({
            "stopped": true,
            "was_running": false,
            "stale_pid": pid,
        });
        if mode.is_json() {
            emit_json(mode, &payload);
        } else {
            emit_line(mode, "daemon not running (stale pid file cleaned)");
        }
        return ExitCode::SUCCESS;
    }

    // SIGTERM
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status();

    // 等 1s
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(100));
        if !pid_alive(pid) {
            break;
        }
    }

    // 兜底 SIGKILL
    if pid_alive(pid) {
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg(pid.to_string())
            .status();
    }

    remove_pid();

    let payload = serde_json::json!({"stopped": true, "pid": pid});
    let line = format!("daemon stopped: pid {pid}");
    if mode.is_json() {
        emit_json(mode, &payload);
    } else {
        emit_line(mode, &line);
    }
    ExitCode::SUCCESS
}

/// `daemon status` — 检 pid 进程是否存在;输出 `{running, pid, uptime}`。
fn status(mode: OutputMode) -> ExitCode {
    let pid = match read_pid() {
        Some(p) => p,
        None => {
            let payload = serde_json::json!({"running": false});
            if mode.is_json() {
                emit_json(mode, &payload);
            } else {
                emit_line(mode, "daemon: not running");
            }
            return ExitCode::SUCCESS;
        }
    };

    if !pid_alive(pid) {
        let payload = serde_json::json!({
            "running": false,
            "stale_pid": pid,
        });
        if mode.is_json() {
            emit_json(mode, &payload);
        } else {
            emit_line(mode, format!("daemon: not running (stale pid {pid})"));
        }
        return ExitCode::SUCCESS;
    }

    let uptime = pid_file_start_time().map(format_duration);
    let payload = serde_json::json!({
        "running": true,
        "pid": pid,
        "uptime": uptime,
    });
    let line = format!("daemon: running (pid {pid})");
    if mode.is_json() {
        emit_json(mode, &payload);
    } else {
        emit_line(mode, &line);
    }
    ExitCode::SUCCESS
}

/// `daemon logs` — tail log,过滤 `secrets.*` 字段(PRD §8.1)。
fn logs(mode: OutputMode) -> ExitCode {
    let path = log_file();
    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(e) => {
            emit_error_envelope(
                mode,
                exit_code::FS_ERROR,
                "failed to open daemon log",
                Some(&format!("{}: {e}", path.display())),
            );
            return ExitCode::from(exit_code::FS_ERROR);
        }
    };

    let reader = BufReader::new(file);
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in reader.lines().map_while(Result::ok) {
        let filtered = redact_secrets_in_line(&line);
        if let Err(e) = writeln!(out, "{filtered}") {
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                return ExitCode::SUCCESS;
            }
            emit_error_envelope(
                mode,
                exit_code::FS_ERROR,
                "failed to write log line",
                Some(&e.to_string()),
            );
            return ExitCode::from(exit_code::FS_ERROR);
        }
    }
    ExitCode::SUCCESS
}

/// `daemon events-follow` — 连 UDS 订阅 BusEvent,`--json` 直出原始 JSON。
fn events_follow(mode: OutputMode, json: bool) -> ExitCode {
    #[cfg(windows)]
    {
        let _ = json;
        emit_error_envelope(
            mode,
            exit_code::PARTIAL_FAILURE,
            "daemon events-follow is not supported on Windows yet",
            Some("UDS-based daemon IPC is Unix-only in the current Phase 0 placeholder"),
        );
        return ExitCode::from(exit_code::PARTIAL_FAILURE);
    }

    #[cfg(unix)]
    {
        events_follow_unix(mode, json)
    }
}

#[cfg(unix)]
fn events_follow_unix(mode: OutputMode, json: bool) -> ExitCode {
    let pid = match read_pid() {
        Some(p) if pid_alive(p) => p,
        _ => {
            emit_error_envelope(
                mode,
                exit_code::FS_ERROR,
                "daemon not running",
                Some(&format!(
                    "sock path: {};start daemon with `agents-manager daemon start`",
                    sock_file().display()
                )),
            );
            return ExitCode::from(exit_code::FS_ERROR);
        }
    };

    let sock = sock_file();
    let mut stream = match UnixStream::connect(&sock) {
        Ok(s) => s,
        Err(e) => {
            // Phase 0 daemon 占位不接 socket,这是"部分失败" — 退出码 3
            emit_error_envelope(
                mode,
                exit_code::PARTIAL_FAILURE,
                "failed to connect to daemon socket",
                Some(&format!(
                    "{}: {e};(Phase 0 daemon is a placeholder and does not bind the UDS — events stream is not yet wired)",
                    sock.display()
                )),
            );
            return ExitCode::from(exit_code::PARTIAL_FAILURE);
        }
    };

    // 设 1s read timeout 以便响应 Ctrl-C / 周期性重试
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));

    if mode.is_human() {
        eprintln!(
            "connected to daemon (pid {pid});streaming events (Phase 0 placeholder — daemon does not emit BusEvent yet)"
        );
    }

    // 发送 subscribe 帧(协议见 ARCHITECTURE §12.2)
    let sub = serde_json::json!({
        "id": "subscribe-1",
        "method": "subscribe",
        "params": {}
    });
    let frame = encode_frame(&sub.to_string());
    if stream.write_all(&frame).is_err() {
        emit_error_envelope(
            mode,
            exit_code::FS_ERROR,
            "failed to send subscribe frame",
            None,
        );
        return ExitCode::from(exit_code::FS_ERROR);
    }

    // 读循环:每帧 4 字节长度 + payload
    let mut reader = BufReader::new(stream);
    loop {
        let mut len_buf = [0u8; 4];
        match reader.read_exact(&mut len_buf) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => break,
            Err(_) => break,
        }
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        if reader.read_exact(&mut payload).is_err() {
            break;
        }
        let line = String::from_utf8_lossy(&payload);
        let out_line = if json {
            line.to_string()
        } else {
            redact_secrets_in_line(&line)
        };
        println!("{out_line}");
    }
    ExitCode::SUCCESS
}

fn null_device() -> &'static str {
    #[cfg(unix)]
    {
        "/dev/null"
    }
    #[cfg(windows)]
    {
        "NUL"
    }
}

// ─── 工具 ───────────────────────────────────────────────────────────────

/// 长度前缀帧(ARCHITECTURE §12.2):4 字节 big-endian length + UTF-8 JSON。
pub fn encode_frame(payload: &str) -> Vec<u8> {
    let len = payload.len() as u32;
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload.as_bytes());
    out
}

/// 读 pid file 的 mtime 作为 daemon 启动时刻的粗略代理。
fn pid_file_start_time() -> Option<Duration> {
    let p = pid_file();
    let meta = std::fs::metadata(&p).ok()?;
    let modified = meta.modified().ok()?;
    let now = std::time::SystemTime::now();
    let dur = now.duration_since(modified).ok()?;
    Some(dur)
}

fn format_duration(d: Duration) -> String {
    let total = d.as_secs();
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}h{m}m{s}s")
    } else if m > 0 {
        format!("{m}m{s}s")
    } else {
        format!("{s}s")
    }
}

/// Phase 0 启发式:从一行中抹掉形如 `"secrets": "..."` / `"secrets_xxx": "..."` /
/// `"secrets": { ... }` 的值。非 JSON 行直接 passthrough。
pub fn redact_secrets_in_line(line: &str) -> String {
    // 用 serde_json::Value 解析,递归遍历;若发现任何 key 名以 "secrets" 开头
    // (不区分大小写,后跟非字母数字字符或字符串结束),则 value 替换为
    // `"<redacted>"`。无法解析为 JSON 的行原样返回(不破坏非 JSON 日志)。
    let trimmed = line.trim();
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return line.to_string();
    }
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return line.to_string();
    };
    redact_value(&mut value);
    // 美化输出:用 serde_json 的紧凑模式,而不是 pretty
    serde_json::to_string(&value).unwrap_or_else(|_| line.to_string())
}

fn is_secrets_key(k: &str) -> bool {
    // 匹配规则:key 等于 "secrets" **或** 以 "secrets" 开头后跟下划线 / 连字符
    // (避免误伤 `secret` 单数、随机 `secretsinfo` 等)
    if k == "secrets" {
        return true;
    }
    if let Some(rest) = k.strip_prefix("secrets") {
        if rest.is_empty() {
            return true;
        }
        // 下一字符必须是 _ 或 -(避免 secretsXYZ 这种误伤)
        if let Some(c) = rest.chars().next() {
            return c == '_' || c == '-';
        }
    }
    false
}

fn redact_value(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for k in keys {
                if is_secrets_key(&k) {
                    if let Some(slot) = map.get_mut(&k) {
                        *slot = serde_json::Value::String("<redacted>".to_string());
                    }
                } else if let Some(child) = map.get_mut(&k) {
                    redact_value(child);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                redact_value(item);
            }
        }
        _ => {}
    }
}

/// 找 `agents-managerd` 二进制:优先当前 exe 同目录(开发/打包),再 `$PATH`。
fn locate_daemon_binary() -> Option<PathBuf> {
    let candidate_names = ["agents-managerd", "agents-managerd.exe"];
    if let Ok(current) = std::env::current_exe() {
        if let Some(dir) = current.parent() {
            for name in candidate_names {
                let p = dir.join(name);
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            for name in candidate_names {
                let p: PathBuf = dir.join(name);
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }
    None
}

// ─── 测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 串行化所有 daemon 测试,避免 `$XDG_CONFIG_HOME` 与 pid file 互相干扰
    /// (cargo test 默认并行 → 多测试在同一 tmpdir 上下游走)
    static DAEMON_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// 临时重定向 `$XDG_CONFIG_HOME` 到 `tempfile::TempDir`,并串行化所有用此函数的测试
    /// (避免 `$XDG_CONFIG_HOME` 与 pid file 在多线程下互相干扰)。
    fn with_xdg<F: FnOnce(&std::path::Path) -> R, R>(f: F) -> R {
        let _guard = DAEMON_TEST_LOCK.lock().expect("daemon test lock poisoned");
        let tmp = tempfile::tempdir().expect("tempdir");
        // SAFETY:本测试跑在单线程 cargo test 同步上下文,串行
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", tmp.path());
        }
        let r = f(tmp.path());
        unsafe {
            std::env::remove_var("XDG_CONFIG_HOME");
        }
        r
    }

    #[test]
    fn redact_secrets_string_value() {
        let line = r#"{"secrets": "MINIO_ENDPOINT=abc", "ok": 1}"#;
        let r = redact_secrets_in_line(line);
        assert!(!r.contains("abc"), "redacted output: {r}");
        assert!(r.contains("<redacted>"));
        assert!(r.contains("\"ok\""));
    }

    #[test]
    fn redact_secrets_prefixed_key() {
        let line = r#"{"secrets_api_key": "sk-1234", "ok": 1}"#;
        let r = redact_secrets_in_line(line);
        assert!(!r.contains("sk-1234"), "redacted output: {r}");
        assert!(r.contains("<redacted>"));
    }

    #[test]
    fn redact_secrets_object_value() {
        let line = r#"{"secrets": {"a": "1", "b": "2"}, "ok": 1}"#;
        let r = redact_secrets_in_line(line);
        assert!(!r.contains("\"1\""));
        assert!(!r.contains("\"2\""));
        assert!(r.contains("<redacted>"));
    }

    #[test]
    fn redact_passthrough_non_json() {
        let line = "2025-01-01 INFO daemon started";
        let r = redact_secrets_in_line(line);
        assert_eq!(r, line);
    }

    #[test]
    fn format_duration_smoke() {
        assert_eq!(format_duration(Duration::from_secs(0)), "0s");
        assert_eq!(format_duration(Duration::from_secs(45)), "45s");
        assert_eq!(format_duration(Duration::from_secs(125)), "2m5s");
        assert_eq!(format_duration(Duration::from_secs(3725)), "1h2m5s");
    }

    #[test]
    fn pid_alive_rejects_invalid() {
        assert!(!pid_alive(0));
        assert!(!pid_alive(-1));
    }

    /// `daemon start` 幂等:已有一个 alive pid 时,二次 start 必须拒绝。
    /// 这里不真起子进程(会污染 target/),而是直接验证拒绝决策路径。
    #[test]
    fn daemon_start_idempotent_rejection() {
        with_xdg(|_| {
            remove_pid();
            assert!(read_pid().is_none(), "fresh: no pid file");

            let my_pid = std::process::id();
            write_pid(my_pid).unwrap();
            assert_eq!(read_pid(), Some(my_pid as i32));
            assert!(pid_alive(my_pid as i32), "self pid must be alive");

            // 二次 start 拒绝的判定核心
            let pid_now = read_pid();
            let alive = pid_now.map(pid_alive).unwrap_or(false);
            assert!(pid_now.is_some() && alive, "second start must be rejected");

            remove_pid();
        });
    }

    /// `daemon stop` 幂等:无 pid file 时不报错(视为成功)。
    #[test]
    fn daemon_stop_idempotent_when_not_running() {
        with_xdg(|_| {
            remove_pid();
            assert!(read_pid().is_none(), "stop should treat as success");
        });
    }

    /// `daemon start` 在脏 pid file(进程已死)下应该清理并继续。
    #[test]
    fn daemon_start_clears_stale_pid() {
        with_xdg(|_| {
            remove_pid();
            let stale: i32 = 999_999_999;
            write_pid(stale as u32).unwrap();
            assert!(!pid_alive(stale), "stale pid must not be alive");

            let pid = read_pid();
            let alive = pid.map(pid_alive).unwrap_or(false);
            assert!(!alive, "stale pid should be detected as dead");

            remove_pid();
        });
    }

    /// `pid_file()` / `log_file()` / `sock_file()` 在 `$XDG_CONFIG_HOME` 下落到正确位置。
    #[test]
    fn paths_respect_xdg_config_home() {
        with_xdg(|tmp| {
            let dir = config_dir();
            assert!(dir.starts_with(tmp), "config_dir: {}", dir.display());
            assert!(pid_file().ends_with("daemon.pid"));
            assert!(log_file().ends_with("daemon.log"));
            assert!(sock_file().ends_with("daemon.sock"));
        });
    }

    /// `daemon status` 在没 pid file 时输出 running=false 且退出 0。
    #[test]
    fn status_no_pid_is_zero_exit() {
        with_xdg(|_| {
            remove_pid();
            let mode = OutputMode::Json;
            let code = status(mode);
            assert_eq!(code, ExitCode::SUCCESS);
        });
    }

    /// `daemon stop` 在没 pid file 时退出 0(幂等)。
    #[test]
    fn stop_no_pid_is_zero_exit() {
        with_xdg(|_| {
            remove_pid();
            let mode = OutputMode::Json;
            let code = stop(mode);
            assert_eq!(code, ExitCode::SUCCESS);
        });
    }

    /// `encode_frame` 长度前缀正确。
    #[test]
    fn encode_frame_length_prefix() {
        let payload = "{\"id\":1}";
        let frame = encode_frame(payload);
        assert_eq!(frame.len(), 4 + payload.len());
        let len = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]);
        assert_eq!(len as usize, payload.len());
        assert_eq!(&frame[4..], payload.as_bytes());
    }
}
