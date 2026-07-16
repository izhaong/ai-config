use camino::Utf8Path;
use sha2::{Digest, Sha256};
use std::fs;
use walkdir::WalkDir;

use crate::error::CoreError;

const LEGACY_MARKER_NAME: &str = ".ai-config-deploy.json";

/// 计算目录内容的稳定摘要。
///
/// 路径、类型、权限和文件内容均参与摘要；隐藏文件必须参与，只有历史部署 marker
/// 不参与。软链接只记录其 target，不跟随外部目录。
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
        if relative.file_name().is_some_and(|name| name == LEGACY_MARKER_NAME) {
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
        hasher.update(entry_mode(&metadata).to_le_bytes());
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

#[cfg(unix)]
fn entry_mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode()
}

#[cfg(not(unix))]
fn entry_mode(_metadata: &fs::Metadata) -> u32 {
    0
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
            root.join(".ai-config-deploy.json").as_std_path(),
            "legacy marker\n",
        )
        .unwrap();

        let before = directory_digest(&root).unwrap();

        fs::write(
            root.join(".ai-config-deploy.json").as_std_path(),
            "changed legacy marker\n",
        )
        .unwrap();
        assert_eq!(before, directory_digest(&root).unwrap());

        fs::write(root.join(".hidden").as_std_path(), "changed hidden file\n").unwrap();
        assert_ne!(before, directory_digest(&root).unwrap());
    }
}
