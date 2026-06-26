//! 5 平台适配器:ai-config 资产源 + 4 个 IDE 下发目标。
//!
//! **Skills 路径上游参考**：[vercel-labs/skills](https://github.com/vercel-labs/skills) `src/agents.ts`；
//! 文档 `docs/reference/vercel-skills-agent-paths.md`，快照 `manifests/vercel-skills-agents.snapshot.json`。
//! 上游变更时先更新参考文档/快照，再评估是否改本模块。
//!
//! 目录约定(PRD §7.1 + ARCHITECTURE §4.3 + §5):
//! - **ai-config**: `<asset_root>/skills|rules|agents/` + `<asset_root>/mcp.json`
//! - **Cursor**: `<base>/.cursor/skills|rules|agents|commands/` + `mcp.json`
//! - **Codex**: `<base>/.codex/skills|rules|subagents/` + `mcp.json`（无斜杠 commands）
//! - **Claude**: `<base>/.claude/skills|rules|subagents|commands/` + `mcp.json`
//! - **Hermes**: skills 恒 `$HOME/.hermes/skills`;rules 仅项目 `<repo>/.cursor/rules/`;MCP 为 `config.yaml`
//!
//! Hermes 例外:
//! - **Skills**：始终 `$HOME/.hermes/skills`（或 `HERMES_SKILLS_DIR`）；项目作用域也写 `$HOME`。
//!   非默认路径须同步进 `config.yaml` → `skills.external_dirs`（见 `hermes_config`）。
//! - **Rules**：仅项目级 `<repo>/.cursor/rules/`（CWD 加载）；全局 user-global 不下发。
//! - **Agents**：Hermes 无静态 agents 目录（用 `AGENTS.md` / `delegate_task`），不下发。

use camino::Utf8PathBuf;
use std::sync::OnceLock;

use crate::error::CoreError;
use crate::model::{AssetKind, PlatformId};
use crate::paths;

/// 平台适配器唯一 trait(ARCHITECTURE §5)。
pub trait PlatformAdapter: Send + Sync {
    fn id(&self) -> PlatformId;
    fn skills_dir(&self) -> Utf8PathBuf;
    fn rules_dir(&self) -> Utf8PathBuf;
    fn agents_dir(&self) -> Utf8PathBuf;
    fn commands_dir(&self) -> Utf8PathBuf;
    fn mcp_json_path(&self) -> Utf8PathBuf;

    /// Hermes 等平台 MCP 实际写入路径（Hermes 恒为 `$HOME/.hermes/config.yaml`）。
    fn mcp_deploy_path(&self) -> Utf8PathBuf {
        self.mcp_json_path()
    }

    /// 平台级能力探测(PRD §6.3)。
    ///
    /// 默认实现:Skill / Mcp / Agent 全平台支持;Rule 在 Codex 上**不**直接消费。
    fn supports(&self, asset: AssetKind) -> bool {
        match asset {
            AssetKind::Skill | AssetKind::Mcp | AssetKind::Agent | AssetKind::Hook => true,
            AssetKind::Rule => self.id() != PlatformId::Codex,
            AssetKind::Command => matches!(self.id(), PlatformId::Cursor | PlatformId::Claude),
        }
    }
}

// ── home / Hermes env 解析 ──────────────────────────────────────────

/// 读 home 目录:macOS/Linux 走 `$HOME`,Windows 走 `$USERPROFILE`,都失败
/// 时退回相对路径 `.`(避免 panic — 单元测试要稳)。
fn home() -> Utf8PathBuf {
    if let Ok(h) = std::env::var("HOME") {
        if !h.is_empty() {
            return Utf8PathBuf::from(h);
        }
    }
    if let Ok(h) = std::env::var("USERPROFILE") {
        if !h.is_empty() {
            return Utf8PathBuf::from(h);
        }
    }
    Utf8PathBuf::from(".")
}

