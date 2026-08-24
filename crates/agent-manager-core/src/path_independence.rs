//! 检测下发目标是否与源共用路径 / symlink / 硬链接（同 inode）。
//!
//! agents-manager 下发策略：**实体硬拷贝**，平台目录与 `~/.agents-manager` / `project/.agents-manager`
//! 之间不得共享 inode；各 IDE 平台之间也不得互相链接。

use camino::{Utf8Path, Utf8PathBuf};

/// `dest` 是否与 `src` 指向同一底层文件（同路径、symlink 指向、或 Unix 硬链接同 inode）。
pub fn paths_alias(dest: &Utf8Path, src: &Utf8Path) -> bool {
    if dest.as_str() == src.as_str() {
        return true;
    }
    let Ok(src_meta) = std::fs::metadata(src.as_std_path()) else {
        return false;
    };
    let Ok(dest_meta) = std::fs::symlink_metadata(dest.as_std_path()) else {
        return false;
    };
    if dest_meta.file_type().is_symlink() {
        if let Ok(target) = std::fs::read_link(dest.as_std_path()) {
            let mut resolved = Utf8PathBuf::from(target.to_string_lossy().into_owned());
            if resolved.is_relative() {
                if let Some(parent) = dest.parent() {
                    resolved = parent.join(resolved);
                }
            }
            if let (Ok(dm), Ok(sm)) = (
                std::fs::canonicalize(dest.as_std_path()),
                std::fs::canonicalize(src.as_std_path()),
            ) {
                return dm == sm;
            }
            return resolved.as_str() == src.as_str();
        }
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        dest_meta.dev() == src_meta.dev() && dest_meta.ino() == src_meta.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = dest_meta;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn same_path_is_alias() {
        let p = Utf8Path::new("/tmp/foo");
        assert!(paths_alias(p, p));
    }

    #[test]
    #[cfg(unix)]
    fn hard_link_is_alias() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src.txt");
        let dest = tmp.path().join("dest.txt");
        fs::write(&src, "x").unwrap();
        fs::hard_link(&src, &dest).unwrap();
        let src_u = Utf8PathBuf::from_path_buf(src).unwrap();
        let dest_u = Utf8PathBuf::from_path_buf(dest).unwrap();
        assert!(paths_alias(&dest_u, &src_u));
    }

    #[test]
    #[cfg(unix)]
    fn symlink_is_alias() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src.txt");
        let dest = tmp.path().join("dest.txt");
        fs::write(&src, "x").unwrap();
        std::os::unix::fs::symlink(&src, &dest).unwrap();
        let src_u = Utf8PathBuf::from_path_buf(src).unwrap();
        let dest_u = Utf8PathBuf::from_path_buf(dest).unwrap();
        assert!(paths_alias(&dest_u, &src_u));
    }

    #[test]
    fn independent_copy_is_not_alias() {
        let tmp = TempDir::new().unwrap();
        let a = tmp.path().join("a.txt");
        let b = tmp.path().join("b.txt");
        fs::write(&a, "a").unwrap();
        fs::write(&b, "a").unwrap();
        let a_u = Utf8PathBuf::from_path_buf(a).unwrap();
        let b_u = Utf8PathBuf::from_path_buf(b).unwrap();
        assert!(!paths_alias(&b_u, &a_u));
    }
}
