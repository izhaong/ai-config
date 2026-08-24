//! 内存数据类型 — align ARCHITECTURE §3。
//!
//! 关键设计:Rule **无** `scope` 字段(单一源 `rules/*.mdc`,PRD §11.2);
//! SyncAction 是 per-item × per-platform 的下/收动作(PRD §4.2 核心)。
//! Project 是唯一主源(视图反转,PRD §3.1)。
//!
//! Phase 1 实装;当前是占位 stub。

#![allow(dead_code)]

use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 项目:本工具纳管的代码仓根目录(唯一主源,PRD §3.1)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: u64,
    pub name: String,
    pub root_path: Utf8PathBuf,
    pub registered_at: DateTime<Utc>,
}

impl Project {
    /// 新建项目;`id = 0` 表示尚未持久化(由 store 分配),`registered_at` 取 `Utc::now()`。
    pub fn new(name: impl Into<String>, root_path: impl Into<Utf8PathBuf>) -> Self {
        Self {
            id: 0,
            name: name.into(),
            root_path: root_path.into(),
            registered_at: Utc::now(),
        }
    }
}

impl Default for Project {
    fn default() -> Self {
        Self::new("", Utf8PathBuf::new())
    }
}

/// 4 类资产之一(PRD §2)。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum AssetKind {
    Skill,
    Rule,
    Mcp,
    Agent,
    Command,
    /// Canonical prompt source; legacy lifecycle must reject it until T008.
    Prompt,
    Hook,
}

/// 单条 skill(目录形态,`SKILL.md` + 可能的 scripts/ / assets/ / agents/)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub project_id: u64,
    pub name: String,
    pub source_path: Utf8PathBuf,
    pub description: String,
    pub updated_at: DateTime<Utc>,
}

impl Skill {
    pub fn new(
        project_id: u64,
        name: impl Into<String>,
        source_path: impl Into<Utf8PathBuf>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            project_id,
            name: name.into(),
            source_path: source_path.into(),
            description: description.into(),
            updated_at: Utc::now(),
        }
    }
}

/// 单条 rule(`.mdc` / `.md` 单文件)。
///
/// **无** `scope` 字段:rule 是单一源,不分平台目录(PRD §11.2)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub project_id: u64,
    pub name: String,
    pub source_path: Utf8PathBuf,
    pub updated_at: DateTime<Utc>,
}

impl Rule {
    pub fn new(
        project_id: u64,
        name: impl Into<String>,
        source_path: impl Into<Utf8PathBuf>,
    ) -> Self {
        Self {
            project_id,
            name: name.into(),
            source_path: source_path.into(),
            updated_at: Utc::now(),
        }
    }
}

/// MCP transport 类型。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    Stdio,
    Http,
    Sse,
}

/// 单条 MCP server(独立条目,**不**是整份 mcp.json;PRD §2.1)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServer {
    pub project_id: u64,
    pub name: String,
    pub transport: McpTransport,
    /// 模板(去除密钥后的 config)
    pub config: serde_json::Value,
    /// `${VAR}` 占位符列表;渲染时从 secrets.env 注入
    pub secret_keys: Vec<String>,
    pub enabled: bool,
}

impl McpServer {
    pub fn new(
        project_id: u64,
        name: impl Into<String>,
        transport: McpTransport,
        config: serde_json::Value,
    ) -> Self {
        Self {
            project_id,
            name: name.into(),
            transport,
            config,
            secret_keys: Vec::new(),
            enabled: true,
        }
    }
}

/// 单条 agent / subagent(按平台约定是文件或目录)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub project_id: u64,
    pub name: String,
    pub source_path: Utf8PathBuf,
    pub updated_at: DateTime<Utc>,
}

impl Agent {
    pub fn new(
        project_id: u64,
        name: impl Into<String>,
        source_path: impl Into<Utf8PathBuf>,
    ) -> Self {
        Self {
            project_id,
            name: name.into(),
            source_path: source_path.into(),
            updated_at: Utc::now(),
        }
    }
}

/// 5 个平台：agent-manager 为资产源；其余 4 个为 IDE 下发目标。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum PlatformId {
    AiConfig,
    Cursor,
    Codex,
    Claude,
    Hermes,
}

impl PlatformId {
    /// 是否可向该平台 deploy / retract（agent-manager 仅为源，不下发）。
    pub fn is_deploy_target(self) -> bool {
        !matches!(self, PlatformId::AiConfig)
    }
}

/// 链接 / 渲染目标(per-item × per-platform)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymlinkTarget {
    pub item_id: u64,
    pub platform: PlatformId,
    pub dest_path: Utf8PathBuf,
    pub kind: TargetKind,
    pub state: TargetState,
    pub last_checked: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    /// symlink / junction / hardlink
    Symlink,
    /// 渲染的 mcp.json
    RenderedJson,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TargetState {
    Linked,
    Unlinked,
    /// 工具先前下发的链接被收回(PRD §4.2 收回动作)
    Retracted,
    /// 源文件丢失
    Missing,
    /// 渲染或链接失败
    Failed,
}

