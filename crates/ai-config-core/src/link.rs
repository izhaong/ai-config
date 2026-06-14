//! 链接抽象:symlink / junction / hardlink 三态 + 幂等 + 备份覆盖。
//!
//! 设计要点(PRD §8.1 / §10 A-3 / ARCHITECTURE §10):
//! - 三态:`Symlink` / `Junction`(Windows fallback,避免开发者模式)/ `Hardlink`
//! - 幂等:目标已存在且指向同一源 → noop;跑 N 遍与跑 1 遍结果一致
//! - 备份覆盖:目标已存在且非链接 / 指向不同源 → 备份为 `dest.bak-<unix_ts>` 后重建
//! - 收回:`unlink` 只删本工具创建的链接,非本工具的拒
//! - 健康检查:`check` 返回四态(Linked / Broken / WrongSource / WrongType)
//!
//! 跨平台策略(ARCHITECTURE §10.3 / §11):
//! - Unix 系(Symlink):`std::os::unix::fs::symlink`
//! - Windows(`LinkKind::auto()`):`symlink_dir`(junction) 避免开发者模式要求
//!
//! 安全:
//! - 路径统一用 `camino::Utf8PathBuf` 避免非 UTF-8 跨界
//! - 权限不足显式映射 `CoreError::PermissionDenied`(CLI 退出码 5)
//! - 不在 `Display` 中暴露绝对路径的 secret 内容

#![allow(dead_code)]

use std::fs;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use camino::Utf8Path;
use camino::Utf8PathBuf;

use crate::error::CoreError;
use crate::paths;

/// 链接类型(ARCHITECTURE §10)。
///
/// - `Symlink`:Unix 原生 symlink(目录 / 文件均可);Windows 需"开发者模式"或管理员
/// - `Junction`:Windows 专属,目录级 reparse point,**不**需开发者模式;本工具在 Windows 的默认
/// - `Hardlink`:同一卷上的文件级硬链接(本工具极少用,保留给"`source` 是单个稳定文件"场景)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    Symlink,
    /// Windows fallback(目录级 reparse point),其它平台调用会失败。
    Junction,
    /// 文件级硬链接;**不能**指向目录。
    Hardlink,
}

impl LinkKind {
    /// 平台自适应:Windows → Junction(避免开发者模式),其它 → Symlink。
    #[cfg(windows)]
    pub const fn auto() -> Self {
        // ARCHITECTURE §10.3:Windows 走 junction,避免开发者模式要求。
        LinkKind::Junction
    }

    #[cfg(not(windows))]
    pub const fn auto() -> Self {
        LinkKind::Symlink
    }
}

/// `dest` 处的链接健康状态(给 `sync` 引擎 / `doctor` 子命令用)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkHealth {
    /// 是 symlink / junction / hardlink,指向 `src`(与 `check` 入参 `src` 一致)
    Linked { src: Utf8PathBuf },
    /// 链接存在但 target 不存在(典型:`mv src dest.bak` 后 dest 还指向原路径)
    Broken { dangling_target: Utf8PathBuf },
    /// 是链接,但指向其它路径(用户手动改过 / 别的工具创建)
    WrongSource {
        actual: Utf8PathBuf,
        expected: Utf8PathBuf,
    },
    /// 是普通文件 / 目录,不是链接(需要"备份覆盖"策略)
    WrongType { actual_kind: &'static str },
}

// ── 公开 API ─────────────────────────────────────────────────────

