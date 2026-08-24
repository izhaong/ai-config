use camino::{Utf8Path, Utf8PathBuf};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

use crate::error::CoreError;
use crate::projection::model::{FingerprintType, PathFingerprint};

const LEGACY_MARKER_NAME: &str = ".agents-manager-deploy.json";

/// 计算目录内容的稳定摘要。
///
/// 路径、类型和文件内容均参与摘要；隐藏文件必须参与，只有历史部署 marker
/// 不参与。权限 mode 属于 apply 前置条件，不能参与“内容等价”判定。软链接只记录
/// 其 target，不跟随外部目录。
pub fn directory_digest(root: &Utf8Path) -> Result<String, CoreError> {
    let mut entries = Vec::new();
    for entry in WalkDir::new(root.as_std_path()).follow_links(false) {
        let entry = entry.map_err(|error| CoreError::InvalidPath(error.to_string()))?;
        let path = entry.path();
        if path == root.as_std_path() {
            continue;
        }
        let relative = path
            .strip_prefix(root.as_std_path())
            .map_err(|error| CoreError::InvalidPath(error.to_string()))?;
        if relative == Path::new(LEGACY_MARKER_NAME) {
            continue;
        }
        let relative = relative
            .to_str()
            .ok_or_else(|| CoreError::InvalidPath("fingerprint 路径不是 UTF-8".to_owned()))?;
        entries.push((relative.to_owned(), path.to_path_buf()));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));

    let mut hasher = Sha256::new();
    for (relative, path) in entries {
        let metadata = fs::symlink_metadata(&path)?;
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        hasher.update([entry_kind(&metadata)]);
        hasher.update([0]);

        if metadata.file_type().is_symlink() {
            let target = fs::read_link(&path)?;
            hasher.update(target.to_string_lossy().as_bytes());
        } else if metadata.is_file() {
            hasher.update(fs::read(&path)?);
        }
        hasher.update([0]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// 计算单个文件、目录或符号链接的语义内容摘要。
///
/// 单文件不把文件名纳入摘要，使不同平台目标的同内容 Rule/Command/Agent/Prompt
/// 可判定为 `Equivalent`；符号链接只记录其 raw target，绝不跟随未知外部路径。
pub fn path_content_digest(path: &Utf8Path) -> Result<String, CoreError> {
    let metadata = fs::symlink_metadata(path.as_std_path())?;
    if metadata.file_type().is_dir() {
        return directory_digest(path);
    }

    let mut hasher = Sha256::new();
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(path.as_std_path())?;
        hasher.update(b"link\0");
        hasher.update(target.as_os_str().as_encoded_bytes());
    } else if metadata.file_type().is_file() {
        hasher.update(b"file\0");
        hasher.update(fs::read(path.as_std_path())?);
    } else {
        return Err(CoreError::InvalidPath(format!(
            "cannot fingerprint unsupported path type: {path}"
        )));
    }
    Ok(hex::encode(hasher.finalize()))
}

/// 获取 apply 使用的严格路径快照。
///
/// `digest` 仍是语义内容摘要；entry type、link target 与 Unix mode 则让 executor
/// 能在写入前拒绝目标自 plan 后发生的形态变化。
pub fn path_fingerprint(path: &Utf8Path) -> Result<PathFingerprint, CoreError> {
    let metadata = match fs::symlink_metadata(path.as_std_path()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PathFingerprint {
                entry_type: FingerprintType::Missing,
                digest: None,
                link_target: None,
                mode: None,
            });
        }
        Err(error) => return Err(CoreError::Io(error)),
    };
    let file_type = metadata.file_type();
    let entry_type = if file_type.is_file() {
        FingerprintType::File
    } else if file_type.is_dir() {
        FingerprintType::Directory
    } else if file_type.is_symlink() {
        FingerprintType::Symlink
    } else {
        FingerprintType::Other
    };
    if entry_type == FingerprintType::Other {
        return Err(CoreError::InvalidPath(format!(
            "cannot fingerprint unsupported path type: {path}"
        )));
    }
    let link_target = if entry_type == FingerprintType::Symlink {
        Utf8PathBuf::from_path_buf(fs::read_link(path.as_std_path())?)
            .map(Some)
            .map_err(|non_utf8| CoreError::InvalidPath(non_utf8.to_string_lossy().into_owned()))?
    } else {
        None
    };

    Ok(PathFingerprint {
        entry_type,
        digest: Some(path_content_digest(path)?),
        link_target,
        mode: path_mode(&metadata),
    })
}