/// Hermes 官方默认 skills 目录：`$HOME/.hermes/skills`。
pub fn default_hermes_skills_dir() -> Utf8PathBuf {
    crate::hermes_config::default_hermes_skills_dir_at(&home())
}

/// ai-config 写入目标：优先 `HERMES_SKILLS_DIR`，退回官方默认。
fn hermes_skills_deploy_dir() -> Utf8PathBuf {
    std::env::var("HERMES_SKILLS_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(Utf8PathBuf::from)
        .unwrap_or_else(default_hermes_skills_dir)
}

/// 按平台 + 作用域判断是否支持该资产种类（`for_scope` 构造的适配器已含作用域语义）。
pub fn supports_at_scope(
    plat: PlatformId,
    kind: AssetKind,
    deploy_base: &camino::Utf8Path,
) -> bool {
    for_scope(plat, deploy_base)
        .map(|a| a.supports(kind))
        .unwrap_or(false)
}

// ── ai-config（资产源）────────────────────────────────────────────

struct AiConfigAdapter {
    asset_root: Utf8PathBuf,
}

impl PlatformAdapter for AiConfigAdapter {
    fn id(&self) -> PlatformId {
        PlatformId::AiConfig
    }
    fn skills_dir(&self) -> Utf8PathBuf {
        self.asset_root.join("skills")
    }
    fn rules_dir(&self) -> Utf8PathBuf {
        self.asset_root.join("rules")
    }
    fn agents_dir(&self) -> Utf8PathBuf {
        self.asset_root.join("agents")
    }
    fn commands_dir(&self) -> Utf8PathBuf {
        self.asset_root.join("commands")
    }
    fn mcp_json_path(&self) -> Utf8PathBuf {
        crate::mcp_json::mcp_json_path(&self.asset_root)
    }
    fn supports(&self, asset: AssetKind) -> bool {
        let _ = asset;
        true
    }
}

// ── Cursor ──────────────────────────────────────────────────────────

struct CursorAdapter {
    home: Utf8PathBuf,
}

impl PlatformAdapter for CursorAdapter {
    fn id(&self) -> PlatformId {
        PlatformId::Cursor
    }
    fn skills_dir(&self) -> Utf8PathBuf {
        self.home.join(".cursor/skills")
    }
    fn rules_dir(&self) -> Utf8PathBuf {
        self.home.join(".cursor/rules")
    }
    fn agents_dir(&self) -> Utf8PathBuf {
        self.home.join(".cursor/agents")
    }
    fn commands_dir(&self) -> Utf8PathBuf {
        self.home.join(".cursor/commands")
    }
    fn mcp_json_path(&self) -> Utf8PathBuf {
        self.home.join(".cursor/mcp.json")
    }
}

// ── Codex ───────────────────────────────────────────────────────────

struct CodexAdapter {
    home: Utf8PathBuf,
}

impl PlatformAdapter for CodexAdapter {
    fn id(&self) -> PlatformId {
        PlatformId::Codex
    }
    fn skills_dir(&self) -> Utf8PathBuf {
        self.home.join(".codex/skills")
    }
    fn rules_dir(&self) -> Utf8PathBuf {
        // Codex 不直接消费 rules(PRD §6.3 + ARCHITECTURE §5),保留目录
        // 约定但 `supports(Rule) == false`,实装层不会往里写东西。
        self.home.join(".codex/rules")
    }
    fn agents_dir(&self) -> Utf8PathBuf {
        // Codex 叫 subagents(PRD §2.2 + ARCHITECTURE §5)。
        self.home.join(".codex/subagents")
    }
    fn commands_dir(&self) -> Utf8PathBuf {
        self.home.join(".codex/commands")
    }
    fn mcp_json_path(&self) -> Utf8PathBuf {
        self.home.join(".codex/mcp.json")
    }
}

// ── Claude Code ─────────────────────────────────────────────────────

struct ClaudeAdapter {
    home: Utf8PathBuf,
}

impl PlatformAdapter for ClaudeAdapter {
    fn id(&self) -> PlatformId {
        PlatformId::Claude
    }
    fn skills_dir(&self) -> Utf8PathBuf {
        self.home.join(".claude/skills")
    }
    fn rules_dir(&self) -> Utf8PathBuf {
        self.home.join(".claude/rules")
    }
    fn agents_dir(&self) -> Utf8PathBuf {
        // Claude 叫 subagents(PRD §2.2 + ARCHITECTURE §5)。
        self.home.join(".claude/subagents")
    }
    fn commands_dir(&self) -> Utf8PathBuf {
        self.home.join(".claude/commands")
    }
    fn mcp_json_path(&self) -> Utf8PathBuf {
        self.home.join(".claude/mcp.json")
    }
}

// ── Hermes ──────────────────────────────────────────────────────────

struct HermesAdapter {
    /// 全局为 `$HOME`；项目为仓库根（仅用于 rules → `.cursor/rules`）。
    deploy_base: Utf8PathBuf,
    /// 始终为全局 skills 根（`HERMES_SKILLS_DIR` 或 `$HOME/.hermes/skills`）。
    skills_root: Utf8PathBuf,
}

impl HermesAdapter {
    fn for_deploy_base(deploy_base: Utf8PathBuf) -> Self {
        Self {
            deploy_base,
            skills_root: hermes_skills_deploy_dir(),
        }
    }
}

impl PlatformAdapter for HermesAdapter {
    fn id(&self) -> PlatformId {
        PlatformId::Hermes
    }
    fn supports(&self, asset: AssetKind) -> bool {
        match asset {
            AssetKind::Skill | AssetKind::Mcp => true,
            AssetKind::Hook => true,
            // Hermes 仅在项目 CWD 读 `.cursor/rules/*.mdc`（与 Cursor 项目级路径一致）
            AssetKind::Rule => self.deploy_base != home(),
            // 无 `~/.hermes/agents`；子代理为运行时 delegate_task
            AssetKind::Agent | AssetKind::Command => false,
        }
    }
    fn skills_dir(&self) -> Utf8PathBuf {
        self.skills_root.clone()
    }
    fn rules_dir(&self) -> Utf8PathBuf {
        self.deploy_base.join(".cursor/rules")
    }
    fn agents_dir(&self) -> Utf8PathBuf {
        self.deploy_base.join(".hermes/agents")
    }
    fn commands_dir(&self) -> Utf8PathBuf {
        self.deploy_base.join(".hermes/commands")
    }
    fn mcp_json_path(&self) -> Utf8PathBuf {
        crate::hermes_config::hermes_config_path()
    }
    fn mcp_deploy_path(&self) -> Utf8PathBuf {
        crate::hermes_config::hermes_config_path()
    }
}

// ── 工厂 + 全局 registry ────────────────────────────────────────────

pub fn aiconfig_adapter(asset_root: &camino::Utf8Path) -> Box<dyn PlatformAdapter> {
    Box::new(AiConfigAdapter {
        asset_root: asset_root.to_path_buf(),
    })
}

pub fn cursor_adapter() -> Result<Box<dyn PlatformAdapter>, CoreError> {
    Ok(Box::new(CursorAdapter { home: home() }))
}
pub fn codex_adapter() -> Result<Box<dyn PlatformAdapter>, CoreError> {
    Ok(Box::new(CodexAdapter { home: home() }))
}
pub fn claude_adapter() -> Result<Box<dyn PlatformAdapter>, CoreError> {
    Ok(Box::new(ClaudeAdapter { home: home() }))
}
pub fn hermes_adapter() -> Result<Box<dyn PlatformAdapter>, CoreError> {
    Ok(Box::new(HermesAdapter::for_deploy_base(home())))
}

/// 按 `PlatformId` 工厂(供 CLI / 同步层按平台拉一个适配器)。
pub fn for_id(id: PlatformId) -> Result<Box<dyn PlatformAdapter>, CoreError> {
    for_scope(id, &home())
}

/// GUI / 浏览用 5 平台（含 ai-config 源），固定顺序。
pub fn ui_platform_ids() -> [PlatformId; 5] {
    [
        PlatformId::AiConfig,
        PlatformId::Cursor,
        PlatformId::Codex,
        PlatformId::Claude,
        PlatformId::Hermes,
    ]
}

/// deploy / retract / 链接状态仅针对 4 个 IDE 目标。
pub fn deploy_platform_ids() -> [PlatformId; 4] {
    [
        PlatformId::Cursor,
        PlatformId::Codex,
        PlatformId::Claude,
        PlatformId::Hermes,
    ]
}

/// 按作用域 + 资产根解析平台目录（ai-config 读 `asset_root`，其余读 `deploy_base`）。
pub fn for_scope_with_asset(
    id: PlatformId,
    deploy_base: &camino::Utf8Path,
    asset_root: &camino::Utf8Path,
) -> Result<Box<dyn PlatformAdapter>, CoreError> {
    if id == PlatformId::AiConfig {
        return Ok(aiconfig_adapter(asset_root));
    }
    for_scope(id, deploy_base)
}

/// 某平台在某作用域下、某资产种类的根路径（skills/rules/agents 为目录，mcp 为配置文件路径）。
pub fn kind_asset_path(
    plat: PlatformId,
    kind: AssetKind,
    deploy_base: &camino::Utf8Path,
    asset_root: &camino::Utf8Path,
) -> Option<Utf8PathBuf> {
    let adapter = for_scope_with_asset(plat, deploy_base, asset_root).ok()?;
    if !adapter.supports(kind) {
        return None;
    }
    match kind {
        AssetKind::Skill => Some(adapter.skills_dir()),
        AssetKind::Rule => Some(adapter.rules_dir()),
        AssetKind::Agent => Some(adapter.agents_dir()),
        AssetKind::Command => Some(adapter.commands_dir()),
        AssetKind::Mcp => Some(adapter.mcp_deploy_path()),
        AssetKind::Hook => Some(match plat {
            PlatformId::Cursor => deploy_base.join(".cursor/hooks"),
            PlatformId::Codex => deploy_base.join(".codex/hooks"),
            PlatformId::Claude => deploy_base.join(".claude/hooks"),
            PlatformId::Hermes => paths::home_dir().join(".hermes/agent-hooks"),
            PlatformId::AiConfig => asset_root.join("hooks"),
        }),
    }
}

/// 按作用域解析平台目录。
///
/// - `deploy_base == $HOME`:与 `for_id` 相同(Hermes 尊重 `HERMES_SKILLS_DIR`)。
/// - 项目作用域:`deploy_base` 为仓库根,平台目录在 `<repo>/.cursor` 等。
/// - **不含** ai-config；请用 `for_scope_with_asset`。
pub fn for_scope(
    id: PlatformId,
    deploy_base: &camino::Utf8Path,
) -> Result<Box<dyn PlatformAdapter>, CoreError> {
    if id == PlatformId::AiConfig {
        return Err(CoreError::InvalidPath(
            "ai-config 平台需 asset_root，请使用 for_scope_with_asset".into(),
        ));
    }
    if deploy_base == home() {
        return match id {
            PlatformId::AiConfig => unreachable!(),
            PlatformId::Cursor => cursor_adapter(),
            PlatformId::Codex => codex_adapter(),
            PlatformId::Claude => claude_adapter(),
            PlatformId::Hermes => hermes_adapter(),
        };
    }
    let base = deploy_base.to_path_buf();
    Ok(match id {
        PlatformId::AiConfig => unreachable!(),
        PlatformId::Cursor => Box::new(CursorAdapter { home: base.clone() }),
        PlatformId::Codex => Box::new(CodexAdapter { home: base.clone() }),
        PlatformId::Claude => Box::new(ClaudeAdapter { home: base.clone() }),
        PlatformId::Hermes => Box::new(HermesAdapter::for_deploy_base(base)),
    })
}

/// 4 平台适配器(PRD §7.1 "4 平台都做")。返回顺序固定便于测试断言。
///
/// 内部用 `OnceLock` 缓存路径元数据(不再克隆对象本身,避免 trait object
/// 没有 Clone 的麻烦 — 每次返回全新 Box,只共享 home() 求值)。
pub fn registry() -> Vec<Box<dyn PlatformAdapter>> {
    vec![
        cursor_adapter().expect("cursor_adapter"),
        codex_adapter().expect("codex_adapter"),
        claude_adapter().expect("claude_adapter"),
        hermes_adapter().expect("hermes_adapter"),
    ]
}

// 保留 OnceLock 类型 re-export 以兼容历史调用方(若有人 use 了这个名字)。
#[allow(dead_code)]
type _CachedRegistry = OnceLock<()>;

/// 平台 ID 的短标签（JSON / GUI）。
pub fn platform_label(id: PlatformId) -> &'static str {
    match id {
        PlatformId::AiConfig => "aiconfig",
        PlatformId::Cursor => "cursor",
        PlatformId::Codex => "codex",
        PlatformId::Claude => "claude",
        PlatformId::Hermes => "hermes",
    }
}