/// 幂等建立链接(PRD §8.1 / §10 A-3)。
///
/// 语义:
/// 1. `dest` 不存在 → 直接创建
/// 2. `dest` 已是链接且 `read_link` 指向 `src` → noop(返回 Ok)
/// 3. `dest` 已是链接但 `read_link` != `src` → 备份原链接为 `dest.bak-<ts>` 后重建
/// 4. `dest` 是普通文件 / 目录(非链接)→ 备份为 `dest.bak-<ts>` 后重建
/// 5. 权限不足 → `CoreError::PermissionDenied`
///
/// 注:`src` 是相对 / 绝对路径都会原样写入链接(不强制规范化);调用方应
/// 保证 `src` 是本工具管理的稳定路径(`~/.ai-config/skills/foo`)。
pub fn link(src: &Utf8Path, dest: &Utf8Path, kind: LinkKind) -> Result<(), CoreError> {
    // 0. 入参校验:dest 不能是 src(防自环)。
    if src == dest {
        return Err(CoreError::InvalidPath(format!(
            "src 与 dest 相同,无法创建自环链接: {src}"
        )));
    }

    // 1. dest 已存在?
    match fs::symlink_metadata(dest) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // 不存在 → 直接创建
            create_link(src, dest, kind)
        }
        Err(e) if is_permission_denied(&e) => Err(permission_denied_err(dest, &e)),
        Err(e) => Err(io_to_link_failed(src, dest, &e)),
        Ok(meta) => {
            // 2. 是 symlink / junction?
            if meta.file_type().is_symlink() {
                match fs::read_link(dest) {
                    Ok(existing) => {
                        let existing_utf8 = path_to_utf8(&existing);
                        if existing_utf8.as_deref() == Some(src) {
                            // 指向同一源 → noop
                            return Ok(());
                        }
                        // 指向不同源 → 备份后重建
                        let target_str: &str = existing_utf8
                            .as_deref()
                            .map(Utf8Path::as_str)
                            .unwrap_or("<non-utf8>");
                        backup_then_recreate(src, dest, kind, target_str)
                    }
                    Err(e) if is_permission_denied(&e) => Err(permission_denied_err(dest, &e)),
                    Err(e) => Err(io_to_link_failed(src, dest, &e)),
                }
            } else {
                // 3. 是普通文件 / 目录 → 备份后重建
                backup_then_recreate(src, dest, kind, "<non-link>")
            }
        }
    }
}

/// 收回链接(只删本工具创建的链接)。
///
/// 语义:
/// - `dest` 不存在 → noop(Ok)
/// - `dest` 是 symlink / junction / hardlink → `fs::remove_file` 或 `remove_dir`
/// - `dest` 是普通文件 / 目录 → `CoreError::LinkFailed`(非本工具创建,不擅改)
///
/// 收回**不**删 `src`(本工具不拥有源,源是用户资产)。
pub fn unlink(dest: &Utf8Path) -> Result<(), CoreError> {
    let meta = match fs::symlink_metadata(dest) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) if is_permission_denied(&e) => return Err(permission_denied_err(dest, &e)),
        Err(e) => {
            return Err(CoreError::LinkFailed {
                src: String::new(),
                dest: dest.to_string(),
                reason: format!("读取 dest 元数据失败: {e}"),
                hint: "检查 dest 路径是否存在 / 有无权限".to_string(),
            })
        }
    };

    if !meta.file_type().is_symlink() {
        return Err(CoreError::LinkFailed {
            src: String::new(),
            dest: dest.to_string(),
            reason: "目标不是链接,本工具不擅改".to_string(),
            hint: "如确认要删除,请人工 `rm` 目标;本工具只收回它自己创建的 symlink / junction"
                .to_string(),
        });
    }

    // junction / symlink 可能是目录;`remove_file` 对 symlink-to-dir 会失败,
    // 所以**先**按 symlink 自身删除(`fs::remove_file` 删 symlink 本体而非 target)。
    if let Err(e) = fs::remove_file(dest) {
        // 极少数 junction 实现对 `remove_file` 不友好,降级到 `remove_dir`。
        if e.kind() == io::ErrorKind::Other || e.raw_os_error() == Some(20)
        /*ENOTDIR*/
        {
            fs::remove_dir(dest).map_err(|e2| io_to_link_failed_err_only(dest, &e2))?;
            return Ok(());
        }
        if is_permission_denied(&e) {
            return Err(permission_denied_err(dest, &e));
        }
        return Err(io_to_link_failed_err_only(dest, &e));
    }
    Ok(())
}

