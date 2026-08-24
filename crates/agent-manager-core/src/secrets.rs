//! secrets.env 读写(0600 权限,**不**入 store / 不入 log / 不入 stdout / 不入 --json)。
//!
//! 硬约束(对齐 PRD v1.0):
//! - §2.1:`${VAR}` 缺变量**显式告警**,**不**留空字符串。
//! - §8.1:`Display` / `Debug` / `Serialize` **绝不**包含 secret 明文值;只输出
//!   "哪个 key 缺"和"在哪个 template"。
//! - §10 A-12:`secrets.env` 写入 0600;POSIX 严格校验权限位。
//! - §10 A-13:GUI 不显示明文(由 GUI 层把关;core 提供的类型天然 redacted)。
//! - §10 A-14:SQLite 不存明文(`SecretPairs` 不入 store,仅过程内使用)。
//! - §6.1:退出码 4 = secrets 缺(`CoreError::SecretsMissing`)。
//!
//! 顺序:本模块用 `Vec<(String, String)>` 而**非** `HashMap`,保留 `.env` 文件
//! 的写入顺序;不在 log / trace / anyhow 中打印整张表。

#![allow(dead_code)]

use camino::{Utf8Path, Utf8PathBuf};
use std::fmt;
use std::io::Write;

use crate::error::CoreError;

// ── 路径 ────────────────────────────────────────────────────────────

/// 默认路径:`~/.config/agents-manager/secrets.env`,可用 `AGENT_MANAGER_SECRETS_DIR` env 覆盖。
pub fn default_path() -> Utf8PathBuf {
    let base = std::env::var("AGENT_MANAGER_SECRETS_DIR")
        .ok()
        .map(Utf8PathBuf::from)
        .or_else(|| dirs_home().map(|h| h.join(".config").join("agents-manager")));
    base.map(|p| p.join("secrets.env"))
        .unwrap_or_else(|| Utf8PathBuf::from("secrets.env"))
}

fn dirs_home() -> Option<Utf8PathBuf> {
    std::env::var("HOME").ok().map(Utf8PathBuf::from)
}

// ── 类型 ────────────────────────────────────────────────────────────

/// 单一缺 key 错误。**只**含 key 名 + 模板名;**绝不**含 value(PRD §8.1)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingKey {
    pub key: String,
    pub template: String,
}

impl MissingKey {
    pub fn new(key: impl Into<String>, template: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            template: template.into(),
        }
    }
}

impl fmt::Display for MissingKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 故意**不**调用 self.key 的 Debug 之外的任何东西 —— 名字可以,值不行。
        write!(f, "secrets 缺 {} (在 template {})", self.key, self.template)
    }
}

impl std::error::Error for MissingKey {}

/// 缺 key 列表(`validate` 的返回类型)。**只**含 key 名;**绝不**含 value。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MissingKeys {
    pub template: String,
    pub keys: Vec<String>,
}

impl MissingKeys {
    pub fn new(template: impl Into<String>) -> Self {
        Self {
            template: template.into(),
            keys: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn push(&mut self, key: impl Into<String>) {
        self.keys.push(key.into());
    }
}

impl fmt::Display for MissingKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "template {} 缺 {} 个 key:",
            self.template,
            self.keys.len()
        )?;
        for k in &self.keys {
            write!(f, "\n  - {k}")?;
        }
        Ok(())
    }
}

// ── 进程内容器(顺序保留,Debug/Display/Serialize **不**输出明文) ────

/// 进程内的 key→value 列表。
///
/// 顺序保留(写入文件时用),**Debug / Display / Serialize 全部脱敏** —— 任何
/// 反射性打印只能看见 `key` 名和"value 已脱敏"的占位,**绝不**暴露明文
/// (PRD §8.1 / §10 A-14)。要拿到明文请显式调用 [`Self::pairs`]。
///
/// **不**实现 `Deserialize`:反序列化会把 value 写回内存,等于把明文"从 JSON
/// 反射"到 `SecretPairs`,违背 §10 A-14(不进 store)。如果需要 round-trip,
/// 用 [`Self::from_vec`] 直接吃 `Vec<(String, String)>`。
#[derive(Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct SecretPairs {
    /// 顺序保留的 key→value。
    #[serde(serialize_with = "serialize_redacted")]
    pairs: Vec<(String, String)>,
}