#[cfg(unix)]
fn path_mode(metadata: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;

    Some(metadata.permissions().mode())
}

#[cfg(not(unix))]
fn path_mode(_metadata: &fs::Metadata) -> Option<u32> {
    None
}

fn entry_kind(metadata: &fs::Metadata) -> u8 {
    if metadata.file_type().is_symlink() {
        b'l'
    } else if metadata.is_dir() {
        b'd'
    } else if metadata.is_file() {
        b'f'
    } else {
        b'o'
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8Path;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn directory_digest_includes_dotfiles_and_excludes_only_legacy_marker() {
        let temp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap().join("skill");
        fs::create_dir_all(root.as_std_path()).unwrap();
        fs::write(root.join("SKILL.md").as_std_path(), "body\n").unwrap();
        fs::write(root.join(".hidden").as_std_path(), "first\n").unwrap();
        fs::write(
            root.join(".agents-manager-deploy.json").as_std_path(),
            "legacy marker\n",
        )
        .unwrap();

        let before = directory_digest(&root).unwrap();

        fs::write(
            root.join(".agents-manager-deploy.json").as_std_path(),
            "changed legacy marker\n",
        )
        .unwrap();
        assert_eq!(before, directory_digest(&root).unwrap());

        fs::write(root.join(".hidden").as_std_path(), "changed hidden file\n").unwrap();
        assert_ne!(before, directory_digest(&root).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn directory_digest_ignores_mode_for_content_equivalence() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap().join("skill");
        fs::create_dir_all(root.as_std_path()).unwrap();
        let skill_md = root.join("SKILL.md");
        fs::write(skill_md.as_std_path(), "body\n").unwrap();
        let before = directory_digest(&root).unwrap();

        fs::set_permissions(skill_md.as_std_path(), fs::Permissions::from_mode(0o600)).unwrap();

        assert_eq!(before, directory_digest(&root).unwrap());
    }

    #[test]
    fn directory_digest_includes_nested_marker_named_files() {
        let temp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap().join("skill");
        let nested = root.join("resources/.agents-manager-deploy.json");
        fs::create_dir_all(nested.parent().unwrap().as_std_path()).unwrap();
        fs::write(root.join("SKILL.md").as_std_path(), "body\n").unwrap();
        fs::write(nested.as_std_path(), "first resource\n").unwrap();
        let before = directory_digest(&root).unwrap();

        fs::write(nested.as_std_path(), "changed resource\n").unwrap();

        assert_ne!(before, directory_digest(&root).unwrap());
    }

    #[test]
    fn path_content_digest_compares_regular_files_without_their_file_names() {
        let temp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let first = root.join("source/rule.mdc");
        let second = root.join("target/renamed-rule.mdc");
        fs::create_dir_all(first.parent().unwrap().as_std_path()).unwrap();
        fs::create_dir_all(second.parent().unwrap().as_std_path()).unwrap();
        fs::write(first.as_std_path(), "same body\n").unwrap();
        fs::write(second.as_std_path(), "same body\n").unwrap();

        assert_eq!(
            path_content_digest(&first).unwrap(),
            path_content_digest(&second).unwrap()
        );

        fs::write(second.as_std_path(), "different body\n").unwrap();
        assert_ne!(
            path_content_digest(&first).unwrap(),
            path_content_digest(&second).unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn path_fingerprint_detects_mode_changes_without_changing_content_digest() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().unwrap();
        let file = Utf8Path::from_path(temp.path()).unwrap().join("rule.mdc");
        fs::write(file.as_std_path(), "rule body\n").unwrap();
        let before = path_fingerprint(&file).unwrap();

        fs::set_permissions(file.as_std_path(), fs::Permissions::from_mode(0o600)).unwrap();
        let after = path_fingerprint(&file).unwrap();

        assert_eq!(before.digest, after.digest);
        assert_ne!(before.mode, after.mode);
    }

    #[test]
    fn path_fingerprint_returns_missing_precondition_for_absent_path() {
        let temp = TempDir::new().unwrap();
        let missing = Utf8Path::from_path(temp.path())
            .unwrap()
            .join("missing-target");

        assert_eq!(
            path_fingerprint(&missing).unwrap(),
            PathFingerprint {
                entry_type: FingerprintType::Missing,
                digest: None,
                link_target: None,
                mode: None,
            }
        );
    }
}