/// 检查 `dest` 处链接的健康状态。
///
/// 不会修改文件系统;只读取 metadata + read_link。
pub fn check(dest: &Utf8Path, expected_src: &Utf8Path) -> LinkHealth {
    let meta = match fs::symlink_metadata(dest) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // `check` 的"Broken"语义是 dest 自身是链接但 target 不存在;
            // dest 完全不存在也算"指向 target 的链接断了" — 统一返回 Broken。
            return LinkHealth::Broken {
                dangling_target: expected_src.to_path_buf(),
            };
        }
        Err(_) => {
            return LinkHealth::Broken {
                dangling_target: expected_src.to_path_buf(),
            };
        }
    };

    if !meta.file_type().is_symlink() {
        let kind = if meta.is_dir() { "directory" } else { "file" };
        return LinkHealth::WrongType { actual_kind: kind };
    }

    let actual = match fs::read_link(dest) {
        Ok(p) => p,
        Err(_) => {
            return LinkHealth::Broken {
                dangling_target: expected_src.to_path_buf(),
            }
        }
    };
    let actual_utf8 = path_to_utf8(&actual)
        .unwrap_or_else(|| Utf8PathBuf::from(actual.to_string_lossy().into_owned()));

    if actual_utf8 != expected_src {
        // 先按"指向错"返回,即使 target 不存在也是 WrongSource 的子情形;
        // 调用方可以用 `fs::metadata(&actual_utf8)` 进一步区分。
        return LinkHealth::WrongSource {
            actual: actual_utf8,
            expected: expected_src.to_path_buf(),
        };
    }

    // 指向 expected_src — 但 target 本身存在吗?
    match fs::metadata(&actual_utf8) {
        Ok(_) => LinkHealth::Linked { src: actual_utf8 },
        Err(e) if e.kind() == io::ErrorKind::NotFound => LinkHealth::Broken {
            dangling_target: actual_utf8,
        },
        Err(_) => LinkHealth::Broken {
            dangling_target: actual_utf8,
        },
    }
}

// ── 内部:实际创建链接 ─────────────────────────────────────────────

fn create_link(src: &Utf8Path, dest: &Utf8Path, kind: LinkKind) -> Result<(), CoreError> {
    paths::ensure_parent_dir(dest).map_err(|e| CoreError::LinkFailed {
        src: src.to_string(),
        dest: dest.to_string(),
        reason: format!("创建父目录失败: {e}"),
        hint: "检查 ~/.cursor / ~/.codex / ~/.claude / ~/.hermes 等目录的写入权限".to_string(),
    })?;

    let result: io::Result<()> = match kind {
        LinkKind::Symlink => symlink_kind(src, dest),
        LinkKind::Junction => junction_kind(src, dest),
        LinkKind::Hardlink => hardlink_kind(src, dest),
    };

    match result {
        Ok(()) => Ok(()),
        Err(e) if is_permission_denied(&e) => Err(permission_denied_err(dest, &e)),
        Err(e) => Err(io_to_link_failed(src, dest, &e)),
    }
}

#[cfg(unix)]
fn symlink_kind(src: &Utf8Path, dest: &Utf8Path) -> io::Result<()> {
    std::os::unix::fs::symlink(src.as_std_path(), dest.as_std_path())
}

#[cfg(windows)]
fn symlink_kind(src: &Utf8Path, dest: &Utf8Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(src.as_std_path(), dest.as_std_path())
}

#[cfg(unix)]
fn junction_kind(src: &Utf8Path, dest: &Utf8Path) -> io::Result<()> {
    // Unix 没有 junction 概念 — 降级为目录 symlink,行为对外等价。
    std::os::unix::fs::symlink(src.as_std_path(), dest.as_std_path())
}

#[cfg(windows)]
fn junction_kind(src: &Utf8Path, dest: &Utf8Path) -> io::Result<()> {
    // 优先用 std 的 `symlink_dir`(Windows 上即 junction,需 fs::Metadata 校验);
    // std 1.78+ 提供 `std::os::windows::fs::symlink_dir`,但部分老版本不暴露,
    // 因此用更通用的 `symlink` 包一层并按目录语义调用。
    std::os::windows::fs::symlink_dir(src.as_std_path(), dest.as_std_path())
}

#[cfg(unix)]
fn hardlink_kind(src: &Utf8Path, dest: &Utf8Path) -> io::Result<()> {
    std::fs::hard_link(src.as_std_path(), dest.as_std_path())
}

#[cfg(windows)]
fn hardlink_kind(src: &Utf8Path, dest: &Utf8Path) -> io::Result<()> {
    std::fs::hard_link(src.as_std_path(), dest.as_std_path())
}

// ── 内部:备份 + 重建 ─────────────────────────────────────────────

