//! 平台下发：复制资产到目标路径（非 symlink），避免删源后各平台链接断裂。
//!
//! - `deploy`：目录/文件实体复制 + `.ai-config-deploy.json` 标记；遇历史 symlink 自动迁移
//! - `retract`：删除本工具下发的副本；兼容收回历史 symlink
//! - `check`：标记 / legacy symlink / **同名同内容** 判定是否已同步
//! - `copy_tree` / `resolve_copy_source`：沿 symlink 找到真实路径并做硬拷贝

use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path};
use std::time::{SystemTime, UNIX_EPOCH};

use camino::Utf8Path;
use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::error::CoreError;
use crate::link;
use crate::model::AssetKind;
use crate::paths;
use crate::sync::link_src_for_create;

const MARKER_NAME: &str = ".ai-config-deploy.json";

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

fn write_marker(marker: &Utf8Path, src: &Utf8Path) -> Result<(), CoreError> {
    if let Some(parent) = marker.parent() {
        paths::ensure_parent_dir(parent)?;
    }
    let body = DeployMarker {
        version: 1,
        source: src.to_string(),
    };
    let json = serde_json::to_string_pretty(&body).map_err(|e| CoreError::Io(e.into()))?;
    fs::write(marker.as_std_path(), json).map_err(CoreError::Io)
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
pub fn deploy(src: &Utf8Path, dest: &Utf8Path) -> Result<(), CoreError> {
    if src == dest {
        return Err(CoreError::InvalidPath(format!(
            "src 与 dest 相同,无法下发: {src}"
        )));
    }
    let material_src = resolve_copy_source(src);
    if !material_src.exists() {
        return Err(CoreError::LinkFailed {
            src: src.to_string(),
            dest: dest.to_string(),
            reason: format!("源不存在: {src}"),
            hint: "确认资产仍在 ~/.ai-config 或项目 .ai-config 中".to_string(),
        });
    }

    let canonical = canonical_src(&material_src);

    // 历史 symlink：后续操作一律迁移为实体副本
    if is_symlink_entry(dest) {
        let _ = link::unlink(dest);
    } else {
        match check(dest, &canonical) {
            DeployHealth::Linked { .. } if has_copy_marker(dest) => return Ok(()),
            DeployHealth::Linked { .. } => {
                // 无标记但 check 认为已链接（不应出现）→ 备份后重建
                backup_dest(dest)?;
            }
            DeployHealth::Broken => backup_dest(dest)?,
            DeployHealth::Unlinked => {
                if dest.exists() {
                    backup_dest(dest)?;
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
        write_marker(&dest.join(MARKER_NAME), &canonical)?;
    } else {
        fs::copy(material_src.as_std_path(), dest.as_std_path()).map_err(CoreError::Io)?;
        write_marker(&marker_path_for_dest(dest), &canonical)?;
    }
    Ok(())
}

/// 收回平台副本或 legacy symlink；不删除源。
pub fn retract(dest: &Utf8Path) -> Result<(), CoreError> {
    if !dest.exists() && fs::symlink_metadata(dest.as_std_path()).is_err() {
        return Ok(());
    }

    if is_symlink_entry(dest) {
        return link::unlink(dest);
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

    Err(CoreError::LinkFailed {
        src: String::new(),
        dest: dest.to_string(),
        reason: "目标不是本工具下发的副本或链接".to_string(),
        hint: "仅收回 ai-config 复制/链接的资产;手工目录请自行删除".to_string(),
    })
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
            if !has_copy_marker(dest) {
                if dest.is_dir() {
                    write_marker(&dest.join(MARKER_NAME), &canonical)?;
                } else if dest.is_file() {
                    write_marker(&marker_path_for_dest(dest), &canonical)?;
                }
            }
            Ok(())
        }
        DeployHealth::Broken => deploy(src, dest),
        DeployHealth::Unlinked => {
            if is_symlink_entry(dest) {
                return Ok(());
            }
            if dest.is_dir() {
                write_marker(&dest.join(MARKER_NAME), &canonical)?;
            } else if dest.is_file() {
                write_marker(&marker_path_for_dest(dest), &canonical)?;
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
        return DeployHealth::Linked {
            src: expected,
        };
    }

    DeployHealth::Unlinked
}

/// 平台 `dest` 是否与 ai-config 源 `src` **同名路径下内容一致**（用于图标点亮）。
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
        match component {
            Component::Normal(name) => {
                let s = name.to_string_lossy();
                if s == MARKER_NAME || s.starts_with('.') {
                    return true;
                }
            }
            _ => {}
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

fn backup_dest(dest: &Utf8Path) -> Result<(), CoreError> {
    let ts = unix_ts();
    let bak = Utf8PathBuf::from(format!("{dest}.bak-{ts}"));
    if bak.exists() {
        return Err(CoreError::LinkFailed {
            src: String::new(),
            dest: dest.to_string(),
            reason: format!("备份路径已存在: {bak}"),
            hint: "稍后重试".to_string(),
        });
    }
    fs::rename(dest.as_std_path(), bak.as_std_path()).map_err(|e| CoreError::LinkFailed {
        src: String::new(),
        dest: dest.to_string(),
        reason: format!("无法备份旧目标 {dest} → {bak}: {e}"),
        hint: "检查目录权限后重试".to_string(),
    })
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

fn unix_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn deploy_copy_and_retract_skill_dir() {
        let tmp = TempDir::new().unwrap();
        let src_root = Utf8PathBuf::from_path_buf(tmp.path().join("src")).unwrap();
        let plat_root = Utf8PathBuf::from_path_buf(tmp.path().join("plat")).unwrap();
        let skill = src_root.join("skills/foo");
        fs::create_dir_all(skill.join("refs").as_std_path()).unwrap();
        fs::write(skill.join("SKILL.md").as_std_path(), "# foo").unwrap();
        let dest = plat_root.join("skills/foo");

        deploy(&skill, &dest).unwrap();
        assert!(dest.join("SKILL.md").is_file());
        assert!(dest.join(MARKER_NAME).is_file());
        assert!(!is_symlink_entry(&dest));
        assert_eq!(
            check(&dest, &skill),
            DeployHealth::Linked {
                src: fs::canonicalize(skill.as_std_path())
                    .map(|p| Utf8PathBuf::from(p.to_string_lossy().into_owned()))
                    .unwrap()
            }
        );

        retract(&dest).unwrap();
        assert!(!dest.exists());
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
        assert!(has_copy_marker(&dest));
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
        assert!(content_matches_source(AssetKind::Skill, &src.join("SKILL.md"), &dest));
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
        assert!(dest.join(MARKER_NAME).is_file());
        assert!(matches!(check(&dest, &src), DeployHealth::Linked { .. }));
    }

    #[test]
    fn retract_legacy_symlink() {
        let tmp = TempDir::new().unwrap();
        let src = Utf8PathBuf::from_path_buf(tmp.path().join("src/skill")).unwrap();
        let dest = Utf8PathBuf::from_path_buf(tmp.path().join("dest/skill")).unwrap();
        fs::create_dir_all(src.as_std_path()).unwrap();
        fs::create_dir_all(dest.parent().unwrap().as_std_path()).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(src.as_std_path(), dest.as_std_path()).unwrap();

        retract(&dest).unwrap();
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
}