/// 资产 kind 短标签。
pub fn asset_kind_label(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Skill => "skill",
        AssetKind::Rule => "rule",
        AssetKind::Mcp => "mcp",
        AssetKind::Agent => "agent",
        AssetKind::Command => "command",
        AssetKind::Hook => "hook",
    }
}

/// 解析平台字符串（含别名）。
pub fn parse_platform_str(s: &str) -> Result<PlatformId, CoreError> {
    match s {
        "aiconfig" | "ai-config" | "AiConfig" | "ac" => Ok(PlatformId::AiConfig),
        "cursor" | "Cursor" | "cu" => Ok(PlatformId::Cursor),
        "codex" | "Codex" | "cx" => Ok(PlatformId::Codex),
        "claude" | "Claude" | "cl" => Ok(PlatformId::Claude),
        "hermes" | "Hermes" | "he" => Ok(PlatformId::Hermes),
        other => Err(CoreError::InvalidPath(format!(
            "未知平台 `{other}`(预期 aiconfig/cursor/codex/claude/hermes)"
        ))),
    }
}

/// 解析 deploy / retract 目标平台（排除 aiconfig 源）。
pub fn parse_deploy_platform_str(s: &str) -> Result<PlatformId, CoreError> {
    let p = parse_platform_str(s)?;
    if !p.is_deploy_target() {
        return Err(CoreError::InvalidPath(format!(
            "平台 `{}` 为资产源，不能 deploy / retract",
            platform_label(p)
        )));
    }
    Ok(p)
}