/// per-item × per-platform 同步动作(PRD §4.2 核心)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SyncAction {
    /// 新建链接(skill / rule / agent)
    Create {
        item_id: u64,
        platform: PlatformId,
        dest: Utf8PathBuf,
        /// 资产源路径(skill 为 `SKILL.md`,rule/agent 为源文件或目录)
        src: Utf8PathBuf,
    },
    /// 链接已存在,无需变更
    Linked { item_id: u64, platform: PlatformId },
    /// 重新渲染 mcp.json(添加/更新 server)
    RenderMcp {
        project_id: u64,
        platform: PlatformId,
    },
    /// 收回链接
    Retract {
        item_id: u64,
        platform: PlatformId,
        dest: Utf8PathBuf,
    },
    /// 收回 + 移除 mcp.json 中的 server
    Unlink { item_id: u64, platform: PlatformId },
    /// 下发 hook（分平台 adapter 合并配置 + 拷贝脚本）
    DeployHook {
        item_id: u64,
        platform: PlatformId,
        name: String,
        src: Utf8PathBuf,
        asset_root: Utf8PathBuf,
        deploy_base: Utf8PathBuf,
    },
}

impl SyncAction {
    /// 取出该动作的目标平台(5 个变体都有 `platform` 字段)。
    pub fn platform(&self) -> PlatformId {
        match self {
            Self::Create { platform, .. }
            | Self::Linked { platform, .. }
            | Self::RenderMcp { platform, .. }
            | Self::Retract { platform, .. }
            | Self::Unlink { platform, .. }
            | Self::DeployHook { platform, .. } => *platform,
        }
    }

    /// 是否为「新建链接」动作(对应 PRD §4.2 下发动作)。
    pub fn is_create(&self) -> bool {
        matches!(self, Self::Create { .. })
    }

    /// 是否为「收回链接」动作(对应 PRD §4.2 收回动作)。
    pub fn is_retract(&self) -> bool {
        matches!(self, Self::Retract { .. })
    }

