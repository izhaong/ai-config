//! 平台下发：复制资产到目标路径（非 symlink），避免删源后各平台链接断裂。
//!
//! - `deploy`：目录/文件实体复制；遇历史 symlink 自动迁移
//! - `retract`：删除本工具下发的副本；兼容收回历史 symlink 与 legacy marker
//! - `check`：legacy marker / legacy symlink / **同名同内容** 判定是否已同步
//! - `copy_tree` / `resolve_copy_source`：沿 symlink 找到真实路径并做硬拷贝

use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path};

use camino::Utf8Path;
use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::error::CoreError;
use crate::link;
use crate::model::AssetKind;
use crate::path_independence;
use crate::paths;
use crate::sync::link_src_for_create;

const MARKER_NAME: &str = ".agents-manager-deploy.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeployHealth {
    Linked { src: Utf8PathBuf },
    Broken,
    Unlinked,
}

#[derive(Debug, Serialize, Deserialize)]
struct DeployMarker {
    version: u8,
    source: String,
}

fn marker_path_for_dest(dest: &Utf8Path) -> Utf8PathBuf {
    if dest.is_file() {
        Utf8PathBuf::from(format!("{dest}{MARKER_NAME}"))
    } else {
        dest.join(MARKER_NAME)
    }
}

fn read_marker(marker: &Utf8Path) -> Option<Utf8PathBuf> {
    let raw = fs::read_to_string(marker.as_std_path()).ok()?;
    let parsed: DeployMarker = serde_json::from_str(&raw).ok()?;
    if parsed.version != 1 {
        return None;
    }
    Some(Utf8PathBuf::from(parsed.source))
}

fn canonical_src(src: &Utf8Path) -> Utf8PathBuf {
    fs::canonicalize(src.as_std_path())
        .map(|p| Utf8PathBuf::from(p.to_string_lossy().into_owned()))
        .unwrap_or_else(|_| src.to_path_buf())
}

fn sources_match(a: &Utf8Path, b: &Utf8Path) -> bool {
    if a == b {
        return true;
    }
    let ca = canonical_src(a);
    let cb = canonical_src(b);
    ca == cb || ca == *b || *a == cb
}

