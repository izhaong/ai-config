//! agent-manager 命令行入口(对齐 plan §2.3.1)。
//!
//! Phase 1 实装 11 个顶层子命令:install / uninstall / sync / status / list / show /
//! doctor / secrets / skill / rule / mcp / daemon / gui / completion。
//!
//! Phase 0 占位:`--help` 与 `--version` 可用,业务子命令以 "not yet implemented" 退出。

use std::{
    io::{self, Write},
    process::ExitCode,
};

use ai_config_core::paths;
use camino::{Utf8Path, Utf8PathBuf};
use clap::{Parser, Subcommand};

mod agent_api;
mod asset;
mod daemon;
mod lifecycle;
mod mcp;
mod migration;
mod output;
mod projection;
mod secrets;
mod serve;

use output::OutputMode;

/// agent-manager — 写一套 skills / rules / mcp / agents,在 4 个 AI 编码 IDE 同步分发
/// (PRD v1.0 一句话定位)
#[derive(Debug, Parser)]
#[command(name = "agent-manager", version, about, long_about = None)]
struct Cli {
    /// 人类可读 / 机器可解析切换(PRD §9.1)
    #[arg(long, global = true)]
    json: bool,

    /// 静默:只输完成/失败一行
    #[arg(long, global = true)]
    quiet: bool,

    /// 资产根目录(可被 --root / AI_CONFIG_ROOT / CWD 覆盖)
    #[arg(long, global = true, value_name = "PATH")]
    root: Option<String>,

    /// 聚合仓模式：对 `.gitmodules` 父仓及已 checkout 子模块逐个 install/sync
    #[arg(long, global = true)]
    workspace: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// 一键安装(取代旧 install.sh,PRD §5 场景 A)
    Install {
        /// Render the reviewed projection plan transactionally.
        #[arg(long)]
        apply: bool,
    },
    /// 卸载(保留用户手写配置,备份 mcp.json)
    Uninstall {
        /// Apply the reviewed retract plan.
        #[arg(long)]
        apply: bool,
    },
    /// 手工同步一次(守护进程未跑时使用,PRD §10 A-5)
    Sync {
        /// Render the reviewed projection plan transactionally.
        #[arg(long)]
        apply: bool,
    },
    /// 当前同步状态
    Status,
    /// 列出已纳管资产
    List,
    /// 显示单条资产详情
    Show { name: String },
    /// 健康检查(输出 `--json` 可被 agent 解析,PRD §10 A-10)
    Doctor {
        /// 将平台目录中 symlink / 同 inode 的旧下发迁移为实体硬拷贝
        #[arg(long)]
        materialize: bool,
    },

    /// secrets 维度(PRD §5.1 必做)
    Secrets {
        #[command(subcommand)]
        action: SecretsCmd,
    },
    /// skill 维度
    Skill {
        #[command(subcommand)]
        action: AssetCmd,
    },
    /// rule 维度
    Rule {
        #[command(subcommand)]
        action: AssetCmd,
    },
    /// agent 维度(目录或单文件 `.md` / `.yaml` / `.json`)
    Agent {
        #[command(subcommand)]
        action: AssetCmd,
    },
    /// command 维度(Cursor / Claude 斜杠命令 `.md`)
    Command {
        #[command(subcommand)]
        action: AssetCmd,
    },
    /// mcp 维度(逐项 + 单条 deploy/retract)
    Mcp {
        #[command(subcommand)]
        action: McpCmd,
    },

    /// Explicitly import one platform asset into the canonical source layer.
    Import {
        /// Asset kind: skill, rule, mcp, agent, command, or prompt.
        kind: String,
        /// Stable canonical asset name (prompt currently accepts AGENTS only).
        name: String,
        /// Platform that currently owns the asset.
        #[arg(long)]
        from: String,
        /// Canonical destination layer.
        #[arg(long)]
        to: String,
        /// Replace a different existing canonical asset after review.
        #[arg(long)]
        replace: bool,
        /// Reviewed JSON plan emitted by the same import command.
        #[arg(long)]
        plan: Option<String>,
        /// Execute the one reviewed import. Omit for a read-only plan.
        #[arg(long)]
        apply: bool,
    },

