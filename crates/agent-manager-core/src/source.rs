//! 资产源解析:扫 `skills/ rules/ mcp.json agents/` → 资产清单。
//!
//! 严格对齐 PRD §2 / §3.1 / §3.2 / §11.2:
//! - **单一规则源**:`rules/*.mdc` / `*.md` 不按平台分目录(§11.2)
//! - **视图合并**:项目 `.agents-manager/` 同名覆盖、异名附加(§3.2)
//! - **五类资产** = skills(目录) / rules(单文件) / commands(单文件) / mcp(单文件) / agents(文件或目录)(§2)
//!
//! 过滤规则:隐藏文件(`.` 开头)、macOS / Windows 噪声(`.DS_Store` / `Thumbs.db`)、
//! 备份(`*.bak` / `*.orig` / `*.swp` / `*~` / `*.tmp`)、构建产物目录
//! (`node_modules` / `target` / `dist` / `build`)一律不进清单。
//!
//! Phase 1 实装:walkdir 扫 4 目录,返回 `Utf8PathBuf` 列表;
//! Phase 2 起 `sync` 模块消费它,产出 `SyncAction`。

use camino::{Utf8Path, Utf8PathBuf};
use walkdir::WalkDir;

use crate::error::CoreError;
use crate::mcp_json;

/// 一次扫描的结果(单根目录视角,或合并后的双根视角)。
///
/// 4 个字段对应 PRD §2 的 4 类资产;每条 entry 都是"源文件 / 源目录"绝对路径,
/// 便于后续模块读 frontmatter、渲染 mcp.json、建链接。
///
/// **顺序**:`merge_override` 保留 default 先入、project 后入 + 同 name 覆盖的
/// 合并顺序(测试断言稳定)。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ScanResult {
    /// 每个元素是 `skills/<name>/SKILL.md`
    pub skills: Vec<Utf8PathBuf>,
    /// 每个元素是 `rules/<name>.mdc` 或 `rules/<name>.md`
    pub rules: Vec<Utf8PathBuf>,
    /// 合并后的 `mcp.json` 路径(存在时)
    pub mcp_json: Option<Utf8PathBuf>,
    /// 每个元素是 `agents/<name>/`(目录)或 `agents/<name>.{md,yaml,json}`(单文件)
    pub agents: Vec<Utf8PathBuf>,
    /// 每个元素是 `commands/<name>.md`
    pub commands: Vec<Utf8PathBuf>,
    /// 每个元素是 `hooks.json` 中一条生命周期绑定。
    pub hooks: Vec<crate::hook::HookListItem>,
}

// ── 过滤规则 ──────────────────────────────────────────────────────

/// 文件 / 目录名是否在"绝对不进清单"的黑名单里。
///
/// - 首字符 `.` → 隐藏(Git / macOS / IDE 都会扔)
/// - `Thumbs.db` / `Desktop.ini` / `node_modules` / `target` / `dist` / `build` → 噪声 / 构建产物
fn is_excluded_name(name: &str) -> bool {
    if name.starts_with('.') {
        return true;
    }
    // README/README.* 仅用于说明文档，不应被识别为资产（rules/commands/agents 常见）
    if is_readme_name(name) {
        return true;
    }
    matches!(
        name,
        "Thumbs.db" | "Desktop.ini" | "node_modules" | "target" | "dist" | "build"
    )
}

fn is_readme_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name);
    stem.eq_ignore_ascii_case("README")
}

/// 文件名后缀是否在备份 / 临时文件白名单里。
fn is_backup_name(name: &str) -> bool {
    name.ends_with(".bak")
        || name.ends_with(".orig")
        || name.ends_with(".swp")
        || name.ends_with("~")
        || name.ends_with(".tmp")
        || name.ends_with(".agents-manager-deploy.json")
}

/// 综合判断"这个 entry 是不是该进清单"。
fn is_excluded(name: &str) -> bool {
    is_excluded_name(name) || is_backup_name(name)
}

// ── 单目录扫描:skills / rules / mcp / agents ──────────────────────