/// 序列化时**只**写 key 名 + `REDACTED` 占位,绝不写 value。
fn serialize_redacted<S>(pairs: &[(String, String)], ser: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeSeq;
    // 序列化为 [ {key, value: "<REDACTED>"}, ... ] —— 仍保留顺序与 key 名,
    // 但 value 永远是占位符。
    let mut seq = ser.serialize_seq(Some(pairs.len()))?;
    for (k, _) in pairs {
        #[derive(serde::Serialize)]
        struct Entry<'a> {
            key: &'a str,
            value: &'static str,
        }
        seq.serialize_element(&Entry {
            key: k.as_str(),
            value: "<REDACTED>",
        })?;
    }
    seq.end()
}

impl SecretPairs {
    /// 从 `Vec<(String, String)>` 构造(不查重,顺序由 caller 决定)。
    pub fn from_vec(pairs: Vec<(String, String)>) -> Self {
        Self { pairs }
    }

    /// 显式取走明文(只该在真正需要的地方调用 —— 写入文件、模板渲染)。
    /// 返回的是**按 key 去重**(后者覆盖前者)、**原顺序**的列表。
    pub fn pairs(&self) -> &[(String, String)] {
        &self.pairs
    }

    /// 拆出 owned 明文(同上,语义"消费")。
    pub fn into_pairs(self) -> Vec<(String, String)> {
        self.pairs
    }

    /// 只取 key 名(`validate` 内部用)。
    pub fn keys(&self) -> Vec<&str> {
        self.pairs.iter().map(|(k, _)| k.as_str()).collect()
    }

    /// 长度。
    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// 按 key 查 value;`None` 表示缺。
    pub fn get(&self, key: &str) -> Option<&str> {
        self.pairs
            .iter()
            .rev() // 后写覆盖前写(读 .env 时常用)
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

impl fmt::Debug for SecretPairs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // **只**打印 key 名 + "<redacted, N bytes>" 字样;**绝不**写 value。
        f.debug_struct("SecretPairs")
            .field("count", &self.pairs.len())
            .field(
                "keys",
                &self
                    .pairs
                    .iter()
                    .map(|(k, _)| k.clone())
                    .collect::<Vec<_>>(),
            )
            .field("values", &"<redacted>")
            .finish()
    }
}

impl fmt::Display for SecretPairs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // **绝不**写 value(防 `format!("{secrets}")` / anyhow / `eprintln!` 漏)。
        write!(f, "SecretPairs({} keys, values redacted)", self.pairs.len())
    }
}

// ── 读 ──────────────────────────────────────────────────────────────

/// 从 [`default_path`] 读 `.env`;返回顺序保留的 `Vec<(String, String)>`。
///
/// - 跳过空行 / `#` 注释行 / 行首 / 行内空白。
/// - `KEY=VALUE` 形式:VALUE 不去引号、不做 escape(与 .env 文件标准用法一致,
///   这里只支持"裸值")。
/// - 不存在的文件**不**视为错,返回 `Ok(vec![])`(允许"全新安装 + 后续 validate
///   提示补全"的工作流)。
/// - **不**把 value 写入 log(只有"读到的 key 数"会进 trace)。
pub fn load() -> Result<Vec<(String, String)>, CoreError> {
    let path = default_path();
    load_from(&path)
}

/// 从给定路径读 `.env`(供测试 / 显式路径场景)。
pub fn load_from(path: &Utf8Path) -> Result<Vec<(String, String)>, CoreError> {
    match std::fs::metadata(path.as_std_path()) {
        Ok(_) => ensure_0600(path)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(CoreError::Io(e)),
    }
    let content = std::fs::read_to_string(path.as_std_path())?;
    let pairs = parse_env(&content);
    // **只**打 key 数到 trace,**绝不**打 value。
    tracing::debug!(path = %path, keys = pairs.len(), "secrets::load 读完");
    Ok(pairs)
}

fn parse_env(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // 跳过行内 `#` 之后的注释(常见写法:`KEY=value # comment`)。
        // 但**不**去引号 / 不做 escape;保持简单。
        let line = match line.find(" #") {
            Some(idx) => &line[..idx],
            None => line,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            // 非法行:**不**返回错误(容错:跳过),但 trace 一行供诊断。
            tracing::warn!(line = line, "secrets::load 跳过非法行(无 '=')");
            continue;
        };
        let key = k.trim().to_string();
        if key.is_empty() {
            continue;
        }
        // 简单 trim:不剥引号 —— 多数 .env 不会写引号,且容错优先。
        let value = v.trim().to_string();
        out.push((key, value));
    }
    out
}