    /// Source-first migration tools; inventory is strictly read-only.
    Migrate {
        #[command(subcommand)]
        action: MigrateCmd,
    },

    /// 守护进程子命令
    Daemon {
        #[command(subcommand)]
        action: DaemonCmd,
    },
    /// 启动 Tauri 窗口
    Gui,

    /// shell 补全脚本
    Completion { shell: String },

    /// 启动 MCP stdio 服务器，供 IDE Agent 外部控制资产
    Serve,
}

#[derive(Debug, Subcommand)]
enum SecretsCmd {
    Path,
    Validate,
    Set { key: String },
    Unset { key: String },
    List,
}

#[derive(Debug, Subcommand)]
enum AssetCmd {
    List,
    Show { name: String },
    Reveal { name: String },
}

#[derive(Debug, Subcommand)]
enum McpCmd {
    List,
    Show {
        name: String,
    },
    /// 从已校验的单 server JSON 创建 canonical source（不触发平台写入）
    Add {
        /// 单 server MCP 定义 JSON；会写入 <root>/mcp/servers/<name>.json
        source: String,
    },
    Remove {
        name: String,
    },
    Enable {
        name: String,
    },
    Disable {
        name: String,
    },
    /// 单条粒度下发到某平台(PRD §4.2 + §10 A-15)
    Deploy {
        name: String,
        to: String,
    },
    /// 单条粒度从某平台收回(PRD §4.2 + §10 A-15)
    Retract {
        name: String,
        from: String,
    },
    /// 整份 mcp.json 模板 → 逐项 `mcp/servers/<name>.json`(W5 残留)
    ///
    /// 默认 source = `mcp/cursor.mcp.template.json`,已存在文件 skip(不覆盖)。
    /// `--dry-run` 跑完不写盘,0 副作用。
    Migrate {
        /// 模板路径(相对 root);默认 `mcp/cursor.mcp.template.json`
        source: Option<String>,
        /// 只输出报告,不真写盘
        #[arg(long)]
        dry_run: bool,
        /// 抽取旧容器中的 literal env/header 到 secret store（尚未接通时明确拒绝）
        #[arg(long)]
        extract_secrets: bool,
        /// 允许 migration 写 source/secret store（尚未接通时明确拒绝）
        #[arg(long)]
        apply: bool,
    },
    /// 遗留 `~/.hermes/mcp.json` → `~/.hermes/config.yaml` 的 `mcp_servers`
    MigrateHermes {
        /// 只输出报告,不真写盘
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Subcommand)]
enum MigrateCmd {
    /// Inventory legacy/current skill locations without changing the filesystem.
    Inventory,
    /// Build the reviewed source-first adoption plan without changing the filesystem.
    Plan,
    /// Adopt explicitly selected equivalent targets from a reviewed migration plan.
    SourceFirst {
        /// JSON file emitted by `agent-manager migrate plan --json`.
        #[arg(long)]
        plan: String,
        /// Stable action ID to adopt. May be repeated; only AdoptEquivalent actions are valid.
        #[arg(long = "select")]
        select: Vec<String>,
        /// Execute the selected reviewed adoptions. Omit for a read-only verification.
        #[arg(long)]
        apply: bool,
    },
    /// Restore a completed explicit import when its canonical result has not drifted.
    Rollback {
        /// Transaction ID returned by `agent-manager import --apply`.
        transaction_id: String,
    },
}