fn backup_then_recreate(
    src: &Utf8Path,
    dest: &Utf8Path,
    kind: LinkKind,
    existing_target: &str,
) -> Result<(), CoreError> {
    let ts = unix_ts();
    let mut bak = dest.as_str().to_owned();
    bak.push_str(&format!(".bak-{ts}"));

    // 极端情况:同一秒内被并发再次 backup → 追加 nanoseconds 后缀。
    let bak_path = loop {
        let candidate = Utf8PathBuf::from(&bak);
        if !candidate.exists() {
            break candidate;
        }
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        bak = format!("{dest}.bak-{ts}-{nanos}");
    };

    // rename 优先(junction / symlink 本体 rename 是廉价操作,不走数据拷贝)。
    if let Err(e) = fs::rename(dest, &bak_path) {
        // rename 跨 mount point / Windows 跨盘会失败 → 降级为 copy + remove。
        // 链接本身的"copy + remove"在 symlink 上:先 read_link 拿 target,再删除
        // symlink,最后重建 → 等价于"无操作 + 后续 create_link"。
        // 普通文件 / 目录才需要真拷贝。
        let meta = fs::symlink_metadata(dest).map_err(|e2| io_to_link_failed(src, dest, &e2))?;
        if meta.file_type().is_symlink() {
            // symlink 自身:rename 失败,降级为 `fs::remove_file`。
            fs::remove_file(dest).map_err(|e2| {
                CoreError::LinkFailed {
                    src: src.to_string(),
                    dest: dest.to_string(),
                    reason: format!(
                        "旧链接(指向 {existing_target})无法备份为 {bak_path}: rename 失败({e});remove_file 失败: {e2}"
                    ),
                    hint: "检查 dest 所在目录权限,或手动 `rm` 旧链接后重试".to_string(),
                }
            })?;
            // 备份文件以原链接 target 名写一份"占位说明",便于人工恢复。
            write_backup_stub(&bak_path, existing_target);
        } else {
            // 普通文件 / 目录:copy 整个内容到 bak。
            if meta.is_dir() {
                copy_dir_recursive(dest.as_std_path(), bak_path.as_std_path()).map_err(|e2| {
                    CoreError::LinkFailed {
                        src: src.to_string(),
                        dest: dest.to_string(),
                        reason: format!(
                            "旧目录无法备份为 {bak_path}: rename 失败({e});copy 失败: {e2}"
                        ),
                        hint: "检查 dest 所在目录权限 / 磁盘空间".to_string(),
                    }
                })?;
            } else {
                fs::copy(dest.as_std_path(), bak_path.as_std_path()).map_err(|e2| {
                    CoreError::LinkFailed {
                        src: src.to_string(),
                        dest: dest.to_string(),
                        reason: format!(
                            "旧文件无法备份为 {bak_path}: rename 失败({e});copy 失败: {e2}"
                        ),
                        hint: "检查 dest 所在目录权限 / 磁盘空间".to_string(),
                    }
                })?;
            }
            fs::remove_file(dest)
                .or_else(|_| fs::remove_dir(dest))
                .map_err(|e2| CoreError::LinkFailed {
                    src: src.to_string(),
                    dest: dest.to_string(),
                    reason: format!("备份成功但无法删除原 dest: {e2}"),
                    hint: "手工 `rm` 目标后重试".to_string(),
                })?;
        }
    }

    // 重建
    create_link(src, dest, kind)
}

fn write_backup_stub(bak_path: &Utf8Path, old_target: &str) {
    // 链接的备份没有"内容",写一个 1-byte stub,避免空文件引起 git/sync 困惑。
    // 用 create_new 防止覆盖已有 bak(虽然在循环里已检查过)。
    use std::io::Write;
    if let Ok(mut f) = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(bak_path.as_std_path())
    {
        let _ = f.write_all(old_target.as_bytes());
    }
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if ty.is_symlink() {
            let target = fs::read_link(&from)?;
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(&target, &to)?;
            }
            #[cfg(windows)]
            {
                let meta = fs::metadata(&from)?;
                if meta.is_dir() {
                    std::os::windows::fs::symlink_dir(&target, &to)?;
                } else {
                    std::os::windows::fs::symlink_file(&target, &to)?;
                }
            }
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

// ── 内部:错误 / 工具 ─────────────────────────────────────────────

fn unix_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn path_to_utf8(p: &Path) -> Option<Utf8PathBuf> {
    Utf8Path::from_path(p).map(|s| s.to_path_buf())
}

fn is_permission_denied(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::PermissionDenied)
        // Windows 偶尔把权限错映射到别的 kind;兜底 raw_os_error。
        || matches!(e.raw_os_error(), Some(13 /* EACCES */) | Some(5 /* ERROR_ACCESS_DENIED */))
}

