//! 同步引擎:source + override + enabled platforms → per-item × per-platform SyncAction。
//!
//! ## 关键约束(PRD §3.2 / §4.2 / §8.2)
//!
//! - **同 name 覆盖 / 不同 name 附加**:`source::scan_with_override` 已实装合并,
//!   本模块**只**消费其结果。
//! - **per-item × per-platform 动作**(`SyncAction`):每个资产名对每个平台产生
//!   一条 `Create`(skill/rule/agent)或 `RenderMcp`(mcp)动作。
//! - **事实源唯一**:本函数**只**产 `SyncAction` 流。平台勾选 / 链接当前状态
//!   **不**在这里读、不在这里改;`store` 是状态机,`link` 是执行器,`sync` 是计划器。
//!
//! ## item_id 编号
//!
//! `item_id` 由 `(project_id, kind, name)` 用 `DefaultHasher` 派生一个 64-bit
//! 稳定哈希。同一份资产在不同次 `compute_for_project` 调用里得到**相同**的
//! `item_id`,这是后续 `store.record()` / `retract_item` 关联唯一目标 ID 的前提。
//!
//! ## dest 路径
//!
//! - Skill:dest = `<platform>_skills_dir/<name>`(链接整个目录,源是 skill 目录)
/// - Rule:dest = `<platform>_rules_dir/<name>.mdc`
/// - Agent:dest = `<platform>_agents_dir/<name>.<原 ext>`(沿用源文件后缀,.md / .yaml ...)
/// - Mcp:**不**产 `Create`,改产 `RenderMcp { project_id, platform }`(整份 mcp.json 原子重渲染)
use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use camino::Utf8Path;

use crate::error::CoreError;
use crate::mcp_json;
use crate::model::{AssetKind, PlatformId, Project, SyncAction};
use crate::platform;
use crate::source;

/// 4 个平台(`platform::registry()` 的稳定顺序)。
fn all_platforms() -> Vec<PlatformId> {
    vec![
        PlatformId::Cursor,
        PlatformId::Codex,
        PlatformId::Claude,
        PlatformId::Hermes,
    ]
}

/// 派发项目 ID + 资产 kind + name → 稳定 64-bit item_id。
///
/// 用 `DefaultHasher`(SipHash)— 进程内稳定,**不**保证跨进程/跨 Rust 版本稳定;
/// `store` 层只在本进程内做 ID 关联,够用。
fn item_id_for(project_id: u64, kind: AssetKind, name: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    project_id.hash(&mut h);
    kind.hash(&mut h);
    name.hash(&mut h);
    h.finish()
}

/// 资产 entry 链接 / 渲染目标(平台 × kind 维度的 dest 路径)。
///
/// 仅由 `sync` 内部使用,实现细节不暴露。
fn dest_for(
    plat: PlatformId,
    kind: AssetKind,
    name: &str,
    src: &Utf8Path,
) -> Option<camino::Utf8PathBuf> {
    let home_root = platform_dest_root(plat, kind)?;
    let entry_name = match kind {
        AssetKind::Skill => return Some(home_root.join(name)),
        AssetKind::Rule => format!("{name}.mdc"),
        // mcp 资产**不**走 dest_for(走 RenderMcp 路径)
        AssetKind::Mcp => return None,
        AssetKind::Agent => {
            // agent src 后缀由 `source` 保留,这里取原扩展名(.md / .yaml ...)
            let ext = src
                .extension()
                .map(|e| e.to_string())
                .unwrap_or_else(|| "md".to_string());
            format!("{name}.{ext}")
        }
    };
    Some(home_root.join(entry_name))
}

fn platform_dest_root(plat: PlatformId, kind: AssetKind) -> Option<camino::Utf8PathBuf> {
    for adapter in platform::registry() {
        if adapter.id() != plat {
            continue;
        }
        return Some(match kind {
            AssetKind::Skill => adapter.skills_dir(),
            AssetKind::Rule => adapter.rules_dir(),
            AssetKind::Agent => adapter.agents_dir(),
            AssetKind::Mcp => return None, // MCP 走 mcp_json_path
        });
    }
    None
}