/// 平台不支持某资产类型时的说明（doctor / GUI issue map）。
pub fn capability_skip_reason(plat: PlatformId, kind: AssetKind) -> String {
    match (plat, kind) {
        (PlatformId::Codex, AssetKind::Rule) => {
            "Codex 通过 AGENTS.md 间接引用 rules，不支持全局 symlink 下发".into()
        }
        (PlatformId::Hermes, AssetKind::Rule) => {
            "Hermes rules 仅项目级：请选已注册项目后 deploy 到 <repo>/.cursor/rules".into()
        }
        (PlatformId::Hermes, AssetKind::Agent) => {
            "Hermes 无静态 agents 目录；请用项目 AGENTS.md 或 delegate_task 子代理".into()
        }
        (PlatformId::Codex, AssetKind::Command) => "Codex 无斜杠 commands 目录，不支持下发".into(),
        (PlatformId::Hermes, AssetKind::Command) => "Hermes 无斜杠 commands，不支持下发".into(),
        _ => format!(
            "platform `{}` 不支持 asset kind `{}`",
            platform_label(plat),
            asset_kind_label(kind)
        ),
    }
}

/// 收集各平台 × 资产类型的能力缺口（doctor / GUI）。
pub fn collect_capability_issues(deploy_base: &camino::Utf8Path) -> Vec<CapabilityIssue> {
    let mut out = Vec::new();
    for plat in deploy_platform_ids() {
        for kind in [
            AssetKind::Skill,
            AssetKind::Rule,
            AssetKind::Mcp,
            AssetKind::Agent,
            AssetKind::Command,
            AssetKind::Hook,
        ] {
            if supports_at_scope(plat, kind, deploy_base) {
                continue;
            }
            out.push(CapabilityIssue {
                platform: plat,
                kind: asset_kind_label(kind).to_string(),
                reason: capability_skip_reason(plat, kind),
            });
        }
    }
    out
}