/// 扫 `<root>/skills/<name>/SKILL.md`,返回 SKILL.md 路径列表。
///
/// 规则:仅直接子目录(深度 2:`<root>/skills/<name>/`);每个目录必含 `SKILL.md` 才算一条
/// skill(避免把"半截"目录当资产)。SKILL.md 缺失的子目录不报错,只是忽略。
fn scan_skills(root: &Utf8Path) -> Result<Vec<Utf8PathBuf>, CoreError> {
    let skills_dir = root.join("skills");
    if !skills_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for entry in WalkDir::new(&skills_dir)
        .min_depth(1)
        .max_depth(1)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
    {
        let Some(name) = entry.file_name().to_str() else {
            continue;
        };
        if is_excluded(name) {
            continue;
        }
        let entry_path = skills_dir.join(name);
        if !entry_path.is_dir() {
            continue;
        }
        let skill_md = entry_path.join("SKILL.md");
        if skill_md.is_file() {
            out.push(skill_md);
        }
    }
    Ok(out)
}

/// 扫 `<root>/rules/<name>.mdc` 或 `<name>.md`。
///
/// 严格按 PRD §11.2:rule **无** scope 字段,不分平台目录,
/// 因此 `rules/cursor/...` 之类**不是**本工具的资产(那是上游 IDE 自己的目录),
/// 不会出现在本层结果里。
fn scan_rules(root: &Utf8Path) -> Result<Vec<Utf8PathBuf>, CoreError> {
    let rules_dir = root.join("rules");
    if !rules_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for entry in WalkDir::new(&rules_dir)
        .min_depth(1)
        .max_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str() else {
            continue;
        };
        if is_excluded(name) {
            continue;
        }
        if name.ends_with(".mdc") || name.ends_with(".md") {
            out.push(rules_dir.join(name));
        }
    }
    Ok(out)
}

/// 解析 `<root>/mcp.json`。
fn scan_mcp_json(root: &Utf8Path) -> Result<Option<Utf8PathBuf>, CoreError> {
    let path = mcp_json::mcp_json_path(root);
    if path.is_file() {
        Ok(Some(path))
    } else {
        Ok(None)
    }
}

/// 扫 `<root>/agents/<name>/`(目录)或 `<name>.{md,yaml,json}`(单文件)。
///
/// PRD §2.2:平台对 agent 叫法不同,这里只关心"有几条 / 叫什么",不区分平台。
fn scan_agents(root: &Utf8Path) -> Result<Vec<Utf8PathBuf>, CoreError> {
    let agents_dir = root.join("agents");
    if !agents_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for entry in WalkDir::new(&agents_dir)
        .min_depth(1)
        .max_depth(1)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
    {
        let Some(name) = entry.file_name().to_str() else {
            continue;
        };
        if is_excluded(name) {
            continue;
        }
        let entry_path = agents_dir.join(name);
        if entry_path.is_dir() {
            out.push(entry_path);
            continue;
        }
        if entry_path.is_file()
            && (name.ends_with(".md") || name.ends_with(".yaml") || name.ends_with(".json"))
        {
            out.push(entry_path);
        }
    }
    Ok(out)
}

/// 扫 `<root>/commands/<name>.md`（Cursor / Claude 斜杠命令源）。
fn scan_commands(root: &Utf8Path) -> Result<Vec<Utf8PathBuf>, CoreError> {
    let commands_dir = root.join("commands");
    if !commands_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for entry in WalkDir::new(&commands_dir)
        .min_depth(1)
        .max_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str() else {
            continue;
        };
        if is_excluded(name) {
            continue;
        }
        if name.ends_with(".md") {
            out.push(commands_dir.join(name));
        }
    }
    Ok(out)
}

/// 扫 `<root>/hooks.json` 与 `hooks/` 目录，返回去重后的 command 型绑定项（供 doctor/sync 兼容）。
fn scan_hooks(root: &Utf8Path) -> Result<Vec<crate::hook::HookListItem>, CoreError> {
    let manifest = root.join(crate::hook::HOOKS_MANIFEST);
    Ok(crate::hook::list_hook_catalog(root)?
        .into_iter()
        .filter(|item| item.hook_type == crate::hook::HookEntryType::Command)
        .map(|item| crate::hook::HookListItem {
            lifecycle: String::new(),
            script_filename: item.binding_key,
            script_path: item.source_path,
            manifest_path: manifest.clone(),
        })
        .collect())
}