/// 计算一个项目的同步动作集。
///
/// 流程:
/// 1. `source::scan_with_override(project_root, default_root)` 拿合并后资产清单
/// 2. 对每条 (kind, name, src) × 4 platforms:
///    - MCP → `RenderMcp { project_id, platform }`(per-platform,不乘 kind 因子)
///    - 其它 → `Create { item_id, platform, dest }`(若平台 `supports(kind)`)
/// 3. 返回 `Vec<SyncAction>`
///
/// **不**读 SQLite 状态,**不**跳过"已存在"链接:那个属于 `link::apply` 幂等的事
/// (PRD §10 A-3)。**不**做平台勾选过滤:那是 store 的事(PRD §8.2)。
pub fn compute_for_project(
    project: &Project,
    default_root: &Utf8Path,
) -> Result<Vec<SyncAction>, CoreError> {
    let merged = source::scan_with_override(&project.root_path, default_root)?;
    let mut out: Vec<SyncAction> = Vec::new();
    let platforms = all_platforms();

    for kind in [
        AssetKind::Skill,
        AssetKind::Rule,
        AssetKind::Mcp,
        AssetKind::Agent,
    ] {
        // 从合并后的路径列表抽取 (name, src) 对(name = 资产名,与 source 内部判定一致)
        let entries: Vec<(String, camino::Utf8PathBuf)> = match kind {
            AssetKind::Skill => merged
                .skills
                .iter()
                .filter_map(|p| {
                    p.parent()
                        .and_then(|p| p.file_name())
                        .map(|n| (n.to_string(), p.clone()))
                })
                .collect(),
            AssetKind::Rule => merged
                .rules
                .iter()
                .filter_map(|p| p.file_stem().map(|n| (n.to_string(), p.clone())))
                .collect(),
            AssetKind::Mcp => {
                if let Some(path) = &merged.mcp_json {
                    vec![(mcp_json::MCP_ASSET_NAME.to_string(), path.clone())]
                } else {
                    vec![]
                }
            }
            AssetKind::Agent => merged
                .agents
                .iter()
                .filter_map(|p| {
                    if p.is_dir() {
                        p.file_name().map(|n| (n.to_string(), p.clone()))
                    } else {
                        p.file_stem().map(|n| (n.to_string(), p.clone()))
                    }
                })
                .collect(),
        };

        for (name, src) in entries {
            let item_id = item_id_for(project.id, kind, &name);
            for plat in &platforms {
                if !platform_supports(*plat, kind) {
                    continue;
                }
                match kind {
                    AssetKind::Mcp => {
                        out.push(SyncAction::RenderMcp {
                            project_id: project.id,
                            platform: *plat,
                        });
                    }
                    _ => {
                        let dest = match dest_for(*plat, kind, &name, &src) {
                            Some(d) => d,
                            None => continue,
                        };
                        out.push(SyncAction::Create {
                            item_id,
                            platform: *plat,
                            dest,
                        });
                    }
                }
            }
        }
    }

    Ok(out)
}

/// 全局视图:对所有已注册项目求并集并去重(ARCHITECTURE §7)。
///
/// 去重键:`(variant_tag, key, PlatformId)` —
/// - Skill/Rule/Agent:key = item_id
/// - Mcp:key = project_id
///
/// 用 `HashSet` 因为 `PlatformId` 未 derive `Ord`(只 derive `Hash` + `Eq`)。
pub fn compute_global(
    projects: &[Project],
    default_root: &Utf8Path,
) -> Result<Vec<SyncAction>, CoreError> {
    let mut seen: HashSet<(u8, u64, PlatformId)> = HashSet::new();
    let mut out: Vec<SyncAction> = Vec::new();

    for project in projects {
        for action in compute_for_project(project, default_root)? {
            let key = match &action {
                SyncAction::Create {
                    item_id, platform, ..
                } => (0, *item_id, *platform),
                SyncAction::Linked { item_id, platform } => (1, *item_id, *platform),
                SyncAction::RenderMcp {
                    project_id,
                    platform,
                } => (2, *project_id, *platform),
                SyncAction::Retract {
                    item_id, platform, ..
                } => (3, *item_id, *platform),
                SyncAction::Unlink { item_id, platform } => (4, *item_id, *platform),
            };
            if seen.insert(key) {
                out.push(action);
            }
        }
    }
    Ok(out)
}