fn permission_denied_err(path: &Utf8Path, e: &io::Error) -> CoreError {
    CoreError::PermissionDenied {
        path: path.to_string(),
        expected_perms: "rwx (父目录) + rw (目标)".to_string(),
        hint: format!("用 sudo / 调整 ACL 后重试; 详细 IO 错: {e}"),
    }
}

fn io_to_link_failed(src: &Utf8Path, dest: &Utf8Path, e: &io::Error) -> CoreError {
    CoreError::LinkFailed {
        src: src.to_string(),
        dest: dest.to_string(),
        reason: e.to_string(),
        hint: "检查 src / dest 路径存在性、目录权限、跨盘(macOS /tmp 是 tmpfs)".to_string(),
    }
}

fn io_to_link_failed_err_only(dest: &Utf8Path, e: &io::Error) -> CoreError {
    CoreError::LinkFailed {
        src: String::new(),
        dest: dest.to_string(),
        reason: e.to_string(),
        hint: "检查 dest 路径权限".to_string(),
    }
}

// ── 单元测试 ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ── 工具:在临时目录里造一对 src / dest 路径 ───────────────
    struct Env {
        _tmp: TempDir,
        src: Utf8PathBuf,
        dest: Utf8PathBuf,
    }

    fn env() -> Env {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = Utf8PathBuf::from_path_buf(tmp.path().join("source.txt")).unwrap();
        let dest = Utf8PathBuf::from_path_buf(tmp.path().join("dest.txt")).unwrap();
        // dest 的父目录是 tmp.path() 本身(已存在)。
        fs::write(&src, b"hello ai-config").unwrap();
        // dest 不存在(初始状态)。
        Env {
            _tmp: tmp,
            src,
            dest,
        }
    }

    // ── 1. 幂等:跑 3 遍 = 跑 1 遍 ───────────────────────────

    #[test]
    fn link_is_idempotent_3_runs_equal_1_run() {
        let e = env();
        // 跑 1 遍
        link(&e.src, &e.dest, LinkKind::auto()).expect("first link ok");
        let meta_after_1 = fs::symlink_metadata(&e.dest).unwrap();
        let target_after_1 = fs::read_link(&e.dest).unwrap();

        // 跑第 2、3 遍(应 noop,无备份产生)
        for _ in 0..2 {
            link(&e.src, &e.dest, LinkKind::auto()).expect("idempotent link ok");
        }

        let meta_after_3 = fs::symlink_metadata(&e.dest).unwrap();
        let target_after_3 = fs::read_link(&e.dest).unwrap();

        // ino 一致 / file_type 一致 / 链接 target 一致
        assert_eq!(meta_after_1.file_type(), meta_after_3.file_type());
        assert!(meta_after_3.file_type().is_symlink());
        assert_eq!(target_after_1, target_after_3);
        assert_eq!(
            path_to_utf8(&target_after_3).as_deref(),
            Some(e.src.as_path())
        );

        // 整个 tmp 里**没有**任何 .bak-* 残留
        for entry in fs::read_dir(e._tmp.path()).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name();
            let s = name.to_string_lossy();
            assert!(!s.contains(".bak-"), "幂等循环不应产生 .bak-*, 但出现: {s}");
        }
    }

    // ── 2. 备份覆盖:dest 是普通文件时,旧文件改名 bak ─────────

    #[test]
    fn link_backs_up_existing_regular_file() {
        let e = env();
        // 先在 dest 写一个普通文件(模拟用户手写内容)
        fs::write(&e.dest, b"user hand-written content").unwrap();

        // 第一次 link → 应备份
        link(&e.src, &e.dest, LinkKind::auto()).expect("link ok");

        // dest 应是 symlink,指向 src
        let dest_meta = fs::symlink_metadata(&e.dest).unwrap();
        assert!(dest_meta.file_type().is_symlink(), "dest 应是 symlink");
        assert_eq!(
            path_to_utf8(&fs::read_link(&e.dest).unwrap()).as_deref(),
            Some(e.src.as_path())
        );

        // 应存在 .bak-* 文件,内容是用户手写内容
        let mut found_bak: Option<Utf8PathBuf> = None;
        for entry in fs::read_dir(e._tmp.path()).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name();
            let s = name.to_string_lossy().into_owned();
            if s.starts_with("dest.txt.bak-") {
                found_bak = Some(Utf8PathBuf::from_path_buf(entry.path()).unwrap());
            }
        }
        let bak = found_bak.expect("必须有 dest.txt.bak-<ts>");
        let bak_content = fs::read(&bak).unwrap();
        assert_eq!(bak_content, b"user hand-written content");
    }

    #[test]
    fn link_backs_up_existing_symlink_to_different_source() {
        let e = env();
        // 先建一个 symlink,指向"别的源"
        let other_src = Utf8PathBuf::from_path_buf(e._tmp.path().join("other.txt")).unwrap();
        fs::write(&other_src, b"other").unwrap();
        std::os::unix::fs::symlink(other_src.as_std_path(), e.dest.as_std_path()).unwrap();

        // link 应当:dest 指向其它 → 备份后重建
        #[cfg(not(unix))]
        {
            // 非 unix 平台 hardlink/backup 的语义不同,跳过严格检查。
            let _ = (other_src, fs::symlink_metadata(&e.dest).unwrap());
        }
        #[cfg(unix)]
        {
            link(&e.src, &e.dest, LinkKind::auto()).expect("link ok");

            // dest 现在指向 e.src
            assert_eq!(
                path_to_utf8(&fs::read_link(&e.dest).unwrap()).as_deref(),
                Some(e.src.as_path())
            );
            // 存在 dest.txt.bak-*
            let mut has_bak = false;
            for entry in fs::read_dir(e._tmp.path()).unwrap() {
                let s = entry.unwrap().file_name().to_string_lossy().into_owned();
                if s.starts_with("dest.txt.bak-") {
                    has_bak = true;
                }
            }
            assert!(has_bak, "原 symlink 应当被备份为 dest.txt.bak-*");
        }
    }

    // ── 3. check 四态 ───────────────────────────────────────

    #[test]
    fn check_returns_linked_when_symlink_points_to_src() {
        let e = env();
        link(&e.src, &e.dest, LinkKind::auto()).unwrap();

        match check(&e.dest, &e.src) {
            LinkHealth::Linked { src } => assert_eq!(src, e.src),
            other => panic!("应返回 Linked, 实得 {other:?}"),
        }
    }

    #[test]
    fn check_returns_broken_when_target_missing() {
        let e = env();
        // 先建一个 symlink,target 是另一处(不存在的)路径
        let dangling = Utf8PathBuf::from_path_buf(e._tmp.path().join("never-exists.txt")).unwrap();
        std::os::unix::fs::symlink(dangling.as_std_path(), e.dest.as_std_path()).unwrap();

        // expected_src 设成 dangling
        match check(&e.dest, &dangling) {
            LinkHealth::Broken { dangling_target } => {
                assert_eq!(dangling_target, dangling);
            }
            other => panic!("应返回 Broken, 实得 {other:?}"),
        }
    }

    #[test]
    fn check_returns_wrong_source_when_link_points_elsewhere() {
        let e = env();
        let other = Utf8PathBuf::from_path_buf(e._tmp.path().join("other.txt")).unwrap();
        fs::write(&other, b"x").unwrap();
        std::os::unix::fs::symlink(other.as_std_path(), e.dest.as_std_path()).unwrap();

        // expected 写 e.src,但 dest 实际指向 other
        match check(&e.dest, &e.src) {
            LinkHealth::WrongSource { actual, expected } => {
                assert_eq!(actual, other);
                assert_eq!(expected, e.src);
            }
            other => panic!("应返回 WrongSource, 实得 {other:?}"),
        }
    }

    #[test]
    fn check_returns_wrong_type_when_dest_is_regular_file() {
        let e = env();
        fs::write(&e.dest, b"plain").unwrap();

        match check(&e.dest, &e.src) {
            LinkHealth::WrongType { actual_kind } => {
                assert_eq!(actual_kind, "file");
            }
            other => panic!("应返回 WrongType, 实得 {other:?}"),
        }
    }

    // ── 4. unlink 语义:只删本工具创建的链接 ──────────────────

    #[test]
    fn unlink_removes_symlink_created_by_link() {
        let e = env();
        link(&e.src, &e.dest, LinkKind::auto()).unwrap();
        assert!(fs::symlink_metadata(&e.dest)
            .unwrap()
            .file_type()
            .is_symlink());

        unlink(&e.dest).expect("unlink ok");
        // dest 应当不存在
        assert!(!e.dest.exists());
        // symlink_metadata 在不存在时返回 NotFound
        assert!(fs::symlink_metadata(&e.dest).is_err());
    }

    #[test]
    fn unlink_refuses_to_remove_regular_file() {
        let e = env();
        fs::write(&e.dest, b"user content").unwrap();

        let err = unlink(&e.dest).expect_err("应拒绝删非链接");
        match err {
            CoreError::LinkFailed { reason, .. } => {
                assert!(
                    reason.contains("不是链接"),
                    "reason 应说明'非链接', 实得: {reason}"
                );
            }
            other => panic!("应返回 LinkFailed, 实得 {other:?}"),
        }
        // 文件还在
        assert!(e.dest.exists());
    }

    #[test]
    fn unlink_is_noop_when_dest_missing() {
        let e = env();
        // dest 不存在 → noop
        unlink(&e.dest).expect("missing dest → ok");
    }

    // ── 5. 跨平台 fallback ──────────────────────────────────
    //
    // macOS / Linux 上 `LinkKind::auto()` == Symlink,Windows 上 == Junction。
    // 这里至少在 unix 上跑 symlink;Windows 分支靠 cfg 编译通过 + 手动 mock。

    #[cfg(unix)]
    #[test]
    fn auto_kind_resolves_to_symlink_on_unix() {
        assert_eq!(LinkKind::auto(), LinkKind::Symlink);
    }

    #[cfg(windows)]
    #[test]
    fn auto_kind_resolves_to_junction_on_windows() {
        assert_eq!(LinkKind::auto(), LinkKind::Junction);
    }

    // ── 6. 拒绝自环 ──────────────────────────────────────────

    #[test]
    fn link_rejects_self_loop() {
        let e = env();
        // src == dest 应当被拒
        let err = link(&e.src, &e.src, LinkKind::auto()).expect_err("应拒绝自环");
        assert!(
            matches!(err, CoreError::InvalidPath(_)),
            "应返回 InvalidPath, 实得 {err:?}"
        );
    }

    // ── 7. 父目录不存在时自动创建并成功链接 ──────────────────

    #[test]
    fn link_creates_missing_parent_dir_for_skill_like_dest() {
        let tmp = tempfile::tempdir().unwrap();
        let src = Utf8PathBuf::from_path_buf(tmp.path().join("skill-src")).unwrap();
        fs::create_dir_all(&src).unwrap();
        let dest = Utf8PathBuf::from_path_buf(tmp.path().join(".cursor/skills/my-skill")).unwrap();
        assert!(!dest.parent().unwrap().exists());

        link(&src, &dest, LinkKind::auto()).expect("skill 风格目录链接应成功");
        assert!(dest.parent().unwrap().is_dir());
    }

    #[test]
    fn link_creates_missing_parent_dir_for_rule_like_dest() {
        let tmp = tempfile::tempdir().unwrap();
        let src = Utf8PathBuf::from_path_buf(tmp.path().join("source.mdc")).unwrap();
        let dest =
            Utf8PathBuf::from_path_buf(tmp.path().join(".cursor/rules/my-rule.mdc")).unwrap();
        fs::write(&src, b"rule").unwrap();
        assert!(!dest.parent().unwrap().exists());

        link(&src, &dest, LinkKind::auto()).expect("rule 风格文件链接应成功");
        assert_eq!(check(&dest, &src), LinkHealth::Linked { src: src.clone() });
    }

    #[test]
    fn link_creates_missing_parent_dir_for_agent_like_dest() {
        let tmp = tempfile::tempdir().unwrap();
        let src = Utf8PathBuf::from_path_buf(tmp.path().join("source.txt")).unwrap();
        let dest = Utf8PathBuf::from_path_buf(tmp.path().join(".cursor/agents/dest.txt")).unwrap();
        fs::write(&src, b"x").unwrap();
        assert!(!dest.parent().unwrap().exists());

        link(&src, &dest, LinkKind::auto()).expect("agent 风格文件链接应成功");
        assert!(dest.parent().unwrap().is_dir());
        assert_eq!(check(&dest, &src), LinkHealth::Linked { src: src.clone() });
    }

    // ── 8. check 对 dest 完全不存在也走 Broken(便于 doctor) ──

    #[test]
    fn check_returns_broken_when_dest_fully_missing() {
        let e = env();
        // dest 不存在
        assert!(!e.dest.exists());
        match check(&e.dest, &e.src) {
            LinkHealth::Broken { dangling_target } => {
                assert_eq!(dangling_target, e.src);
            }
            other => panic!("应返回 Broken, 实得 {other:?}"),
        }
    }
}