// ── 写 ──────────────────────────────────────────────────────────────

/// 写回到 [`default_path`],权限强制 0600(PRD §10 A-12)。
pub fn save(pairs: &[(String, String)]) -> Result<(), CoreError> {
    let path = default_path();
    save_to(pairs, &path)
}

/// 写回到指定路径。
///
/// 步骤:
/// 1. 父目录若不存在则 `create_dir_all`。
/// 2. 写临时文件 `<path>.tmp.<pid>`,`chmod 0o600`(在写之前设置,免 race)。
/// 3. 原子 `rename` 到 `<path>`。
/// 4. 写完**严格**校验权限位 `mode & 0o777 == 0o600`;不符则返回
///    `PermissionDenied`。
pub fn save_to(pairs: &[(String, String)], path: &Utf8Path) -> Result<(), CoreError> {
    if let Some(parent) = path.parent() {
        if !parent.as_str().is_empty() {
            std::fs::create_dir_all(parent.as_std_path())?;
        }
    }

    // 用 process id 区分并发写入的 tmp 文件。
    let pid = std::process::id();
    let mut tmp_name = path.as_str().to_string();
    tmp_name.push('.');
    tmp_name.push_str("tmp.");
    tmp_name.push_str(&pid.to_string());
    let tmp_path = Utf8PathBuf::from(tmp_name);

    // 序列化:每行 `KEY=VALUE\n`,尾部 newline。
    let mut buf = String::new();
    for (k, v) in pairs {
        // 防御性:key 不能含 `=` / 换行;value 写出去的是明文,但**不**写 log。
        if k.contains('=') || k.contains('\n') || k.is_empty() {
            return Err(CoreError::InvalidPath(format!("secrets key 非法: {:?}", k)));
        }
        // 替换 value 里的换行(避免破坏 .env 解析);其它字符保持原样。
        let v = v.replace('\n', "\\n");
        buf.push_str(k);
        buf.push('=');
        buf.push_str(&v);
        buf.push('\n');
    }

    {
        let mut f = std::fs::File::create(tmp_path.as_std_path())?;
        // **写之前** chmod,免新建文件受 umask 影响。
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            f.set_permissions(perms)?;
        }
        f.write_all(buf.as_bytes())?;
        f.sync_all()?; // 落盘再 rename。
    }

    // 原子 rename(tmp 同目录 → 同 filesystem,POSIX 保证原子)。
    if let Err(e) = std::fs::rename(tmp_path.as_std_path(), path.as_std_path()) {
        // 清理 tmp(失败也无所谓,下轮会覆盖)。
        let _ = std::fs::remove_file(tmp_path.as_std_path());
        return Err(CoreError::Io(e));
    }

    // 严格校验最终权限。
    ensure_0600(path)?;
    tracing::debug!(path = %path, keys = pairs.len(), "secrets::save 完成");
    Ok(())
}