#[derive(Debug, Subcommand)]
enum DaemonCmd {
    Start,
    Stop,
    Status,
    Logs,
    /// 实时事件流(agent 订阅,PRD §5.1)
    EventsFollow,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if matches!(cli.cmd, Cmd::Serve) {
        let default_root = resolve_root(cli.root.as_deref());
        return match serve::run(default_root) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("agent-manager serve: {e}");
                ExitCode::from(5)
            }
        };
    }

    let mode = OutputMode::from_flags(cli.json, cli.quiet);

    if !mode.is_quiet() && !mode.is_json() {
        eprintln!(
            "agent-manager v{} (Phase 1 业务子命令:install/uninstall/sync/status/list/show/doctor/secrets/skill/rule/mcp/daemon)",
            env!("CARGO_PKG_VERSION")
        );
    }

    // 解析 default_root
    let default_root = resolve_root(cli.root.as_deref());

    match cli.cmd {
        Cmd::Gui => {
            // Phase 3 实装:启动 Tauri 应用
            if !mode.is_quiet() && !mode.is_json() {
                eprintln!("gui: 启动 Tauri 窗口 — Phase 3 实装");
            }
            ExitCode::from(2)
        }
        Cmd::Completion { shell } => {
            let shell = match shell.as_str() {
                "bash" => clap_complete::Shell::Bash,
                "zsh" => clap_complete::Shell::Zsh,
                "fish" => clap_complete::Shell::Fish,
                "powershell" | "pwsh" => clap_complete::Shell::PowerShell,
                "elvish" => clap_complete::Shell::Elvish,
                _ => {
                    eprintln!("unknown shell: {shell}");
                    return ExitCode::from(2);
                }
            };
            let stdout = io::stdout();
            let mut stdout = stdout.lock();
            match emit_completion(shell, &mut stdout) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("failed to write completion: {error}");
                    ExitCode::from(5)
                }
            }
        }
        Cmd::Secrets { action } => {
            let mapped = match action {
                SecretsCmd::Path => secrets::SecretsCmd::Path,
                SecretsCmd::Validate => secrets::SecretsCmd::Validate,
                SecretsCmd::Set { key } => secrets::SecretsCmd::Set { key },
                SecretsCmd::Unset { key } => secrets::SecretsCmd::Unset { key },
                SecretsCmd::List => secrets::SecretsCmd::List,
            };
            mapped.run(mode, &default_root)
        }
        Cmd::Skill { action } => {
            let mapped = match action {
                AssetCmd::List => asset::AssetCmd::List,
                AssetCmd::Show { name } => asset::AssetCmd::Show { name },
                AssetCmd::Reveal { name } => asset::AssetCmd::Reveal { name },
            };
            mapped.run(mode, &default_root, asset::AssetKind::Skill)
        }
        Cmd::Rule { action } => {
            let mapped = match action {
                AssetCmd::List => asset::AssetCmd::List,
                AssetCmd::Show { name } => asset::AssetCmd::Show { name },
                AssetCmd::Reveal { name } => asset::AssetCmd::Reveal { name },
            };
            mapped.run(mode, &default_root, asset::AssetKind::Rule)
        }
        Cmd::Agent { action } => {
            let mapped = match action {
                AssetCmd::List => asset::AssetCmd::List,
                AssetCmd::Show { name } => asset::AssetCmd::Show { name },
                AssetCmd::Reveal { name } => asset::AssetCmd::Reveal { name },
            };
            mapped.run(mode, &default_root, asset::AssetKind::Agent)
        }
        Cmd::Command { action } => {
            let mapped = match action {
                AssetCmd::List => asset::AssetCmd::List,
                AssetCmd::Show { name } => asset::AssetCmd::Show { name },
                AssetCmd::Reveal { name } => asset::AssetCmd::Reveal { name },
            };
            mapped.run(mode, &default_root, asset::AssetKind::Command)
        }
        Cmd::Mcp { action } => {
            let mapped = match action {
                McpCmd::List => mcp::McpCmd::List,
                McpCmd::Show { name } => mcp::McpCmd::Show { name },
                McpCmd::Add { source } => mcp::McpCmd::Add { source },
                McpCmd::Remove { name } => mcp::McpCmd::Remove { name },
                McpCmd::Enable { name } => mcp::McpCmd::Enable { name },
                McpCmd::Disable { name } => mcp::McpCmd::Disable { name },
                McpCmd::Deploy { name, to } => mcp::McpCmd::Deploy { name, to },
                McpCmd::Retract { name, from } => mcp::McpCmd::Retract { name, from },
                McpCmd::Migrate {
                    source,
                    dry_run,
                    extract_secrets,
                    apply,
                } => mcp::McpCmd::Migrate {
                    source,
                    dry_run,
                    extract_secrets,
                    apply,
                },
                McpCmd::MigrateHermes { dry_run } => mcp::McpCmd::MigrateHermes { dry_run },
            };
            mapped.run(mode, &default_root)
        }
        Cmd::Import {
            kind,
            name,
            from,
            to,
            replace,
            plan,
            apply,
        } => migration::run_import(
            mode,
            &default_root,
            &kind,
            &name,
            &from,
            &to,
            replace,
            plan.as_deref(),
            apply,
        ),
        Cmd::Migrate { action } => match action {
            MigrateCmd::Inventory => migration::run_inventory(mode, &default_root, cli.workspace),
            MigrateCmd::Plan => migration::run_plan(mode, &default_root, cli.workspace),
            MigrateCmd::SourceFirst {
                plan,
                select,
                apply,
            } => migration::run_source_first(
                mode,
                &default_root,
                cli.workspace,
                &plan,
                select,
                apply,
            ),
            MigrateCmd::Rollback { transaction_id } => {
                migration::run_rollback(mode, &default_root, &transaction_id)
            }
        },
        Cmd::Daemon { action } => {
            // 桥接 main::DaemonCmd → daemon 模块的 DaemonCmd(传入 --json 全局标志)
            let mapped = match action {
                DaemonCmd::Start => daemon::DaemonCmd::Start,
                DaemonCmd::Stop => daemon::DaemonCmd::Stop,
                DaemonCmd::Status => daemon::DaemonCmd::Status,
                DaemonCmd::Logs => daemon::DaemonCmd::Logs,
                DaemonCmd::EventsFollow => {
                    daemon::DaemonCmd::EventsFollow(daemon::EventsFollowOpts { json: cli.json })
                }
            };
            daemon::run(mapped, mode)
        }
        Cmd::Install { apply } => projection::run(&default_root, cli.workspace, apply, false, mode),
        Cmd::Uninstall { apply } => {
            projection::run(&default_root, cli.workspace, apply, true, mode)
        }
        Cmd::Sync { apply } => projection::run(&default_root, cli.workspace, apply, false, mode),
        Cmd::Status => lifecycle::run_status(&default_root, mode),
        Cmd::List => lifecycle::run_list(&default_root, mode),
        Cmd::Show { name } => lifecycle::run_show(&default_root, &name, mode),
        Cmd::Doctor { materialize } => lifecycle::run_doctor(&default_root, mode, materialize),
        Cmd::Serve => unreachable!("Serve handled before match"),
    }
}