fn is_symlink_entry(path: &Utf8Path) -> bool {
    fs::symlink_metadata(path.as_std_path())
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

fn has_copy_marker(dest: &Utf8Path) -> bool {
    if dest.is_dir() {
        dest.join(MARKER_NAME).is_file()
    } else {
        marker_path_for_dest(dest).is_file()
    }
}

/// 是否由本工具下发（`.agents-manager-deploy.json` 或 legacy symlink）。
pub fn is_managed_deploy(dest: &Utf8Path) -> bool {
    if is_symlink_entry(dest) {
        return true;
    }
    has_copy_marker(dest)
}

/// 沿 symlink 链解析到真实文件/目录（用于导入与硬拷贝源）。
pub fn resolve_copy_source(path: &Utf8Path) -> Utf8PathBuf {
    if is_symlink_entry(path) {
        if let Ok(target) = fs::read_link(path.as_std_path()) {
            let mut resolved = Utf8PathBuf::from(target.to_string_lossy().into_owned());
            if resolved.is_relative() {
                if let Some(parent) = path.parent() {
                    resolved = parent.join(resolved);
                }
            }
            return resolve_copy_source(&resolved);
        }
    }
    canonical_src(path)
}

/// 将 `src` 树实体复制到 `dest`（不保留内部 symlink，全部展开为真实文件）。
pub fn copy_tree(src: &Utf8Path, dest: &Utf8Path) -> Result<(), CoreError> {
    let material = resolve_copy_source(src);
    if material.is_file() {
        if let Some(parent) = dest.parent() {
            paths::ensure_parent_dir(parent)?;
        }
        fs::copy(material.as_std_path(), dest.as_std_path()).map_err(CoreError::Io)?;
        return Ok(());
    }
    copy_dir_materialized(&material, dest)
}

/// 幂等复制下发（目录或单文件）。历史 symlink 会在本次操作中迁移为实体副本。
///
/// **硬约束**：
/// - **永远写硬拷贝**（实体复制；不生成 symlink / junction / Unix 硬链接）。
/// - agents-manager 源（`~/.agents-manager` / `project/.agents-manager`）与各 IDE 平台目录 **互不共享 inode**；
///   删/改某一平台副本不影响源，也不影响其它平台。
/// - MCP：`mcp.json` 为单文件 JSON，下发前 `ensure_platform_mcp_independent` 断开与源的链接。
pub fn deploy(src: &Utf8Path, dest: &Utf8Path) -> Result<(), CoreError> {
    let material_src = resolve_copy_source(src);
    if material_src.as_str() == dest.as_str() {
        return Err(CoreError::InvalidPath(format!(
            "dest 与 src 为同一路径,无法下发: {dest}"
        )));
    }
    if !material_src.exists() {
        return Err(CoreError::LinkFailed {
            src: src.to_string(),
            dest: dest.to_string(),
            reason: format!("源不存在: {src}"),
            hint: "确认资产仍在 ~/.agents-manager 或项目 .agents-manager 中".to_string(),
        });
    }

    // 与源共用 inode / symlink 的旧下发：先断开再写独立副本
    if path_independence::paths_alias(dest, &material_src) {
        link::remove_dest_path(dest)?;
    }

    let canonical = canonical_src(&material_src);

    // 历史 symlink（remove 后可能已不存在）：后续仍按实体副本流程
    if is_symlink_entry(dest) {
        let _ = link::unlink(dest);
    } else {
        match check(dest, &canonical) {
            DeployHealth::Linked { .. } => return Ok(()),
            DeployHealth::Broken => link::remove_dest_path(dest)?,
            DeployHealth::Unlinked => {
                if dest.exists() {
                    link::remove_dest_path(dest)?;
                }
            }
        }
    }

    paths::ensure_parent_dir(dest).map_err(|e| CoreError::LinkFailed {
        src: src.to_string(),
        dest: dest.to_string(),
        reason: format!("创建父目录失败: {e}"),
        hint: "检查平台配置目录写入权限".to_string(),
    })?;

    if material_src.is_dir() {
        copy_dir_materialized(&material_src, dest)?;
    } else {
        fs::copy(material_src.as_std_path(), dest.as_std_path()).map_err(CoreError::Io)?;
    }
    Ok(())
}

/// 收回平台副本或 legacy symlink；不删除源。
pub fn retract(dest: &Utf8Path) -> Result<(), CoreError> {
    if !dest.exists() && fs::symlink_metadata(dest.as_std_path()).is_err() {
        return Ok(());
    }

    if is_symlink_entry(dest) {
        return Err(unowned_retract_error(dest));
    }

    let marker = marker_path_for_dest(dest);
    if marker.exists() {
        if dest.is_dir() {
            fs::remove_dir_all(dest.as_std_path()).map_err(CoreError::Io)?;
        } else {
            fs::remove_file(dest.as_std_path()).map_err(CoreError::Io)?;
            let _ = fs::remove_file(marker.as_std_path());
        }
        return Ok(());
    }

    if dest.is_dir() && dest.join(MARKER_NAME).exists() {
        return fs::remove_dir_all(dest.as_std_path()).map_err(CoreError::Io);
    }

    Err(unowned_retract_error(dest))
}

/// 收回有明确 expected source 的 legacy symlink；普通副本仍只接受 marker 证明。
pub fn retract_linked_to(dest: &Utf8Path, expected_src: &Utf8Path) -> Result<(), CoreError> {
    if is_symlink_entry(dest) {
        let expected = canonical_src(expected_src);
        return match link::check(dest, &expected) {
            link::LinkHealth::Linked { .. } => link::unlink(dest),
            link::LinkHealth::WrongSource { actual, .. } if canonical_src(&actual) == expected => {
                link::unlink(dest)
            }
            link::LinkHealth::Broken { .. }
            | link::LinkHealth::WrongSource { .. }
            | link::LinkHealth::WrongType { .. } => Err(unowned_retract_error(dest)),
        };
    }
    retract(dest)
}

fn unowned_retract_error(dest: &Utf8Path) -> CoreError {
    CoreError::LinkFailed {
        src: "agents-manager managed deploy".to_owned(),
        dest: dest.to_string(),
        reason: "目标不是本工具下发：没有 marker 或精确匹配的 legacy symlink".to_owned(),
        hint: "保留该外部资产；如需接管，请先通过显式迁移生成计划".to_owned(),
    }
}

/// 导入到源后，将平台上**已存在**的同名资产纳管为从 `src` 下发（写标记，不覆盖内容）。
///
/// 典型场景：从 Claude 导入 skill 到项目源后，Claude 侧目录仍在，应显示为已同步。
pub fn adopt_existing_deploy(src: &Utf8Path, dest: &Utf8Path) -> Result<(), CoreError> {
    if !src.exists() {
        return Ok(());
    }
    let canonical = canonical_src(&resolve_copy_source(src));
    if fs::symlink_metadata(dest.as_std_path()).is_err() {
        return Ok(());
    }

    match check(dest, &canonical) {
        DeployHealth::Linked { .. } => {
            if is_symlink_entry(dest) {
                return deploy(src, dest);
            }
            Ok(())
        }
        DeployHealth::Broken => deploy(src, dest),
        DeployHealth::Unlinked => {
            if is_symlink_entry(dest) {
                return Ok(());
            }
            Ok(())
        }
    }
}

/// 检查 dest 是否由本工具从 `expected_src` 下发。
pub fn check(dest: &Utf8Path, expected_src: &Utf8Path) -> DeployHealth {
    let expected = canonical_src(&resolve_copy_source(expected_src));

    if is_symlink_entry(dest) {
        return match link::check(dest, &expected) {
            link::LinkHealth::Linked { src } => DeployHealth::Linked { src },
            link::LinkHealth::Broken { .. } => DeployHealth::Broken,
            link::LinkHealth::WrongSource { .. } | link::LinkHealth::WrongType { .. } => {
                DeployHealth::Unlinked
            }
        };
    }

    if !dest.exists() {
        return DeployHealth::Unlinked;
    }

    let marker = if dest.is_dir() {
        dest.join(MARKER_NAME)
    } else {
        marker_path_for_dest(dest)
    };

    if let Some(recorded) = read_marker(&marker) {
        if sources_match(&recorded, &expected) {
            if recorded.exists() || expected.exists() {
                return DeployHealth::Linked { src: recorded };
            }
            return DeployHealth::Broken;
        }
        return DeployHealth::Unlinked;
    }

    if dest.is_dir() {
        let inner = dest.join(MARKER_NAME);
        if let Some(recorded) = read_marker(&inner) {
            if sources_match(&recorded, &expected) {
                return if expected.exists() {
                    DeployHealth::Linked { src: recorded }
                } else {
                    DeployHealth::Broken
                };
            }
        }
    }

    if content_matches_source_for_dest(expected_src, dest) {
        return DeployHealth::Linked { src: expected };
    }

    DeployHealth::Unlinked
}

/// 平台 `dest` 是否与 agents-manager 源 `src` **同名路径下内容一致**（用于图标点亮）。
pub fn content_matches_source(kind: AssetKind, src: &Utf8Path, dest: &Utf8Path) -> bool {
    let link_src = link_src_for_create(kind, src);
    content_matches_source_for_dest(&link_src, dest)
}

fn content_matches_source_for_dest(src: &Utf8Path, dest: &Utf8Path) -> bool {
    if fs::symlink_metadata(dest.as_std_path()).is_err() {
        return false;
    }
    let material_src = resolve_copy_source(src);
    let material_dest = resolve_copy_source(dest);
    if !material_src.exists() || !material_dest.exists() {
        return false;
    }
    if material_src.is_dir() && material_dest.is_dir() {
        dir_trees_equal(&material_src, &material_dest)
    } else if material_src.is_file() && material_dest.is_file() {
        files_equal(&material_src, &material_dest)
    } else {
        false
    }
}

fn should_skip_tree_rel(rel: &Path) -> bool {
    if rel.as_os_str().is_empty() {
        return true;
    }
    for component in rel.components() {
        if let Component::Normal(name) = component {
            let s = name.to_string_lossy();
            if s == MARKER_NAME || s.starts_with('.') {
                return true;
            }
        }
    }
    false
}

fn tree_content_map(root: &Utf8Path) -> Result<HashMap<String, Vec<u8>>, CoreError> {
    let mut map = HashMap::new();
    for entry in WalkDir::new(root.as_std_path()).follow_links(true) {
        let entry = entry.map_err(|e| CoreError::Io(e.into()))?;
        if entry.file_type().is_dir() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(root.as_std_path())
            .map_err(|e| CoreError::InvalidPath(format!("strip_prefix: {e}")))?;
        if should_skip_tree_rel(rel) {
            continue;
        }
        let content = fs::read(entry.path()).map_err(CoreError::Io)?;
        map.insert(rel.to_string_lossy().into_owned(), content);
    }
    Ok(map)
}

fn dir_trees_equal(a: &Utf8Path, b: &Utf8Path) -> bool {
    match (tree_content_map(a), tree_content_map(b)) {
        (Ok(ma), Ok(mb)) => ma == mb,
        _ => false,
    }
}

fn files_equal(a: &Utf8Path, b: &Utf8Path) -> bool {
    match (fs::read(a.as_std_path()), fs::read(b.as_std_path())) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

fn copy_dir_materialized(src: &Utf8Path, dest: &Utf8Path) -> Result<(), CoreError> {
    fs::create_dir_all(dest.as_std_path()).map_err(CoreError::Io)?;
    for entry in WalkDir::new(src.as_std_path()).follow_links(true) {
        let entry = entry.map_err(|e| CoreError::Io(e.into()))?;
        let rel = entry
            .path()
            .strip_prefix(src.as_std_path())
            .map_err(|e| CoreError::InvalidPath(format!("strip_prefix: {e}")))?;
        if rel.as_os_str().is_empty() {
            continue;
        }
        let name = rel.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == MARKER_NAME {
            continue;
        }
        let target = dest.join(rel.to_string_lossy().as_ref());
        if entry.file_type().is_dir() {
            fs::create_dir_all(target.as_std_path()).map_err(CoreError::Io)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent.as_std_path()).map_err(CoreError::Io)?;
            }
            fs::copy(entry.path(), target.as_std_path()).map_err(CoreError::Io)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn retract_preserves_unmarked_deployed_skill_copy() {
        let tmp = TempDir::new().unwrap();
        let src_root = Utf8PathBuf::from_path_buf(tmp.path().join("src")).unwrap();
        let plat_root = Utf8PathBuf::from_path_buf(tmp.path().join("plat")).unwrap();
        let skill = src_root.join("skills/foo");
        fs::create_dir_all(skill.join("refs").as_std_path()).unwrap();
        fs::write(skill.join("SKILL.md").as_std_path(), "# foo").unwrap();
        let dest = plat_root.join("skills/foo");

        deploy(&skill, &dest).unwrap();
        assert!(dest.join("SKILL.md").is_file());
        assert!(!dest.join(MARKER_NAME).exists());
        assert!(!is_symlink_entry(&dest));
        assert_eq!(
            check(&dest, &skill),
            DeployHealth::Linked {
                src: fs::canonicalize(skill.as_std_path())
                    .map(|p| Utf8PathBuf::from(p.to_string_lossy().into_owned()))
                    .unwrap()
            }
        );

        assert!(retract(&dest).is_err());
        assert!(dest.join("SKILL.md").is_file());
    }

    #[test]
    fn retract_refuses_unowned_regular_directory() {
        let tmp = TempDir::new().unwrap();
        let dest = Utf8PathBuf::from_path_buf(tmp.path().join("platform/skills/foreign")).unwrap();
        fs::create_dir_all(dest.as_std_path()).unwrap();
        fs::write(dest.join("SKILL.md").as_std_path(), "# foreign").unwrap();

        assert!(retract(&dest).is_err());
        assert_eq!(
            fs::read_to_string(dest.join("SKILL.md").as_std_path()).unwrap(),
            "# foreign"
        );
    }

    #[cfg(unix)]
    #[test]
    fn retract_refuses_symlink_without_expected_source() {
        let tmp = TempDir::new().unwrap();
        let foreign = Utf8PathBuf::from_path_buf(tmp.path().join("foreign/skill")).unwrap();
        let dest = Utf8PathBuf::from_path_buf(tmp.path().join("platform/skills/demo")).unwrap();
        fs::create_dir_all(foreign.as_std_path()).unwrap();
        fs::create_dir_all(dest.parent().unwrap().as_std_path()).unwrap();
        std::os::unix::fs::symlink(foreign.as_std_path(), dest.as_std_path()).unwrap();

        assert!(retract(&dest).is_err());
        assert!(fs::symlink_metadata(dest.as_std_path()).is_ok());
    }

    #[test]
    fn deploy_migrates_legacy_symlink_to_copy() {
        let tmp = TempDir::new().unwrap();
        let src = Utf8PathBuf::from_path_buf(tmp.path().join("src/skill")).unwrap();
        let dest = Utf8PathBuf::from_path_buf(tmp.path().join("dest/skill")).unwrap();
        fs::create_dir_all(src.as_std_path()).unwrap();
        fs::write(src.join("SKILL.md").as_std_path(), "# migrated").unwrap();
        fs::create_dir_all(dest.parent().unwrap().as_std_path()).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(src.as_std_path(), dest.as_std_path()).unwrap();

        assert!(is_symlink_entry(&dest));
        deploy(&src, &dest).unwrap();
        assert!(!is_symlink_entry(&dest));
        assert!(dest.join("SKILL.md").is_file());
        assert!(!has_copy_marker(&dest));
        assert_eq!(
            fs::read_to_string(dest.join("SKILL.md").as_std_path()).unwrap(),
            "# migrated"
        );
    }

    #[test]
    fn check_linked_when_content_matches_without_marker() {
        let tmp = TempDir::new().unwrap();
        let src = Utf8PathBuf::from_path_buf(tmp.path().join("src/foo")).unwrap();
        let dest = Utf8PathBuf::from_path_buf(tmp.path().join("plat/foo")).unwrap();
        fs::create_dir_all(src.as_std_path()).unwrap();
        fs::create_dir_all(dest.as_std_path()).unwrap();
        fs::write(src.join("SKILL.md").as_std_path(), "same").unwrap();
        fs::write(dest.join("SKILL.md").as_std_path(), "same").unwrap();

        assert!(matches!(check(&dest, &src), DeployHealth::Linked { .. }));
        assert!(content_matches_source(
            AssetKind::Skill,
            &src.join("SKILL.md"),
            &dest
        ));
    }

    #[test]
    fn check_unlinked_when_content_differs() {
        let tmp = TempDir::new().unwrap();
        let src = Utf8PathBuf::from_path_buf(tmp.path().join("src/foo")).unwrap();
        let dest = Utf8PathBuf::from_path_buf(tmp.path().join("plat/foo")).unwrap();
        fs::create_dir_all(src.as_std_path()).unwrap();
        fs::create_dir_all(dest.as_std_path()).unwrap();
        fs::write(src.join("SKILL.md").as_std_path(), "a").unwrap();
        fs::write(dest.join("SKILL.md").as_std_path(), "b").unwrap();

        assert_eq!(check(&dest, &src), DeployHealth::Unlinked);
    }

    #[test]
    fn adopt_existing_deploy_marks_platform_copy() {
        let tmp = TempDir::new().unwrap();
        let src = Utf8PathBuf::from_path_buf(tmp.path().join("src/skill")).unwrap();
        let dest = Utf8PathBuf::from_path_buf(tmp.path().join("plat/skill")).unwrap();
        fs::create_dir_all(src.as_std_path()).unwrap();
        fs::write(src.join("SKILL.md").as_std_path(), "# x").unwrap();
        fs::create_dir_all(dest.as_std_path()).unwrap();
        fs::write(dest.join("SKILL.md").as_std_path(), "# x").unwrap();

        adopt_existing_deploy(&src, &dest).unwrap();
        assert!(!dest.join(MARKER_NAME).exists());
        assert!(matches!(check(&dest, &src), DeployHealth::Linked { .. }));
    }

    #[test]
    fn retract_linked_to_removes_exact_legacy_symlink() {
        let tmp = TempDir::new().unwrap();
        let src = Utf8PathBuf::from_path_buf(tmp.path().join("src/skill")).unwrap();
        let dest = Utf8PathBuf::from_path_buf(tmp.path().join("dest/skill")).unwrap();
        fs::create_dir_all(src.as_std_path()).unwrap();
        fs::create_dir_all(dest.parent().unwrap().as_std_path()).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(src.as_std_path(), dest.as_std_path()).unwrap();

        retract_linked_to(&dest, &src).unwrap();
        assert!(!dest.exists());
        assert!(src.exists());
    }

    #[test]
    fn copy_tree_follows_symlink_source() {
        let tmp = TempDir::new().unwrap();
        let real = Utf8PathBuf::from_path_buf(tmp.path().join("real")).unwrap();
        let link = Utf8PathBuf::from_path_buf(tmp.path().join("link")).unwrap();
        let dest = Utf8PathBuf::from_path_buf(tmp.path().join("dest")).unwrap();
        fs::create_dir_all(real.as_std_path()).unwrap();
        fs::write(real.join("SKILL.md").as_std_path(), "body").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(real.as_std_path(), link.as_std_path()).unwrap();

        copy_tree(&link, &dest).unwrap();
        assert!(!is_symlink_entry(&dest));
        assert_eq!(
            fs::read_to_string(dest.join("SKILL.md").as_std_path()).unwrap(),
            "body"
        );
    }

    #[test]
    #[cfg(unix)]
    fn deploy_replaces_hardlink_with_independent_copy() {
        let tmp = TempDir::new().unwrap();
        let src = Utf8PathBuf::from_path_buf(tmp.path().join("src.md")).unwrap();
        let dest = Utf8PathBuf::from_path_buf(tmp.path().join("plat.md")).unwrap();
        fs::write(src.as_std_path(), "v1").unwrap();
        fs::hard_link(src.as_std_path(), dest.as_std_path()).unwrap();
        assert!(path_independence::paths_alias(&dest, &src));

        deploy(&src, &dest).unwrap();
        assert!(!path_independence::paths_alias(&dest, &src));
        assert_eq!(fs::read_to_string(dest.as_std_path()).unwrap(), "v1");

        fs::write(src.as_std_path(), "v2").unwrap();
        assert_eq!(fs::read_to_string(dest.as_std_path()).unwrap(), "v1");
    }
}