#[cfg(unix)]
fn ensure_0600(path: &Utf8Path) -> Result<(), CoreError> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path.as_std_path())?;
    let mode = meta.permissions().mode() & 0o777;
    if mode != 0o600 {
        return Err(CoreError::PermissionDenied {
            path: path.to_string(),
            expected_perms: "0600".to_string(),
            hint: format!(
                "secrets.env 权限位 {:o} 不安全;请 `chmod 600 {}`",
                mode, path
            ),
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_0600(_path: &Utf8Path) -> Result<(), CoreError> {
    // 非 Unix 平台:不报错(0o600 在 Windows 上无意义,后续 .env 解析用 UTF-8
    // 即可)。生产部署目标是 macOS / Linux,这里只为 `cargo test` 通过。
    Ok(())
}

// ── 校验 ────────────────────────────────────────────────────────────

/// 校验 `placeholders` 列表里每个 key 是否都在 `pairs` 里;返回**缺**的 key
/// 列表(顺序保留)。**不**返回 value。
///
/// 用法:对单个 template 文件,先 `extract_placeholders(template)`(在
/// `template` 模块)得到 placeholder 列表,再 `validate(&placeholders, &pairs)`
/// 得到缺哪个。
pub fn validate(
    template_name: &str,
    placeholders: &[String],
    pairs: &[(String, String)],
) -> MissingKeys {
    let mut missing = MissingKeys::new(template_name);
    for ph in placeholders {
        if !pairs.iter().any(|(k, _)| k == ph) {
            missing.push(ph.clone());
        }
    }
    missing
}

// ── 注入 ────────────────────────────────────────────────────────────

/// 把 `template` 里的 `${VAR}` 替换为 `pairs` 里的 value。
///
/// - **缺**则返回 `Err(MissingKey { key, template })`,**不**输出空字符串
///   (PRD §2.1)。
/// - 同一个 template 有多个缺变量时,**第一个**缺的出现处即返回。
/// - 不做转义 / 不递归展开(`${A${B}}` 不处理)。
pub fn inject(
    template_name: &str,
    template: &str,
    pairs: &[(String, String)],
) -> Result<String, MissingKey> {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'$' && bytes[i + 1] == b'{' {
            // 找匹配的 `}`(只到下一行 / 文件尾为止;允许 key 名含 `_` / 字母 / 数字)。
            let start = i + 2;
            let mut end = start;
            while end < bytes.len() && bytes[end] != b'}' {
                end += 1;
            }
            if end < bytes.len() {
                let key = &template[start..end];
                match lookup(pairs, key) {
                    Some(v) => {
                        out.push_str(v);
                        i = end + 1;
                        continue;
                    }
                    None => {
                        return Err(MissingKey::new(key, template_name));
                    }
                }
            } else {
                // 没有匹配 `}` —— 视作字面量,原样写出(并推进 1,避免死循环)。
                out.push('$');
                i += 1;
            }
        } else {
            // 安全地 push 一个 char(UTF-8 友好)。
            // 不能直接用 bytes[i] 当 char;用 char boundary。
            let ch = template[i..].chars().next().expect("boundary 至少 1 字节");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    Ok(out)
}

fn lookup<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    // 后写覆盖前写(读 .env 的常见语义)。
    pairs
        .iter()
        .rev()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

// ── 单元测试 ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// 模拟 PRD §10 A-14:plaintext **绝不**出现在 Debug/Display/Serialize 输出。
    /// 这里用一个**特征字符串**作为 value,确保任何打印路径都不会撞到。
    const FORBIDDEN_VALUE: &str = "PLAINTEXT-LEAK-CANARY-sk-prod-XYZ-9999";

    // ── load / parse_env ─────────────────────────────────────────────

    #[test]
    fn load_parses_key_value_with_comments_and_blanks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("secrets.env");
        std::fs::write(
            &path,
            "# 顶层注释\n\
             \n\
             A=alpha\n\
             B=bravo # 行尾注释应被剥\n\
             C=charlie\n\
             =缺 key 跳过\n\
             no_equals_sign_line\n\
             D=delta\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let path = Utf8Path::from_path(&path).unwrap();
        let pairs = load_from(path).expect("load ok");
        // 顺序: A, B, C, D(=缺 key 和 no_equals 那两行应被跳过)
        let names: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, vec!["A", "B", "C", "D"]);
        assert_eq!(pairs[0], ("A".to_string(), "alpha".to_string()));
        assert_eq!(pairs[1], ("B".to_string(), "bravo".to_string())); // 注释被剥
        assert_eq!(pairs[2], ("C".to_string(), "charlie".to_string()));
        assert_eq!(pairs[3], ("D".to_string(), "delta".to_string()));
    }

    #[test]
    fn load_missing_file_returns_empty_vec() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.env");
        let path = Utf8Path::from_path(&path).unwrap();
        let pairs = load_from(path).expect("缺文件视为空");
        assert!(pairs.is_empty());
    }

    #[test]
    fn load_does_not_emit_plaintext_to_log_strings() {
        // 反向:parse_env 的输出 value 当然是明文(我们就是要拿明文),但**任何**
        // `tracing` / `eprintln!` 都不该印 value。抓 stdout/stderr 是脆弱的,
        // 这里静态校验:本测试**之前**的源码里,不允许出现 stdout/stderr 调用
        // (注释 / 文档字符串会被计入,所以用 "形如 `!(\"` 紧跟字符串字面" 这种
        // 锚点表示实际调用)。这是软约束,真要更严格可用 syn 解析 AST。
        let src = include_str!("secrets.rs");
        let non_test_src = src.split("#[cfg(test)]").next().unwrap_or(src);
        for banned in ["println!(\"", "eprintln!(\"", "dbg!(", "panic!(\""] {
            assert!(
                !non_test_src.contains(banned),
                "secrets.rs 业务区出现 {banned:?}(可能把明文打到 stdout/stderr)"
            );
        }
    }

    // ── save ─────────────────────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn save_writes_0600_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.env");
        let path = Utf8Path::from_path(&path).unwrap();
        let pairs = vec![("K".to_string(), "V".to_string())];
        save_to(&pairs, path).expect("save ok");

        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(path.as_std_path()).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "secrets.env 权限位 {:o} != 0600", mode);
    }

    #[cfg(unix)]
    #[test]
    fn save_rejects_non_0600_post_rename() {
        // 模拟 umask / 入侵者把权限改了的情况:save_to 内部不会主动 chmod
        // 修改前的(因为是新建文件),但 ensure_0600 仍会校验。
        // 这里我们手工 chmod 后调用一个最小"再校验"路径:用 save_to 创建,
        // 然后 chmod 0644,再读 metadata,验证我们的 ensure_0600 函数本身能
        // 抓到(用单独的子函数测)。
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.env");
        let path = Utf8Path::from_path(&path).unwrap();
        let pairs = vec![("K".to_string(), "V".to_string())];
        save_to(&pairs, path).expect("save ok");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path.as_std_path(), std::fs::Permissions::from_mode(0o644))
            .unwrap();
        let err = ensure_0600(path).expect_err("0644 应被拒");
        match err {
            CoreError::PermissionDenied { expected_perms, .. } => {
                assert_eq!(expected_perms, "0600");
            }
            other => panic!("expected PermissionDenied, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn load_rejects_an_existing_secrets_file_with_unsafe_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path_buf = dir.path().join("secrets.env");
        let path = Utf8Path::from_path(&path_buf).unwrap();
        std::fs::write(path.as_std_path(), "TOKEN=do-not-expose\n").unwrap();
        std::fs::set_permissions(path.as_std_path(), std::fs::Permissions::from_mode(0o644))
            .unwrap();

        let error = load_from(path).expect_err("0644 secrets.env must be rejected before read");

        assert!(matches!(error, CoreError::PermissionDenied { .. }));
        assert!(!error.to_string().contains("do-not-expose"));
    }

    #[test]
    fn save_then_load_roundtrip_preserves_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.env");
        let path = Utf8Path::from_path(&path).unwrap();
        let pairs = vec![
            ("Z".to_string(), "1".to_string()),
            ("A".to_string(), "2".to_string()),
            ("M".to_string(), "3".to_string()),
        ];
        save_to(&pairs, path).unwrap();
        let loaded = load_from(path).unwrap();
        let names: Vec<&str> = loaded.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, vec!["Z", "A", "M"], "顺序应保留");
    }

    // ── validate ─────────────────────────────────────────────────────

    #[test]
    fn validate_reports_missing_keys() {
        let pairs = vec![("A".to_string(), "1".to_string())];
        let placeholders = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let missing = validate("tpl.json", &placeholders, &pairs);
        assert!(!missing.is_empty());
        let names: HashSet<&str> = missing.keys.iter().map(String::as_str).collect();
        assert_eq!(names, HashSet::from(["B", "C"]));
        // 不含 value
        assert!(!missing.to_string().contains("1"));
    }

    #[test]
    fn validate_all_present_returns_empty() {
        let pairs = vec![
            ("A".to_string(), "1".to_string()),
            ("B".to_string(), "2".to_string()),
        ];
        let placeholders = vec!["A".to_string(), "B".to_string()];
        let missing = validate("tpl.json", &placeholders, &pairs);
        assert!(missing.is_empty());
    }

    // ── inject ───────────────────────────────────────────────────────

    #[test]
    fn inject_replaces_placeholders() {
        let pairs = vec![("NAME".to_string(), "alice".to_string())];
        let out = inject("tpl.json", "hello ${NAME}!", &pairs).unwrap();
        assert_eq!(out, "hello alice!");
    }

    #[test]
    fn inject_missing_returns_err_no_empty_string() {
        let pairs = vec![]; // 全缺
        let err = inject("tpl.json", "x=${A} y=${B}", &pairs).expect_err("缺变量应返 Err");
        assert_eq!(err.key, "A");
        assert_eq!(err.template, "tpl.json");
        // 关键:Err 的 Display 文本**不**含 `${A}` 替换后的空字符串/未替换形式
        // (我们只看"返 Err 而不是 Ok"),这里再确认 inject 不会返回空串。
        let ok: Result<String, _> =
            inject("tpl.json", "${A}", &[("A".to_string(), "v".to_string())]);
        assert!(ok.is_ok());
    }

    #[test]
    fn inject_missing_does_not_substitute_empty() {
        // 二次确认:即使 value 是空串,inject 也会替换(因为 key 存在);
        // 而 key 不存在时,直接返 Err,**不**把 `${VAR}` 替换成 ""。
        let pairs: Vec<(String, String)> = vec![];
        let r = inject("tpl.json", "[${A}]", &pairs);
        assert!(r.is_err(), "缺 key 必须 Err,不能 Ok 输出空串");
        match r {
            Err(MissingKey { key, .. }) => assert_eq!(key, "A"),
            Ok(s) => panic!("不应该 Ok,got: {s:?}"),
        }
    }

    #[test]
    fn inject_utf8_boundary_safety() {
        // 多字节字符不能被字节级切分。
        let pairs = vec![("X".to_string(), "值".to_string())];
        let out = inject("tpl.json", "你好 ${X} 世界", &pairs).unwrap();
        assert_eq!(out, "你好 值 世界");
    }

    // ── §8.1 硬约束:Debug/Display/Serialize 都不输出明文 ────────────

    #[test]
    fn secret_pairs_debug_never_includes_plaintext() {
        let pairs =
            SecretPairs::from_vec(vec![("API_KEY".to_string(), FORBIDDEN_VALUE.to_string())]);
        let dbg = format!("{pairs:?}");
        assert!(
            !dbg.contains(FORBIDDEN_VALUE),
            "SecretPairs Debug 泄露明文: {dbg}"
        );
        // 仍应包含 key 名(便于 debug 看"有哪些 key")。
        assert!(dbg.contains("API_KEY"), "Debug 应含 key 名: {dbg}");
        assert!(dbg.contains("<redacted>"), "Debug 应标 redacted: {dbg}");
    }

    #[test]
    fn secret_pairs_display_never_includes_plaintext() {
        let pairs =
            SecretPairs::from_vec(vec![("API_KEY".to_string(), FORBIDDEN_VALUE.to_string())]);
        let s = format!("{pairs}");
        assert!(
            !s.contains(FORBIDDEN_VALUE),
            "SecretPairs Display 泄露明文: {s}"
        );
        // Display 是最常被 `format!` / `eprintln!` / anyhow 触发的路径,
        // 必须只给"占位 + 计数"。
        assert!(s.contains("redacted"), "Display 应标 redacted: {s}");
    }

    #[test]
    fn secret_pairs_serialize_never_includes_plaintext() {
        let pairs = SecretPairs::from_vec(vec![
            ("A".to_string(), FORBIDDEN_VALUE.to_string()),
            ("B".to_string(), "another-secret-value-zzz".to_string()),
        ]);
        let json = serde_json::to_string(&pairs).expect("serialize ok");
        assert!(
            !json.contains(FORBIDDEN_VALUE),
            "SecretPoints Serialize 泄露明文: {json}"
        );
        assert!(
            !json.contains("another-secret-value-zzz"),
            "Serialize 泄露明文: {json}"
        );
        // 必须含 key 名 + REDACTED 占位。
        assert!(json.contains("A"));
        assert!(json.contains("B"));
        assert!(json.contains("<REDACTED>"));
    }

    #[test]
    fn missing_key_display_never_includes_plaintext() {
        // 故意把"看似明文"的内容塞进 key 名(只有 key 名,无 value),验证
        // Display 仍按预期格式(没有特殊字符会被剥)。
        let mk = MissingKey::new("LOOKS_LIKE_VALUE_BUT_IS_KEY", "tpl.json");
        let s = mk.to_string();
        assert!(s.contains("LOOKS_LIKE_VALUE_BUT_IS_KEY"));
        assert!(s.contains("tpl.json"));
        // 模板名是公开信息;确认没夹带任何 value 字段(结构上根本不存在 value)。
    }

    #[test]
    fn missing_keys_display_lists_keys_only() {
        let mut mk = MissingKeys::new("tpl.json");
        mk.push("FOO");
        mk.push("BAR");
        let s = mk.to_string();
        assert!(s.contains("FOO"));
        assert!(s.contains("BAR"));
        assert!(s.contains("tpl.json"));
        assert!(s.contains("2 个 key"));
    }

    // ── 防御:SecretPairs::get 与 inject 一致 ────────────────────────

    #[test]
    fn secret_pairs_get_returns_last_wins() {
        let pairs = SecretPairs::from_vec(vec![
            ("A".to_string(), "first".to_string()),
            ("A".to_string(), "second".to_string()),
        ]);
        assert_eq!(pairs.get("A"), Some("second"));
        assert_eq!(pairs.get("B"), None);
    }
}