/// 解析资产根目录:优先级 `--root` > `AI_CONFIG_ROOT` > `~/.agent-manager`。
/// 命令入口只解析路径；初始化/播种必须由显式写操作负责。
fn resolve_root(flag: Option<&str>) -> Utf8PathBuf {
    let raw = match flag {
        Some(s) => s.to_string(),
        None => std::env::var("AI_CONFIG_ROOT").unwrap_or_else(|_| String::new()),
    };
    if raw.is_empty() {
        return paths::discover_global_asset_root_read_only();
    }
    paths::resolve_asset_root(Utf8Path::new(&raw))
}

fn emit_completion<W: Write>(shell: clap_complete::Shell, writer: &mut W) -> io::Result<()> {
    let mut generated = Vec::new();
    clap_complete::generate(shell, &mut Cli::command(), "agent-manager", &mut generated);
    match writer.write_all(&generated) {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result,
    }
}

// re-export 给 clap_complete
use clap::CommandFactory;

#[cfg(test)]
mod completion_tests {
    use std::io::{self, Write};

    struct FailingWriter(io::ErrorKind);

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(self.0))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn completion_broken_pipe_is_success() {
        let mut writer = FailingWriter(io::ErrorKind::BrokenPipe);

        super::emit_completion(clap_complete::Shell::Zsh, &mut writer)
            .expect("an early-closing completion consumer is not a CLI failure");
    }

    #[test]
    fn completion_other_io_error_fails() {
        let mut writer = FailingWriter(io::ErrorKind::PermissionDenied);

        let error = super::emit_completion(clap_complete::Shell::Zsh, &mut writer)
            .expect_err("non-broken-pipe output errors must remain visible");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }
}