/// 单条平台能力问题。
#[derive(Debug, Clone, serde::Serialize)]
pub struct CapabilityIssue {
    pub platform: PlatformId,
    pub kind: String,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── env 守卫:设/恢复 HERMES_SKILLS_DIR 等,避免污染其它测试 ──────
    struct EnvGuard {
        key: &'static str,
        prev: Option<String>,
    }
    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, prev }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    // ── 4 个适配器目录路径正确(macOS 用 dirs::home_dir 验证前缀) ───

    fn assert_starts_with_home(p: &Utf8PathBuf) {
        if let Some(dirs_home) = dirs::home_dir() {
            let dirs_home =
                Utf8PathBuf::from_path_buf(dirs_home).expect("dirs::home_dir 应该是 UTF-8 路径");
            assert!(
                p.starts_with(&dirs_home),
                "路径 {p} 应以 home {dirs_home} 为前缀"
            );
        }
    }

    #[test]
    fn cursor_paths_live_under_home() {
        let a = cursor_adapter().unwrap();
        assert_eq!(a.id(), PlatformId::Cursor);
        assert!(a.skills_dir().as_str().ends_with(".cursor/skills"));
        assert!(a.rules_dir().as_str().ends_with(".cursor/rules"));
        assert!(a.agents_dir().as_str().ends_with(".cursor/agents"));
        assert!(a.mcp_json_path().as_str().ends_with(".cursor/mcp.json"));
        for p in [
            a.skills_dir(),
            a.rules_dir(),
            a.agents_dir(),
            a.mcp_json_path(),
        ] {
            assert_starts_with_home(&p);
        }
    }

    #[test]
    fn codex_paths_live_under_home() {
        let a = codex_adapter().unwrap();
        assert_eq!(a.id(), PlatformId::Codex);
        assert!(a.skills_dir().as_str().ends_with(".codex/skills"));
        assert!(a.rules_dir().as_str().ends_with(".codex/rules"));
        // Codex 叫 subagents
        assert!(a.agents_dir().as_str().ends_with(".codex/subagents"));
        assert!(a.mcp_json_path().as_str().ends_with(".codex/mcp.json"));
        for p in [
            a.skills_dir(),
            a.rules_dir(),
            a.agents_dir(),
            a.mcp_json_path(),
        ] {
            assert_starts_with_home(&p);
        }
    }

    #[test]
    fn claude_paths_live_under_home() {
        let a = claude_adapter().unwrap();
        assert_eq!(a.id(), PlatformId::Claude);
        assert!(a.skills_dir().as_str().ends_with(".claude/skills"));
        assert!(a.rules_dir().as_str().ends_with(".claude/rules"));
        // Claude 叫 subagents(PRD §2.2)
        assert!(a.agents_dir().as_str().ends_with(".claude/subagents"));
        assert!(a.mcp_json_path().as_str().ends_with(".claude/mcp.json"));
        for p in [
            a.skills_dir(),
            a.rules_dir(),
            a.agents_dir(),
            a.mcp_json_path(),
        ] {
            assert_starts_with_home(&p);
        }
    }

    #[test]
    fn hermes_paths_live_under_home_when_env_unset() {
        // 清掉可能的残留 env
        let _g = EnvGuard::set("HERMES_SKILLS_DIR", "/tmp/ignored-for-this-assert");
        // 撤掉刚才的 set,改成 remove,确保走默认路径
        drop(_g);
        let prev = std::env::var("HERMES_SKILLS_DIR").ok();
        std::env::remove_var("HERMES_SKILLS_DIR");

        let a = hermes_adapter().unwrap();
        assert_eq!(a.id(), PlatformId::Hermes);
        assert!(a.skills_dir().as_str().ends_with(".hermes/skills"));
        assert!(a.rules_dir().as_str().ends_with(".cursor/rules"));
        assert!(!a.supports(AssetKind::Rule));
        assert!(!a.supports(AssetKind::Agent));
        assert!(a
            .mcp_deploy_path()
            .as_str()
            .ends_with(".hermes/config.yaml"));
        for p in [a.skills_dir(), a.rules_dir(), a.mcp_deploy_path()] {
            assert_starts_with_home(&p);
        }

        // 恢复
        if let Some(v) = prev {
            std::env::set_var("HERMES_SKILLS_DIR", v);
        }
    }

    // ── CodexAdapter.supports(Rule) == false(PRD §6.3) ─────────────

    #[test]
    fn codex_does_not_support_rules() {
        let a = codex_adapter().unwrap();
        // PRD §6.3:Codex 不直接消费 rules,通过 AGENTS.md 间接
        assert!(!a.supports(AssetKind::Rule));
    }

    #[test]
    fn codex_supports_agents() {
        let a = codex_adapter().unwrap();
        assert!(a.supports(AssetKind::Agent));
        assert!(a.agents_dir().as_str().ends_with(".codex/subagents"));
    }

    #[test]
    fn codex_supports_skill_and_mcp() {
        let a = codex_adapter().unwrap();
        assert!(a.supports(AssetKind::Skill));
        assert!(a.supports(AssetKind::Mcp));
    }

    #[test]
    fn cursor_and_claude_support_all_four_assets() {
        for id in [PlatformId::Cursor, PlatformId::Claude] {
            let a = for_id(id).unwrap();
            for k in [
                AssetKind::Skill,
                AssetKind::Rule,
                AssetKind::Mcp,
                AssetKind::Agent,
            ] {
                assert!(a.supports(k), "{:?} should support {:?}", id, k);
            }
        }
    }

    #[test]
    fn hermes_supports_skill_and_mcp_only_at_global() {
        let a = hermes_adapter().unwrap();
        assert!(a.supports(AssetKind::Skill));
        assert!(a.supports(AssetKind::Mcp));
        assert!(!a.supports(AssetKind::Agent));
        assert!(!a.supports(AssetKind::Rule));
    }

    #[test]
    fn hermes_project_scope_rules_and_global_skills() {
        let prev = std::env::var("HERMES_SKILLS_DIR").ok();
        std::env::remove_var("HERMES_SKILLS_DIR");
        let repo = Utf8PathBuf::from("/tmp/hermes-repo-fixture");
        let a = for_scope(PlatformId::Hermes, &repo).unwrap();
        assert!(a.supports(AssetKind::Rule));
        assert_eq!(a.rules_dir(), repo.join(".cursor/rules"));
        assert!(a.skills_dir().as_str().ends_with(".hermes/skills"));
        assert_ne!(a.skills_dir(), repo.join(".hermes/skills"));
        if let Some(v) = prev {
            std::env::set_var("HERMES_SKILLS_DIR", v);
        }
    }

    // ── HermesAdapter 读 HERMES_SKILLS_DIR env(PRD §5.x) ───────────

    #[test]
    fn hermes_reads_hermes_skills_dir_env() {
        let _g = EnvGuard::set("HERMES_SKILLS_DIR", "/opt/hermes/custom-skills");
        let a = hermes_adapter().unwrap();
        assert_eq!(
            a.skills_dir(),
            Utf8PathBuf::from("/opt/hermes/custom-skills"),
            "skills 目录应走 HERMES_SKILLS_DIR env"
        );
        assert!(a.rules_dir().as_str().ends_with(".cursor/rules"));
        assert!(!a.supports(AssetKind::Agent));
        assert!(a
            .mcp_deploy_path()
            .as_str()
            .ends_with(".hermes/config.yaml"));
        // _g drop 时自动恢复
    }

    #[test]
    fn hermes_skills_dir_falls_back_to_default_when_env_unset() {
        let prev = std::env::var("HERMES_SKILLS_DIR").ok();
        std::env::remove_var("HERMES_SKILLS_DIR");
        let a = hermes_adapter().unwrap();
        assert!(a.skills_dir().as_str().ends_with(".hermes/skills"));
        if let Some(v) = prev {
            std::env::set_var("HERMES_SKILLS_DIR", v);
        }
    }

    #[test]
    fn hermes_skills_dir_treats_empty_env_as_unset() {
        let _g = EnvGuard::set("HERMES_SKILLS_DIR", "");
        let a = hermes_adapter().unwrap();
        // 空字符串应被当作"未设",退回默认
        assert!(a.skills_dir().as_str().ends_with(".hermes/skills"));
    }

    // ── registry / for_id 工厂 ───────────────────────────────────

    #[test]
    fn for_scope_uses_repo_root_for_project() {
        let repo = Utf8PathBuf::from("/tmp/my-repo");
        let a = for_scope(PlatformId::Cursor, &repo).unwrap();
        assert_eq!(a.skills_dir(), repo.join(".cursor/skills"));
        assert_eq!(a.mcp_json_path(), repo.join(".cursor/mcp.json"));
    }

    #[test]
    fn registry_returns_four_platforms_in_canonical_order() {
        let r = registry();
        assert_eq!(r.len(), 4, "4 平台都做(PRD §7.1)");
        assert_eq!(r[0].id(), PlatformId::Cursor);
        assert_eq!(r[1].id(), PlatformId::Codex);
        assert_eq!(r[2].id(), PlatformId::Claude);
        assert_eq!(r[3].id(), PlatformId::Hermes);
    }

    #[test]
    fn for_id_returns_correct_adapter_for_each_platform() {
        for id in [
            PlatformId::Cursor,
            PlatformId::Codex,
            PlatformId::Claude,
            PlatformId::Hermes,
        ] {
            let a = for_id(id).expect("for_id 不应失败");
            assert_eq!(a.id(), id);
        }
    }

    #[test]
    fn for_id_codex_has_subagents_dir() {
        let a = for_id(PlatformId::Codex).unwrap();
        assert!(a.agents_dir().as_str().ends_with(".codex/subagents"));
    }

    #[test]
    fn for_id_claude_has_subagents_dir() {
        let a = for_id(PlatformId::Claude).unwrap();
        assert!(a.agents_dir().as_str().ends_with(".claude/subagents"));
    }

    #[test]
    fn for_id_cursor_has_agents_dir() {
        let a = for_id(PlatformId::Cursor).unwrap();
        assert!(a.agents_dir().as_str().ends_with(".cursor/agents"));
    }

    #[test]
    fn for_id_hermes_has_no_agent_deploy() {
        let prev = std::env::var("HERMES_SKILLS_DIR").ok();
        std::env::remove_var("HERMES_SKILLS_DIR");
        let a = for_id(PlatformId::Hermes).unwrap();
        assert!(!a.supports(AssetKind::Agent));
        if let Some(v) = prev {
            std::env::set_var("HERMES_SKILLS_DIR", v);
        }
    }
}
