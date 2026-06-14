//! 4 平台适配器:暴露每个平台的 skills / rules / agents / mcp.json 目录约定。
//!
//! 目录约定(PRD §7.1 + ARCHITECTURE §4.3 + §5):
//! - `~/.{platform}/skills/`
//! - `~/.{platform}/rules/`
//! - `~/.{platform}/agents/` (Cursor/Hermes);Codex/Claude 叫 `subagents/`
//! - `~/.{platform}/mcp.json`
//!
//! Hermes 例外:skills 目录可由 `HERMES_SKILLS_DIR` env 覆盖(PRD §5.x)。

use camino::Utf8PathBuf;
use std::sync::OnceLock;

use crate::error::CoreError;
use crate::model::{AssetKind, PlatformId};

/// 平台适配器唯一 trait(ARCHITECTURE §5)。
pub trait PlatformAdapter: Send + Sync {
    fn id(&self) -> PlatformId;
    fn skills_dir(&self) -> Utf8PathBuf;
    fn rules_dir(&self) -> Utf8PathBuf;
    fn agents_dir(&self) -> Utf8PathBuf;
    fn mcp_json_path(&self) -> Utf8PathBuf;

    /// 平台级能力探测(PRD §6.3)。
    ///
    /// 默认实现:Skill / Mcp / Agent 全平台支持;Rule 在 Codex / Hermes 上**不**直接
    /// 消费 — Codex Rule 通过 AGENTS.md 间接;Hermes Rule 仅 CWD `.cursor/rules/`。
    fn supports(&self, asset: AssetKind) -> bool {
        match asset {
            AssetKind::Skill | AssetKind::Mcp | AssetKind::Agent => true,
            AssetKind::Rule => !matches!(self.id(), PlatformId::Codex | PlatformId::Hermes),
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

/// Hermes 的 skills 目录(PRD §5.x):优先 `HERMES_SKILLS_DIR` env,退回
/// `$HOME/.hermes/skills`。
fn hermes_skills_dir() -> Utf8PathBuf {
    std::env::var("HERMES_SKILLS_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(Utf8PathBuf::from)
        .unwrap_or_else(|| home().join(".hermes/skills"))
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
    fn mcp_json_path(&self) -> Utf8PathBuf {
        self.home.join(".claude/mcp.json")
    }
}

// ── Hermes ──────────────────────────────────────────────────────────

struct HermesAdapter {
    home: Utf8PathBuf,
    /// skills 目录独立存储(可能与 home 解耦,env 覆盖场景)。
    skills_root: Utf8PathBuf,
}

impl HermesAdapter {
    /// 默认构造:从 env 读 `HERMES_SKILLS_DIR`(失败退回 `$HOME/.hermes/skills`)。
    fn from_env() -> Self {
        Self {
            home: home(),
            skills_root: hermes_skills_dir(),
        }
    }
}

impl PlatformAdapter for HermesAdapter {
    fn id(&self) -> PlatformId {
        PlatformId::Hermes
    }
    fn supports(&self, asset: AssetKind) -> bool {
        match asset {
            AssetKind::Skill | AssetKind::Mcp | AssetKind::Agent => true,
            // Hermes 只在工作目录读 `.cursor/rules/*.mdc`,不消费全局 ~/.hermes/rules
            AssetKind::Rule => false,
        }
    }
    fn skills_dir(&self) -> Utf8PathBuf {
        self.skills_root.clone()
    }
    fn rules_dir(&self) -> Utf8PathBuf {
        // 保留路径约定供文档/未来扩展;`supports(Rule)==false` 时不会写入
        self.home.join(".hermes/rules")
    }
    fn agents_dir(&self) -> Utf8PathBuf {
        self.home.join(".hermes/agents")
    }
    fn mcp_json_path(&self) -> Utf8PathBuf {
        self.home.join(".hermes/mcp.json")
    }
}

// ── 工厂 + 全局 registry ────────────────────────────────────────────

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
    Ok(Box::new(HermesAdapter::from_env()))
}

/// 按 `PlatformId` 工厂(供 CLI / 同步层按平台拉一个适配器)。
pub fn for_id(id: PlatformId) -> Result<Box<dyn PlatformAdapter>, CoreError> {
    for_scope(id, &home())
}

/// 按作用域解析平台目录。
///
/// - `deploy_base == $HOME`:与 `for_id` 相同(Hermes 尊重 `HERMES_SKILLS_DIR`)。
/// - 项目作用域:`deploy_base` 为仓库根,平台目录在 `<repo>/.cursor` 等。
pub fn for_scope(
    id: PlatformId,
    deploy_base: &camino::Utf8Path,
) -> Result<Box<dyn PlatformAdapter>, CoreError> {
    if deploy_base == home() {
        return match id {
            PlatformId::Cursor => cursor_adapter(),
            PlatformId::Codex => codex_adapter(),
            PlatformId::Claude => claude_adapter(),
            PlatformId::Hermes => hermes_adapter(),
        };
    }
    let base = deploy_base.to_path_buf();
    Ok(match id {
        PlatformId::Cursor => Box::new(CursorAdapter { home: base.clone() }),
        PlatformId::Codex => Box::new(CodexAdapter { home: base.clone() }),
        PlatformId::Claude => Box::new(ClaudeAdapter { home: base.clone() }),
        PlatformId::Hermes => Box::new(HermesAdapter {
            home: base.clone(),
            skills_root: base.join(".hermes/skills"),
        }),
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
        assert!(a.rules_dir().as_str().ends_with(".hermes/rules"));
        assert!(a.agents_dir().as_str().ends_with(".hermes/agents"));
        assert!(a.mcp_json_path().as_str().ends_with(".hermes/mcp.json"));
        for p in [
            a.skills_dir(),
            a.rules_dir(),
            a.agents_dir(),
            a.mcp_json_path(),
        ] {
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
    fn hermes_supports_skill_mcp_and_agent() {
        let a = hermes_adapter().unwrap();
        assert!(a.supports(AssetKind::Skill));
        assert!(a.supports(AssetKind::Mcp));
        assert!(a.supports(AssetKind::Agent));
        assert!(!a.supports(AssetKind::Rule));
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
        // rules / agents / mcp 仍走 home(与 skills 解耦)
        assert!(a.rules_dir().as_str().ends_with(".hermes/rules"));
        assert!(a.agents_dir().as_str().ends_with(".hermes/agents"));
        assert!(a.mcp_json_path().as_str().ends_with(".hermes/mcp.json"));
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
    fn for_id_hermes_has_agents_dir() {
        // 显式清 env 避免与 hermes_reads_* 测试相互污染
        let prev = std::env::var("HERMES_SKILLS_DIR").ok();
        std::env::remove_var("HERMES_SKILLS_DIR");
        let a = for_id(PlatformId::Hermes).unwrap();
        assert!(a.agents_dir().as_str().ends_with(".hermes/agents"));
        if let Some(v) = prev {
            std::env::set_var("HERMES_SKILLS_DIR", v);
        }
    }
}
