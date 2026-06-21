//! ai-config 命令行入口(对齐 plan §2.3.1)。
//!
//! Phase 1 实装 11 个顶层子命令:install / uninstall / sync / status / list / show /
//! doctor / secrets / skill / rule / mcp / daemon / gui / completion。
//!
//! Phase 0 占位:`--help` 与 `--version` 可用,业务子命令以 "not yet implemented" 退出。

use std::process::ExitCode;

use ai_config_core::paths;
use camino::{Utf8Path, Utf8PathBuf};
use clap::{Parser, Subcommand};

mod asset;
mod daemon;
mod lifecycle;
mod mcp;
mod output;
mod secrets;

use output::OutputMode;

/// ai-config — 写一套 skills / rules / mcp / agents,在 4 个 AI 编码 IDE 同步分发
/// (PRD v1.0 一句话定位)
#[derive(Debug, Parser)]
#[command(name = "ai-config", version, about, long_about = None)]
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

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// 一键安装(取代旧 install.sh,PRD §5 场景 A)
    Install,
    /// 卸载(保留用户手写配置,备份 mcp.json)
    Uninstall,
    /// 手工同步一次(守护进程未跑时使用,PRD §10 A-5)
    Sync,
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

    /// 守护进程子命令
    Daemon {
        #[command(subcommand)]
        action: DaemonCmd,
    },
    /// 启动 Tauri 窗口
    Gui,

    /// shell 补全脚本
    Completion { shell: String },
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
    Add,
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
    },
    /// 遗留 `~/.hermes/mcp.json` → `~/.hermes/config.yaml` 的 `mcp_servers`
    MigrateHermes {
        /// 只输出报告,不真写盘
        #[arg(long)]
        dry_run: bool,
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
    let mode = OutputMode::from_flags(cli.json, cli.quiet);

    if !mode.is_quiet() && !mode.is_json() {
        eprintln!(
            "ai-config v{} (Phase 1 业务子命令:install/uninstall/sync/status/list/show/doctor/secrets/skill/rule/mcp/daemon)",
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
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "ai-config",
                &mut std::io::stdout(),
            );
            ExitCode::SUCCESS
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
                McpCmd::Add => mcp::McpCmd::Add,
                McpCmd::Remove { name } => mcp::McpCmd::Remove { name },
                McpCmd::Enable { name } => mcp::McpCmd::Enable { name },
                McpCmd::Disable { name } => mcp::McpCmd::Disable { name },
                McpCmd::Deploy { name, to } => mcp::McpCmd::Deploy { name, to },
                McpCmd::Retract { name, from } => mcp::McpCmd::Retract { name, from },
                McpCmd::Migrate { source, dry_run } => mcp::McpCmd::Migrate { source, dry_run },
                McpCmd::MigrateHermes { dry_run } => mcp::McpCmd::MigrateHermes { dry_run },
            };
            mapped.run(mode, &default_root)
        }
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
        Cmd::Install => lifecycle::run_install(&default_root, mode),
        Cmd::Uninstall => {
            // Phase 1 不区分 interactive(无 stdin 提示);force 始终为 true。
            lifecycle::run_uninstall(&default_root, true, mode)
        }
        Cmd::Sync => lifecycle::run_sync(&default_root, mode),
        Cmd::Status => lifecycle::run_status(&default_root, mode),
        Cmd::List => lifecycle::run_list(&default_root, mode),
        Cmd::Show { name } => lifecycle::run_show(&default_root, &name, mode),
        Cmd::Doctor { materialize } => {
            lifecycle::run_doctor(&default_root, mode, materialize)
        }
    }
}

/// 解析资产根目录:优先级 `--root` > `AI_CONFIG_ROOT` > `~/.ai-config`(自动创建)。
fn resolve_root(flag: Option<&str>) -> Utf8PathBuf {
    let raw = match flag {
        Some(s) => s.to_string(),
        None => std::env::var("AI_CONFIG_ROOT").unwrap_or_else(|_| String::new()),
    };
    if raw.is_empty() {
        return paths::discover_global_asset_root();
    }
    paths::resolve_asset_root(Utf8Path::new(&raw))
}

// re-export 给 clap_complete
use clap::CommandFactory;