    /// 目标路径(仅 `Create` / `Retract` 携带 `dest`;其它变体返回 `None`)。
    pub fn dest(&self) -> Option<&Utf8PathBuf> {
        match self {
            Self::Create { dest, .. } | Self::Retract { dest, .. } => Some(dest),
            Self::Linked { .. } | Self::RenderMcp { .. } | Self::Unlink { .. } => None,
            Self::DeployHook { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- Project ----------

    #[test]
    fn project_new_sets_fields() {
        let p = Project::new("zh-cloud", "/Users/zhonghao/Code/zh-cloud");
        assert_eq!(p.name, "zh-cloud");
        assert_eq!(p.root_path.as_str(), "/Users/zhonghao/Code/zh-cloud");
        // id 留 0 表示尚未持久化
        assert_eq!(p.id, 0);
        // registered_at 取 Utc::now,应与墙钟时间一致(允许 1s 漂移)
        let now = Utc::now();
        let drift = (now - p.registered_at).num_seconds().abs();
        assert!(drift <= 1, "registered_at 偏离 Utc::now 超过 1s");
    }

    #[test]
    fn project_default_is_empty() {
        let p = Project::default();
        assert_eq!(p.name, "");
        assert_eq!(p.root_path.as_str(), "");
        assert_eq!(p.id, 0);
    }

    // ---------- Skill / Rule / McpServer / Agent 构造器 ----------

    #[test]
    fn skill_new_sets_fields() {
        let s = Skill::new(7, "git-sync-gitea", "skills/git-sync-gitea", "Git 同步");
        assert_eq!(s.project_id, 7);
        assert_eq!(s.name, "git-sync-gitea");
        assert_eq!(s.source_path.as_str(), "skills/git-sync-gitea");
        assert_eq!(s.description, "Git 同步");
    }

    #[test]
    fn rule_new_sets_fields() {
        let r = Rule::new(3, "alibaba-java", "rules/alibaba-java.mdc");
        assert_eq!(r.project_id, 3);
        assert_eq!(r.name, "alibaba-java");
        assert_eq!(r.source_path.as_str(), "rules/alibaba-java.mdc");
    }

    #[test]
    fn mcp_server_new_defaults() {
        let cfg = serde_json::json!({"command": "npx", "args": ["-y", "@x/y"]});
        let m = McpServer::new(11, "fetch", McpTransport::Stdio, cfg.clone());
        assert_eq!(m.project_id, 11);
        assert_eq!(m.name, "fetch");
        assert_eq!(m.transport, McpTransport::Stdio);
        assert_eq!(m.config, cfg);
        assert!(m.secret_keys.is_empty());
        assert!(m.enabled);
    }

    #[test]
    fn agent_new_sets_fields() {
        let a = Agent::new(5, "code-reviewer", "agents/code-reviewer.md");
        assert_eq!(a.project_id, 5);
        assert_eq!(a.name, "code-reviewer");
        assert_eq!(a.source_path.as_str(), "agents/code-reviewer.md");
    }

    // ---------- SyncAction 平台 / is_create / is_retract / dest ----------

    #[test]
    fn sync_action_platform_across_variants() {
        let dest = Utf8PathBuf::from(".cursor/rules/foo.mdc");

        let create = SyncAction::Create {
            item_id: 1,
            platform: PlatformId::Cursor,
            dest: dest.clone(),
            src: Utf8PathBuf::from("skills/foo/SKILL.md"),
        };
        assert_eq!(create.platform(), PlatformId::Cursor);

        let linked = SyncAction::Linked {
            item_id: 2,
            platform: PlatformId::Codex,
        };
        assert_eq!(linked.platform(), PlatformId::Codex);

        let render = SyncAction::RenderMcp {
            project_id: 1,
            platform: PlatformId::Claude,
        };
        assert_eq!(render.platform(), PlatformId::Claude);

        let retract = SyncAction::Retract {
            item_id: 3,
            platform: PlatformId::Hermes,
            dest: dest.clone(),
        };
        assert_eq!(retract.platform(), PlatformId::Hermes);

        let unlink = SyncAction::Unlink {
            item_id: 4,
            platform: PlatformId::Cursor,
        };
        assert_eq!(unlink.platform(), PlatformId::Cursor);
    }

    #[test]
    fn sync_action_is_create_and_is_retract() {
        let dest = Utf8PathBuf::from(".cursor/skills/foo");
        let create = SyncAction::Create {
            item_id: 1,
            platform: PlatformId::Cursor,
            dest: dest.clone(),
            src: Utf8PathBuf::from("skills/foo/SKILL.md"),
        };
        assert!(create.is_create());
        assert!(!create.is_retract());
        assert_eq!(create.dest().unwrap(), &dest);

        let retract = SyncAction::Retract {
            item_id: 1,
            platform: PlatformId::Cursor,
            dest: dest.clone(),
        };
        assert!(retract.is_retract());
        assert!(!retract.is_create());
        assert_eq!(retract.dest().unwrap(), &dest);

        let linked = SyncAction::Linked {
            item_id: 1,
            platform: PlatformId::Cursor,
        };
        assert!(!linked.is_create());
        assert!(!linked.is_retract());
        assert!(linked.dest().is_none());

        let unlink = SyncAction::Unlink {
            item_id: 1,
            platform: PlatformId::Cursor,
        };
        assert!(!unlink.is_create());
        assert!(!unlink.is_retract());
        assert!(unlink.dest().is_none());

        let render = SyncAction::RenderMcp {
            project_id: 1,
            platform: PlatformId::Cursor,
        };
        assert!(!render.is_create());
        assert!(!render.is_retract());
        assert!(render.dest().is_none());
    }

    // ---------- serde 往返 ----------

    #[test]
    fn sync_action_create_roundtrip() {
        let original = SyncAction::Create {
            item_id: 42,
            platform: PlatformId::Cursor,
            dest: Utf8PathBuf::from(".cursor/rules/git-flow-and-release.mdc"),
            src: Utf8PathBuf::from("rules/git-flow-and-release.mdc"),
        };
        let json = serde_json::to_string(&original).expect("serialize");
        // 验证 tag 形式 + 字段命名
        assert!(
            json.contains("\"action\":\"create\""),
            "expected snake_case action tag, got {json}"
        );
        assert!(
            json.contains("\"platform\":\"cursor\""),
            "expected lowercase platform, got {json}"
        );
        assert!(json.contains("\"item_id\":42"), "got {json}");

        let decoded: SyncAction = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, original);
        assert!(decoded.is_create());
        assert_eq!(
            decoded.dest().unwrap().as_str(),
            ".cursor/rules/git-flow-and-release.mdc"
        );
    }

    #[test]
    fn asset_kind_serde_lowercase() {
        // 验证 AssetKind 的 4 个变体均使用 lowercase 命名(PRD §2)。
        for (k, expected) in [
            (AssetKind::Skill, "\"skill\""),
            (AssetKind::Rule, "\"rule\""),
            (AssetKind::Mcp, "\"mcp\""),
            (AssetKind::Agent, "\"agent\""),
            (AssetKind::Command, "\"command\""),
            (AssetKind::Prompt, "\"prompt\""),
            (AssetKind::Hook, "\"hook\""),
        ] {
            assert_eq!(serde_json::to_string(&k).unwrap(), expected);
        }
    }

    #[test]
    fn target_state_serde_snake_case() {
        // 验证 TargetState 使用 snake_case(Retracted / Unlinked 等)。
        for (s, expected) in [
            (TargetState::Linked, "\"linked\""),
            (TargetState::Unlinked, "\"unlinked\""),
            (TargetState::Retracted, "\"retracted\""),
            (TargetState::Missing, "\"missing\""),
            (TargetState::Failed, "\"failed\""),
        ] {
            assert_eq!(serde_json::to_string(&s).unwrap(), expected);
        }
    }
}