// ── 公共 API ──────────────────────────────────────────────────────

/// 扫描单个根目录下的 4 类资产。
///
/// `root` 通常是 `<project>/.agents-manager/` 或 `~/.agents-manager/`(全局资产根)。
/// 不存在的子目录(缺 `skills/` / `mcp/servers/` 等)按"空清单"返回,**不**报错;
/// `root` 本身不存在才报 `InvalidPath`。
pub fn scan_project_root(root: &Utf8Path) -> Result<ScanResult, CoreError> {
    if !root.exists() {
        return Err(CoreError::InvalidPath(format!("资产根目录不存在: {root}")));
    }

    Ok(ScanResult {
        skills: scan_skills(root)?,
        rules: scan_rules(root)?,
        mcp_json: scan_mcp_json(root)?,
        agents: scan_agents(root)?,
        commands: scan_commands(root)?,
        hooks: scan_hooks(root)?,
    })
}

/// 合并扫描:项目 `.agents-manager/` 覆盖 agents-manager 默认仓(同名覆盖,异名附加,PRD §3.2)。
///
/// - `project_root`:项目下 `.agents-manager/` 目录路径
/// - `default_root`:全局资产根(`~/.agents-manager/` 或测试用临时目录)
/// - 哪边不存在就只取另一边(不报错,空清单即可)
pub fn scan_with_override(
    project_root: &Utf8Path,
    default_root: &Utf8Path,
) -> Result<ScanResult, CoreError> {
    let project = if project_root.exists() {
        scan_project_root(project_root)?
    } else {
        ScanResult::default()
    };
    let default = if default_root.exists() {
        scan_project_root(default_root)?
    } else {
        ScanResult::default()
    };

    Ok(ScanResult {
        skills: merge_override(&default.skills, &project.skills, |p| {
            // skills/<...>/SKILL.md → 取 <name>
            p.parent().and_then(|p| p.file_name()).map(str::to_owned)
        }),
        rules: merge_override(&default.rules, &project.rules, |p| {
            // rules/<name>.mdc / .md → 去扩展名
            p.file_stem().map(str::to_owned)
        }),
        mcp_json: project.mcp_json.or(default.mcp_json),
        agents: merge_override(&default.agents, &project.agents, |p| {
            // agents/<name>/ 目录 → 取 <name>
            // agents/<name>.md 单文件 → 去扩展名
            if p.is_dir() {
                p.file_name().map(str::to_owned)
            } else {
                p.file_stem().map(str::to_owned)
            }
        }),
        commands: merge_override(&default.commands, &project.commands, |p| {
            p.file_stem().map(str::to_owned)
        }),
        hooks: merge_hook_items(&default.hooks, &project.hooks),
    })
}

fn merge_hook_items(
    default: &[crate::hook::HookListItem],
    project: &[crate::hook::HookListItem],
) -> Vec<crate::hook::HookListItem> {
    let mut out = default.to_vec();
    for item in project {
        let key = format!("{}:{}", item.lifecycle, item.script_filename);
        if let Some(idx) = out
            .iter()
            .position(|x| format!("{}:{}", x.lifecycle, x.script_filename) == key)
        {
            out[idx] = item.clone();
        } else {
            out.push(item.clone());
        }
    }
    out.sort_by(|a, b| (&a.lifecycle, &a.script_filename).cmp(&(&b.lifecycle, &b.script_filename)));
    out
}

/// `default` 先入,`override` 后入;同 key 时**后者**赢(项目覆盖默认)。
///
/// 用 `Vec` 保留扫描顺序(测试断言稳定);同一资产名仅首次出现的路径留下。
fn merge_override<F>(
    default: &[Utf8PathBuf],
    project: &[Utf8PathBuf],
    key_of: F,
) -> Vec<Utf8PathBuf>
where
    F: Fn(&Utf8Path) -> Option<String>,
{
    let mut out: Vec<Utf8PathBuf> = Vec::with_capacity(default.len() + project.len());
    let mut seen: Vec<String> = Vec::with_capacity(default.len() + project.len());

    for p in default {
        if let Some(k) = key_of(p) {
            out.push(p.clone());
            seen.push(k);
        }
    }
    for p in project {
        if let Some(k) = key_of(p) {
            if let Some(idx) = seen.iter().position(|x| x == &k) {
                out[idx] = p.clone(); // 覆盖
            } else {
                out.push(p.clone());
                seen.push(k);
            }
        }
    }
    out
}