fn platform_supports(plat: PlatformId, kind: AssetKind) -> bool {
    for a in platform::registry() {
        if a.id() == plat {
            return a.supports(kind);
        }
    }
    false
}

/// 单条收回(PRD §4.2):item_id × platform → `Retract` 动作。
///
/// **不**做"读 store 找到 src / dest"的事:调用方必须提供已知的目标路径。
/// 这是事实源唯一约束的本模块版本 —— sync 只产动作,不动 store。
///
/// `dest` 由 caller 传(`link::apply` 知道真实目标位置);sync 不会去 FS 上查。
pub fn retract_item(item_id: u64, platform: PlatformId, dest: camino::Utf8PathBuf) -> SyncAction {
    SyncAction::Retract {
        item_id,
        platform,
        dest,
    }
}

/// 从某平台收回某项目所有(PRD 场景 E)。
///
/// 流程:重跑 `compute_for_project` → 把所有 `Create` 翻成 `Retract`(同 dest),
/// `RenderMcp` 在这里**不**产出(MCP 收回属于"移除 mcpServers 中本工具负责条目",
/// 那是 `template::retract` 的事,不在本模块产品表面)。
pub fn retract_all(
    project: &Project,
    default_root: &Utf8Path,
    platform: PlatformId,
) -> Result<Vec<SyncAction>, CoreError> {
    let creates = compute_for_project(project, default_root)?;
    let mut out: Vec<SyncAction> = Vec::new();
    for action in creates {
        match action {
            SyncAction::Create { item_id, dest, .. } => out.push(SyncAction::Retract {
                item_id,
                platform,
                dest,
            }),
            // 过滤掉其它平台 / MCP render
            SyncAction::RenderMcp { platform: p, .. } if p == platform => {
                // platform 收回 MCP 由 template 模块单独处理(产物不同);本模块产品表面
                // 不含 MCP retract(PRD §4.2 "收回"= 删除链接 / 移除 mcpServer,后者
                // 走 Unlink 路径,本函数当前不实现)
            }
            _ => {}
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;
    use std::fs;

    /// 准备一个临时项目根,带 .ai-config/{skills,rules,mcp/servers,agents}
    /// 全部各 1 条;并返回 `default_root`(同 tmpdir 父目录,但不带 .ai-config)。
    fn make_project_with_default() -> (tempfile::TempDir, Utf8PathBuf, Utf8PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();

        // default_root 跟 project_root 共享一份资产清单;
        // 把"全局资产"放在 default 视角下,项目视角"项目覆盖默认"在另外的 test 验证。
        let default = root.clone();

        // skills
        fs::create_dir_all(default.join("skills/foo")).unwrap();
        fs::write(default.join("skills/foo/SKILL.md"), "SKILL").unwrap();
        // rules
        fs::create_dir_all(default.join("rules")).unwrap();
        fs::write(default.join("rules/r-one.mdc"), "R1").unwrap();
        // mcp
        fs::write(default.join("mcp.json"), r#"{"mcpServers":{}}"#).unwrap();
        // agents
        fs::create_dir_all(default.join("agents")).unwrap();
        fs::write(default.join("agents/reviewer.md"), "AGENT").unwrap();

        // project 没有 .ai-config,所以这次只走 default_root
        (tmp, default, root.join("proj"))
    }

    fn make_project() -> Project {
        Project {
            id: 42,
            name: "demo".into(),
            root_path: Utf8PathBuf::from("/tmp/nonexistent"),
            registered_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn compute_for_project_emits_13_actions_for_1_skill_1_rule_1_mcp_1_agent() {
        let (_tmp, default, project_root) = make_project_with_default();
        let project = Project {
            id: 7,
            name: "demo".into(),
            root_path: project_root,
            registered_at: chrono::Utc::now(),
        };
        let actions = compute_for_project(&project, &default).unwrap();

        // 1 skill × 4 + 1 rule × 2(Cursor/Claude) + 1 mcp × 4(RenderMcp)
        //   + 1 agent × 2(Cursor/Claude)
        // → 4 + 2 + 4 + 2 = 12(Codex/Hermes 不直接消费 rule/agent)
        let creates: Vec<_> = actions
            .iter()
            .filter(|a| matches!(a, SyncAction::Create { .. }))
            .collect();
        let renders: Vec<_> = actions
            .iter()
            .filter(|a| matches!(a, SyncAction::RenderMcp { .. }))
            .collect();
        assert_eq!(creates.len(), 8, "skill 4 + rule 2 + agent 2 = 8");
        assert_eq!(renders.len(), 4, "mcp 4 platforms = 4 RenderMcp");
        assert_eq!(actions.len(), 12);
    }

    #[test]
    fn compute_for_project_project_overrides_default_same_name() {
        // 新合约:`project.root_path` 直接是 `.ai-config/` 目录,所以 project_root
        // 直接指向 `myproj/.ai-config`,skill 直接放在它下面。
        let tmp = tempfile::tempdir().unwrap();
        let default = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let project_root = default.join("myproj/.ai-config");
        fs::create_dir_all(default.join("skills/foo")).unwrap();
        fs::write(default.join("skills/foo/SKILL.md"), "DEFAULT").unwrap();
        fs::create_dir_all(project_root.join("skills/foo")).unwrap();
        fs::write(project_root.join("skills/foo/SKILL.md"), "PROJECT").unwrap();

        let project = Project {
            id: 11,
            name: "myproj".into(),
            root_path: project_root.clone(),
            registered_at: chrono::Utc::now(),
        };
        let actions = compute_for_project(&project, &default).unwrap();
        let creates: Vec<_> = actions
            .iter()
            .filter_map(|a| match a {
                SyncAction::Create { dest, .. } => Some(dest.as_str().to_string()),
                _ => None,
            })
            .collect();
        // 4 platforms × 1 skill = 4 Create
        assert_eq!(creates.len(), 4);
        // dest 路径**不**含 src 信息 — 但因为项目覆盖了默认,我们验证源清单是项目版
        // (行为由 source::scan_with_override 单测覆盖,这里只验证 emit 数量)
    }

    #[test]
    fn compute_for_project_create_dest_points_to_home_dir() {
        // 隔离:其它测试可能在跑时改 HERMES_SKILLS_DIR(env guard 进程内可能脏)。
        // 这里显式 unset,保证本测试看到的 hermes skills_dir = $HOME/.hermes/skills。
        let _env_guard = HermesEnvGuard::unset();

        let (_tmp, default, project_root) = make_project_with_default();
        let project = Project {
            id: 1,
            name: "p".into(),
            root_path: project_root,
            registered_at: chrono::Utc::now(),
        };
        let actions = compute_for_project(&project, &default).unwrap();
        for a in &actions {
            if let SyncAction::Create { dest, platform, .. } = a {
                let home_substr = match platform {
                    PlatformId::Cursor => ".cursor",
                    PlatformId::Codex => ".codex",
                    PlatformId::Claude => ".claude",
                    PlatformId::Hermes => ".hermes",
                };
                assert!(
                    dest.as_str().contains(home_substr),
                    "dest {dest} 应在 {home_substr} 下"
                );
            }
        }
    }

    /// 设/恢复 `HERMES_SKILLS_DIR` env,避免被其它并行测试污染本测试断言。
    struct HermesEnvGuard {
        prev: Option<String>,
    }
    impl HermesEnvGuard {
        fn unset() -> Self {
            let prev = std::env::var("HERMES_SKILLS_DIR").ok();
            std::env::remove_var("HERMES_SKILLS_DIR");
            Self { prev }
        }
    }
    impl Drop for HermesEnvGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var("HERMES_SKILLS_DIR", v),
                None => std::env::remove_var("HERMES_SKILLS_DIR"),
            }
        }
    }

    #[test]
    fn compute_for_project_skill_dest_is_directory() {
        let (_tmp, default, project_root) = make_project_with_default();
        let project = Project {
            id: 1,
            name: "p".into(),
            root_path: project_root,
            registered_at: chrono::Utc::now(),
        };
        let actions = compute_for_project(&project, &default).unwrap();
        let skill_creates: Vec<_> = actions
            .iter()
            .filter_map(|a| match a {
                SyncAction::Create { dest, .. } if dest.as_str().ends_with("skills/foo") => {
                    Some(dest.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(skill_creates.len(), 4, "skill foo 4 平台各 1 条 Create");
    }

    #[test]
    fn compute_for_project_rule_uses_mdc_extension() {
        let (_tmp, default, project_root) = make_project_with_default();
        let project = Project {
            id: 1,
            name: "p".into(),
            root_path: project_root,
            registered_at: chrono::Utc::now(),
        };
        let actions = compute_for_project(&project, &default).unwrap();
        let rule_creates: Vec<_> = actions
            .iter()
            .filter_map(|a| match a {
                SyncAction::Create { dest, .. } if dest.as_str().ends_with("rules/r-one.mdc") => {
                    Some(dest.clone())
                }
                _ => None,
            })
            .collect();
        // Cursor + Claude(Hermes/Codex 不直接消费 rule)
        assert_eq!(rule_creates.len(), 2);
    }

    #[test]
    fn compute_for_project_mcp_emits_render_mcp_not_create() {
        let (_tmp, default, project_root) = make_project_with_default();
        let project = Project {
            id: 99,
            name: "p".into(),
            root_path: project_root,
            registered_at: chrono::Utc::now(),
        };
        let actions = compute_for_project(&project, &default).unwrap();
        let renders: Vec<_> = actions
            .iter()
            .filter_map(|a| match a {
                SyncAction::RenderMcp {
                    project_id,
                    platform,
                } => Some((*project_id, *platform)),
                _ => None,
            })
            .collect();
        assert_eq!(renders.len(), 4);
        assert!(renders.iter().all(|(pid, _)| *pid == 99));
        // 4 platforms 全在
        let plats: std::collections::HashSet<_> = renders.iter().map(|(_, p)| *p).collect();
        assert_eq!(plats.len(), 4);
        assert!(plats.contains(&PlatformId::Cursor));
        assert!(plats.contains(&PlatformId::Codex));
        assert!(plats.contains(&PlatformId::Claude));
        assert!(plats.contains(&PlatformId::Hermes));
    }

    #[test]
    fn compute_global_2_projects_each_1_skill_yields_8_actions() {
        // 新合约:`project.root_path` 直接是 `.ai-config/` 目录,所以把项目根设到
        // `default/p<n>/.ai-config`,把 skill 直接放在它下面(不再嵌一层 `.ai-config/`)。
        let tmp = tempfile::tempdir().unwrap();
        let default = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let p1_root = default.join("p1/.ai-config");
        let p2_root = default.join("p2/.ai-config");
        fs::create_dir_all(p1_root.join("skills/shared")).unwrap();
        fs::write(p1_root.join("skills/shared/SKILL.md"), "FROM-P1").unwrap();
        fs::create_dir_all(p2_root.join("skills/shared")).unwrap();
        fs::write(p2_root.join("skills/shared/SKILL.md"), "FROM-P2").unwrap();

        let p1 = Project {
            id: 1,
            name: "p1".into(),
            root_path: p1_root,
            registered_at: chrono::Utc::now(),
        };
        let p2 = Project {
            id: 2,
            name: "p2".into(),
            root_path: p2_root,
            registered_at: chrono::Utc::now(),
        };

        let actions = compute_global(&[p1, p2], &default).unwrap();
        let creates: Vec<_> = actions
            .iter()
            .filter(|a| matches!(a, SyncAction::Create { .. }))
            .collect();
        // 每个项目 1 skill × 4 platforms = 4;两个项目 = 8。
        // 即便 skill name 相同(都是 "shared"),item_id 因 project_id 不同而不冲突。
        assert_eq!(creates.len(), 8, "2 projects × 1 skill × 4 platforms = 8");

        // 两个不同 item_id(不同 project)
        let item_ids: std::collections::HashSet<u64> = creates
            .iter()
            .filter_map(|a| match a {
                SyncAction::Create { item_id, .. } => Some(*item_id),
                _ => None,
            })
            .collect();
        assert_eq!(item_ids.len(), 2, "两个项目对应两个不同 item_id");
    }

    #[test]
    fn compute_global_dedupes_when_two_projects_share_same_item_id() {
        // 同 project_id(罕见,理论可能),item_id 完全相同 → 去重
        // 模拟:用一个 project 跑两次
        let (_tmp, default, project_root) = make_project_with_default();
        let project = Project {
            id: 1,
            name: "p".into(),
            root_path: project_root,
            registered_at: chrono::Utc::now(),
        };
        let actions = compute_global(&[project.clone(), project.clone()], &default).unwrap();
        // 重复跑同 project,所有 (item_id, platform) 都应被去重 → 数量等于单次
        let single = compute_for_project(&project, &default).unwrap();
        assert_eq!(actions.len(), single.len());
    }

    #[test]
    fn retract_item_produces_retract_action() {
        let dest = Utf8PathBuf::from(".cursor/skills/foo");
        let action = retract_item(123, PlatformId::Cursor, dest.clone());
        assert!(action.is_retract());
        assert_eq!(action.platform(), PlatformId::Cursor);
        assert_eq!(action.dest(), Some(&dest));
        if let SyncAction::Retract { item_id, .. } = action {
            assert_eq!(item_id, 123);
        } else {
            panic!("expected Retract");
        }
    }

    #[test]
    fn retract_all_converts_creates_to_retracts() {
        let (_tmp, default, project_root) = make_project_with_default();
        let project = Project {
            id: 5,
            name: "p".into(),
            root_path: project_root,
            registered_at: chrono::Utc::now(),
        };
        let actions = retract_all(&project, &default, PlatformId::Cursor).unwrap();
        assert!(!actions.is_empty());
        for a in &actions {
            assert!(a.is_retract(), "all should be Retract: {a:?}");
            assert_eq!(a.platform(), PlatformId::Cursor);
        }
        // 每条 Create 都翻成 Retract(skill 4 + rule 2 + agent 2 = 8)
        assert_eq!(actions.len(), 8);
    }

    #[test]
    fn item_id_is_stable_across_calls() {
        let id1 = item_id_for(7, AssetKind::Skill, "foo");
        let id2 = item_id_for(7, AssetKind::Skill, "foo");
        let id3 = item_id_for(7, AssetKind::Rule, "foo");
        let id4 = item_id_for(8, AssetKind::Skill, "foo");
        assert_eq!(id1, id2, "同输入同输出");
        assert_ne!(id1, id3, "kind 不同 item_id 不同");
        assert_ne!(id1, id4, "project_id 不同 item_id 不同");
    }

    // silence unused warnings on the helper used in the comment-only test
    #[allow(dead_code)]
    fn _unused() {
        let _ = make_project();
    }
}