// ── 单元测试 ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// 写一个空文件,确保父目录已建。
    fn touch(path: &Utf8Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(path, b"").expect("write file");
    }

    /// 写一个空目录。
    fn mkdir(path: &Utf8Path) {
        fs::create_dir_all(path).expect("create dir");
    }

    // ── 过滤规则 ──

    #[test]
    fn hidden_and_backup_names_are_excluded() {
        // 隐藏
        assert!(is_excluded(".DS_Store"));
        assert!(is_excluded(".git"));
        assert!(is_excluded(".hidden-rule.mdc"));
        // README
        assert!(is_excluded("README"));
        assert!(is_excluded("README.md"));
        assert!(is_excluded("readme.mdc"));
        // 备份
        assert!(is_excluded("foo.bak"));
        assert!(is_excluded("foo.mdc.orig"));
        assert!(is_excluded("foo.mdc~"));
        assert!(is_excluded("foo.mdc.swp"));
        assert!(is_excluded("foo.tmp"));
        // 噪声 / 构建产物
        assert!(is_excluded("node_modules"));
        assert!(is_excluded("target"));
        // 正常
        assert!(!is_excluded("SKILL.md"));
        assert!(!is_excluded("foo.mdc"));
        assert!(!is_excluded("bar.md"));
        assert!(!is_excluded("server.json"));
    }

    // ── scan_project_root:skills ──

    #[test]
    fn scan_skills_finds_two() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(tmp.path()).unwrap();

        // skills/<a>/SKILL.md
        touch(&root.join("skills").join("a").join("SKILL.md"));
        // skills/<b>/SKILL.md
        touch(&root.join("skills").join("b").join("SKILL.md"));
        // 隐藏目录不应计入
        touch(&root.join("skills").join(".hidden").join("SKILL.md"));
        // 缺 SKILL.md 的目录不计
        mkdir(&root.join("skills").join("half-baked"));
        touch(&root.join("skills").join("half-baked").join("README.md"));

        let r = scan_skills(root).expect("scan ok");
        assert_eq!(r.len(), 2, "expected exactly 2 skills, got {:?}", r);

        // 名称应该是 a 和 b
        let names: Vec<String> = r
            .iter()
            .map(|p| {
                p.parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string())
                    .unwrap()
            })
            .collect();
        assert!(names.contains(&"a".to_string()), "got: {names:?}");
        assert!(names.contains(&"b".to_string()), "got: {names:?}");
    }

    // ── scan_project_root:rules ──

    #[test]
    fn scan_rules_finds_mdc_and_md() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(tmp.path()).unwrap();

        touch(&root.join("rules").join("alibaba-java.mdc"));
        touch(&root.join("rules").join("notes.md"));
        // README 不应计入
        touch(&root.join("rules").join("README.md"));
        // 其它扩展名不计入
        touch(&root.join("rules").join("random.txt"));
        // 备份不计
        touch(&root.join("rules").join("foo.mdc.bak"));
        // 隐藏不计
        touch(&root.join("rules").join(".hidden.mdc"));
        // 子目录不算(PRD §11.2 单一源,不分平台)
        mkdir(&root.join("rules").join("cursor"));

        let r = scan_rules(root).expect("scan ok");
        assert_eq!(r.len(), 2, "expected exactly 2 rules, got {:?}", r);
        for p in &r {
            let s = p.as_str();
            assert!(
                s.ends_with("alibaba-java.mdc") || s.ends_with("notes.md"),
                "unexpected rule: {p}"
            );
        }
    }

    // ── scan_project_root:mcp ──

    #[test]
    fn scan_mcp_json_finds_file() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(tmp.path()).unwrap();
        touch(&root.join("mcp.json"));
        let r = scan_mcp_json(root).expect("scan ok");
        assert_eq!(r.as_ref().map(|p| p.file_name()), Some(Some("mcp.json")));
    }

    // ── scan_project_root:agents ──

    #[test]
    fn scan_agents_finds_dir_and_file() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(tmp.path()).unwrap();

        // 目录形态
        mkdir(&root.join("agents").join("code-reviewer"));
        touch(&root.join("agents").join("code-reviewer").join("AGENT.md"));
        // 单文件形态
        touch(&root.join("agents").join("data-explorer.md"));
        touch(&root.join("agents").join("hermes-bot.yaml"));
        // deploy marker（文件型下发的旁路标记）不应被识别为 agent
        touch(
            &root
                .join("agents")
                .join("backend-java-dev.md.agents-manager-deploy.json"),
        );
        // README 不应计入（无论目录还是文件）
        mkdir(&root.join("agents").join("README"));
        touch(&root.join("agents").join("README.md"));
        // 其它扩展名不计
        touch(&root.join("agents").join("notes.txt"));
        // 隐藏不计
        mkdir(&root.join("agents").join(".private"));

        let r = scan_agents(root).expect("scan ok");
        assert_eq!(r.len(), 3, "expected 3 agents, got {:?}", r);
    }

    #[test]
    fn scan_commands_ignores_readme() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(tmp.path()).unwrap();

        touch(&root.join("commands").join("hello.md"));
        touch(&root.join("commands").join("README.md"));

        let r = scan_commands(root).expect("scan ok");
        assert_eq!(r.len(), 1);
        assert!(r[0].as_str().ends_with("hello.md"));
    }

    // ── scan_project_root:不存在的根 → InvalidPath ──

    #[test]
    fn scan_with_nonexistent_root_errors() {
        let tmp = TempDir::new().unwrap();
        let missing = Utf8Path::from_path(tmp.path())
            .unwrap()
            .join("definitely-not-here");
        let r = scan_project_root(&missing);
        assert!(r.is_err(), "expected error for missing root");
    }

    // ── scan_project_root:缺子目录(空根)→ 空清单 ──

    #[test]
    fn scan_empty_root_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(tmp.path()).unwrap();
        let r = scan_project_root(root).expect("scan ok");
        assert!(r.skills.is_empty());
        assert!(r.rules.is_empty());
        assert!(r.mcp_json.is_none());
        assert!(r.agents.is_empty());
        assert!(r.commands.is_empty());
    }

    // ── scan_with_override:同名覆盖 / 异名附加 ──

    #[test]
    fn override_same_name_replaces_default() {
        let default = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        let d = Utf8Path::from_path(default.path()).unwrap();
        let p = Utf8Path::from_path(project.path()).unwrap();

        // 默认: skills/a + skills/x
        touch(&d.join("skills").join("a").join("SKILL.md"));
        touch(&d.join("skills").join("x").join("SKILL.md"));
        // 项目: skills/a(覆盖) + skills/b(附加)
        touch(&p.join("skills").join("a").join("SKILL.md"));
        touch(&p.join("skills").join("b").join("SKILL.md"));

        let r = scan_with_override(p, d).expect("scan ok");
        // 共 3 条:a(被覆盖,取项目) + x(仅默认) + b(仅项目)
        assert_eq!(r.skills.len(), 3, "got: {:?}", r.skills);

        // a 应该来自项目(路径前缀含 project.path())
        let project_prefix = project.path().to_str().unwrap();
        let a_entry = r
            .skills
            .iter()
            .find(|p| {
                p.parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n == "a")
                    .unwrap_or(false)
            })
            .expect("a must exist");
        assert!(
            a_entry.as_str().starts_with(project_prefix),
            "a 应该来自项目: got {a_entry}"
        );

        // x 只在默认里,应该保留
        assert!(r.skills.iter().any(|p| {
            p.parent()
                .and_then(|p| p.file_name())
                .map(|n| n == "x")
                .unwrap_or(false)
        }));

        // b 只在项目里,应该附加
        assert!(r.skills.iter().any(|p| {
            p.parent()
                .and_then(|p| p.file_name())
                .map(|n| n == "b")
                .unwrap_or(false)
        }));
    }

    #[test]
    fn override_only_default_returns_default() {
        // 项目目录不存在 → 只取默认
        let default = TempDir::new().unwrap();
        let d = Utf8Path::from_path(default.path()).unwrap();
        touch(&d.join("skills").join("a").join("SKILL.md"));

        let missing = d.join("definitely-not-here");
        let r = scan_with_override(&missing, d).expect("scan ok");
        assert_eq!(r.skills.len(), 1, "got: {:?}", r.skills);
    }

    #[test]
    fn override_only_project_returns_project() {
        // 默认本仓库不存在 → 只取项目
        let project = TempDir::new().unwrap();
        let p = Utf8Path::from_path(project.path()).unwrap();
        touch(&p.join("rules").join("a.mdc"));

        let no_default_dir = TempDir::new().unwrap();
        let missing = no_default_dir.path().join("no-default");
        let missing = Utf8Path::from_path(&missing).unwrap();
        let r = scan_with_override(p, missing).expect("scan ok");
        assert_eq!(r.rules.len(), 1, "got: {:?}", r.rules);
    }

    #[test]
    fn override_applies_to_all_four_kinds() {
        let default = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        let d = Utf8Path::from_path(default.path()).unwrap();
        let p = Utf8Path::from_path(project.path()).unwrap();

        // 默认:4 类各 1 条
        touch(&d.join("skills").join("s1").join("SKILL.md"));
        touch(&d.join("rules").join("r1.mdc"));
        touch(&d.join("mcp.json"));
        mkdir(&d.join("agents").join("a1"));
        touch(&d.join("agents").join("a1").join("AGENT.md"));

        // 项目:全部覆盖(同名)
        touch(&p.join("skills").join("s1").join("SKILL.md"));
        touch(&p.join("rules").join("r1.mdc"));
        touch(&p.join("mcp.json"));
        mkdir(&p.join("agents").join("a1"));
        touch(&p.join("agents").join("a1").join("AGENT.md"));

        let r = scan_with_override(p, d).expect("scan ok");
        let project_prefix = project.path().to_str().unwrap();
        assert_eq!(r.skills.len(), 1);
        assert_eq!(r.rules.len(), 1);
        assert_eq!(r.agents.len(), 1);
        assert!(r
            .mcp_json
            .as_ref()
            .unwrap()
            .as_str()
            .starts_with(project_prefix));
        for entry in r.skills.iter().chain(r.rules.iter()).chain(r.agents.iter()) {
            assert!(entry.as_str().starts_with(project_prefix));
        }
    }

    /// PRD §11.2 关键约束:rule 单一源,不分平台;`rules/cursor/` 等子目录**不**进清单。
    #[test]
    fn rules_subdirs_are_not_picked_up() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(tmp.path()).unwrap();

        // 根 rules/<name>.mdc:应计入(1 条)
        touch(&root.join("rules").join("only.mdc"));
        // 平台子目录(IDE 自己的目录):不应计入
        mkdir(&root.join("rules").join("cursor"));
        touch(&root.join("rules").join("cursor").join("foo.mdc"));
        mkdir(&root.join("rules").join("claude"));
        touch(&root.join("rules").join("claude").join("bar.mdc"));

        let r = scan_rules(root).expect("scan ok");
        assert_eq!(r.len(), 1, "only 根 rules/<name>.mdc 进清单, got {:?}", r);
    }

    #[test]
    fn scan_project_root_does_not_reconcile_orphan_hooks() {
        let tmp = TempDir::new().unwrap();
        let repo = Utf8Path::from_path(tmp.path()).unwrap().join("repo");
        let asset_root = repo.join(".agents-manager");
        touch(&asset_root.join("hooks").join("speak.py"));
        touch(&repo.join(".cursor").join("hooks").join("speak.py"));
        fs::write(
            repo.join(".cursor/hooks.json").as_std_path(),
            r#"{"version":1,"hooks":{"sessionStart":[{"command":".cursor/hooks/speak.py"}]}}"#,
        )
        .unwrap();

        scan_project_root(&asset_root).expect("scan succeeds");

        assert!(
            !asset_root.join("hooks.json").exists(),
            "read-only scan must not register hooks from platform configuration"
        );
    }
}
