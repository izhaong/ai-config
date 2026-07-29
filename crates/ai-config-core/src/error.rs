//! 产品错误类型(`thiserror`)。所有错误带 `hint` 字段(PRD §6.3),
//! CLI 翻译成人话 + JSON 的 `hint`。
//!
//! 反模式:不在 `Display` 中包含 secrets 明文(PRD §8.1)。
//! Display 文本**只**写"缺哪个 key / 在哪个 template / 哪个路径",
//! **绝不**写 secret 的 value。
//!
//! 退出码(PRD §9.1):0 成功 / 2 参数错 / 3 部分失败 / 4 缺密钥 / 5 文件系统错。

#![allow(dead_code)]

use thiserror::Error;

/// CLI 退出码(PRD §9.1)。
///
/// | code | 含义 |
/// |------|------|
/// | `0`  | 成功 |
/// | `2`  | 参数 / 配置错(`InvalidPath` / `NotImplemented`) |
/// | `3`  | 部分失败(`LinkFailed` / `TemplateRender` / `UnsupportedAsset` 等可重试场景) |
/// | `4`  | 缺密钥(`SecretsMissing`) |
/// | `5`  | 文件系统 / IO 错(`Io` / `PermissionDenied` / `ConfigNotFound` / `AssetNotFound`) |
pub mod exit_code {
    pub const SUCCESS: u8 = 0;
    pub const ARG_ERROR: u8 = 2;
    pub const PARTIAL_FAILURE: u8 = 3;
    pub const SECRETS_MISSING: u8 = 4;
    pub const FS_ERROR: u8 = 5;
}

#[derive(Debug, Error)]
pub enum CoreError {
    // ── Phase 0 占位 ───────────────────────────────────────────────
    #[error("未实装(Phase 0 占位): {0}")]
    NotImplemented(&'static str),

    // ── 密钥 ────────────────────────────────────────────────────────
    //
    // Display 文本**绝不**包含 secret value;只写"缺哪个 key"和"在哪个 template"。
    // PRD §8.1 / §10 A-12 / A-14。`hint` 给修复建议,**也不**写值。
    #[error("secrets 缺 {key} (在 template {template})")]
    SecretsMissing {
        key: String,
        template: String,
        hint: String,
    },

    // ── 链接 ────────────────────────────────────────────────────────
    #[error("链接失败: {dest} ← {src} ({reason})")]
    LinkFailed {
        src: String,
        dest: String,
        reason: String,
        hint: String,
    },

    // ── 平台 / 资产 ────────────────────────────────────────────────
    #[error("平台 {platform:?} 不支持资产 {asset:?}")]
    UnsupportedAsset {
        platform: crate::model::PlatformId,
        asset: crate::model::AssetKind,
        hint: String,
    },

    // ── 模板渲染(占位,Phase 1 实装后字段会变) ─────────────────────
    #[error("模板渲染失败: {template} ({reason})")]
    TemplateRender {
        template: String,
        reason: String,
        hint: String,
    },

    // ── 资产 / 项目找不到 ──────────────────────────────────────────
    #[error("资产 {kind:?} 名称 {name} 找不到")]
    AssetNotFound {
        kind: crate::model::AssetKind,
        name: String,
        hint: String,
    },

    #[error("配置文件 {path} 找不到")]
    ConfigNotFound { path: String, hint: String },

    // ── 路径 / 权限 / IO ───────────────────────────────────────────
    #[error("路径非法: {0}")]
    InvalidPath(String),

    #[error("文件系统权限不足: {path} (需要 {expected_perms})")]
    PermissionDenied {
        path: String,
        expected_perms: String,
        hint: String,
    },

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON 解析错误: {0}")]
    Json(#[from] serde_json::Error),

    #[error("投影账本错误: {0}")]
    ProjectionLedger(String),
}

impl CoreError {
    /// 给 agent / 人类看的修复建议(PRD §6.3)。**所有**变体都返回 `Some` —
    /// Display 已经写了"出了什么事",hint 负责"接下来怎么办"。
    pub fn hint(&self) -> Option<&str> {
        match self {
            Self::NotImplemented(_) => None,
            Self::SecretsMissing { hint, .. }
            | Self::LinkFailed { hint, .. }
            | Self::UnsupportedAsset { hint, .. }
            | Self::TemplateRender { hint, .. }
            | Self::AssetNotFound { hint, .. }
            | Self::ConfigNotFound { hint, .. }
            | Self::PermissionDenied { hint, .. } => Some(hint.as_str()),
            Self::InvalidPath(_) | Self::Io(_) | Self::Json(_) | Self::ProjectionLedger(_) => None,
        }
    }

    /// 映射到 CLI 退出码(PRD §9.1)。
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::NotImplemented(_) | Self::InvalidPath(_) => exit_code::ARG_ERROR,
            Self::SecretsMissing { .. } => exit_code::SECRETS_MISSING,
            Self::LinkFailed { .. }
            | Self::UnsupportedAsset { .. }
            | Self::TemplateRender { .. }
            | Self::AssetNotFound { .. } => exit_code::PARTIAL_FAILURE,
            Self::Io(_)
            | Self::Json(_)
            | Self::ProjectionLedger(_)
            | Self::PermissionDenied { .. }
            | Self::ConfigNotFound { .. } => exit_code::FS_ERROR,
        }
    }
}

// ── anyhow 友好转换 ──────────────────────────────────────────────
//
// anyhow 的 blanket `From<E: std::error::Error + Send + Sync + 'static>`
// 已经覆盖 `CoreError -> anyhow::Error`,`?` 一行就够。这里**不**写显式 impl
// (会与 blanket 冲突);保留这段注释作 code-review 锚点,提醒"CoreError 一定
// 能走进 anyhow::Error"。

// ── CliError:CLI 友好的扁平结构,机器可解析输出用 ───────────────────
//
// PRD §9.1:机器可解析输出**不**夹杂人类文本,带 `hint` 字段;
// agent 据此决策下一步(场景 G)。

/// CLI / 机器可解析错误(PRD §9.1)。
///
/// 与 `CoreError` 的差别:
/// - `CoreError` 是带类型的内部错误(便于上层 match)
/// - `CliError` 是扁平字符串,直接序列化给 agent / JSON stdout
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    /// 数字退出码(同 [`CoreError::exit_code`],0 表示成功,这里不出现 0)
    pub code: u8,
    /// 人类 + 机器都能读的错误摘要(**不**包含 secrets 明文,见 §8.1)
    pub message: String,
    /// 修复建议(PRD §6.3)
    pub hint: Option<String>,
}

impl CliError {
    pub fn from_core(err: &CoreError) -> Self {
        Self {
            code: err.exit_code(),
            message: err.to_string(),
            hint: err.hint().map(str::to_owned),
        }
    }

    /// 仅用于测试 / 手工构造。
    pub fn new(code: u8, message: impl Into<String>, hint: Option<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint,
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[exit {}] {}", self.code, self.message)?;
        if let Some(hint) = &self.hint {
            write!(f, "\nhint: {hint}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CliError {}

impl From<CoreError> for CliError {
    fn from(err: CoreError) -> Self {
        Self::from_core(&err)
    }
}

impl From<&CoreError> for CliError {
    fn from(err: &CoreError) -> Self {
        Self::from_core(err)
    }
}

// ── 单元测试 ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AssetKind, PlatformId};

    /// PRD §8.1 硬约束:`Display` 文本**绝不**包含 secrets 明文值。
    /// 任意 secret value 字符串作为 hint 的一部分传入,Display 文本
    /// **绝不**含该字符串。
    #[test]
    fn secrets_missing_display_never_includes_secret_values() {
        // 假设这些字符串就是"用户塞进 secrets.env 的真值"。
        let fake_secret_value = "sk-prod-AAA-BBB-CCC-DDD-EEE-FFF";
        let another_secret = "p@ssw0rd!MINIO_ROOT_PASSWORD=supersecret-xyz";
        let env_path_leak = "/Users/alice/.config/ai-config/secrets.env";
        // 故意把这些值塞进 hint —— Display 文本必须不暴露。
        let hint_with_values =
            format!("在 {env_path_leak} 给 {fake_secret_value} 配 {another_secret}");

        let err = CoreError::SecretsMissing {
            key: "MINIO_ENDPOINT".to_string(),
            template: "mcp/servers/minio.json".to_string(),
            hint: hint_with_values.clone(),
        };

        let displayed = err.to_string();

        // Display 只该含 key 名 + template 名,不含任何"值"。
        assert!(
            displayed.contains("MINIO_ENDPOINT"),
            "Display 必须含 key 名(用于人/agent 识别): got {displayed}"
        );
        assert!(
            displayed.contains("mcp/servers/minio.json"),
            "Display 必须含 template 名(用于人/agent 定位): got {displayed}"
        );

        // 硬约束:Display 文本不含**任一**模拟的 secret value。
        assert!(
            !displayed.contains(fake_secret_value),
            "Display 泄露了 secret 明文: {displayed}"
        );
        assert!(
            !displayed.contains(another_secret),
            "Display 泄露了 secret 明文: {displayed}"
        );
        // 也不该出现 secrets.env 的绝对路径(防止路径即密钥)。
        assert!(
            !displayed.contains(env_path_leak),
            "Display 暴露了 secrets 文件绝对路径: {displayed}"
        );

        // 防御性:把所有"看起来像真值"的子串都验一遍。
        for forbidden in [
            "sk-prod-AAA-BBB-CCC-DDD-EEE-FFF",
            "supersecret-xyz",
            "p@ssw0rd",
        ] {
            assert!(
                !displayed.contains(forbidden),
                "Display 命中禁用子串 {forbidden:?}: {displayed}"
            );
        }
    }

    /// PRD §6.3 + 任务硬约束:`LinkFailed` 的 `hint` 字段必须非空。
    #[test]
    fn link_failed_hint_is_non_empty() {
        let err = CoreError::LinkFailed {
            src: "/Users/alice/.ai-config/skills/foo".to_string(),
            dest: "/Users/alice/.cursor/skills/foo".to_string(),
            reason: "目标路径已存在非链接文件".to_string(),
            hint: "把目标文件挪走或 `ai-config retract skill foo` 收回再重试".to_string(),
        };

        let hint = err.hint().expect("LinkFailed 必须带 hint");
        assert!(!hint.trim().is_empty(), "LinkFailed hint 不可为空字符串");
        // 退出码必须是"部分失败"类。
        assert_eq!(err.exit_code(), exit_code::PARTIAL_FAILURE);

        // 走 CliError 也不能丢 hint。
        let cli = CliError::from_core(&err);
        assert_eq!(cli.code, exit_code::PARTIAL_FAILURE);
        assert_eq!(cli.hint.as_deref(), Some(hint));
    }

    #[test]
    fn secrets_missing_exit_code_is_4() {
        let err = CoreError::SecretsMissing {
            key: "X".into(),
            template: "t.json".into(),
            hint: "配 X".into(),
        };
        assert_eq!(err.exit_code(), exit_code::SECRETS_MISSING);
        assert_eq!(err.exit_code(), 4);
    }

    #[test]
    fn fs_errors_map_to_exit_code_5() {
        let err = CoreError::ConfigNotFound {
            path: "/nope.toml".into(),
            hint: "跑 `ai-config init`".into(),
        };
        assert_eq!(err.exit_code(), 5);

        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let core = CoreError::Io(io_err);
        assert_eq!(core.exit_code(), 5);
    }

    #[test]
    fn cli_error_carries_code_message_and_hint() {
        let err = CoreError::UnsupportedAsset {
            platform: PlatformId::Codex,
            asset: AssetKind::Rule,
            hint: "Codex 通过 AGENTS.md 间接消费 rules,无独立 rules 目录".into(),
        };
        let cli = CliError::from_core(&err);
        assert_eq!(cli.code, exit_code::PARTIAL_FAILURE);
        // PlatformId / AssetKind 的 Display 走 Debug 形式(`Codex` / `Rule`)。
        assert!(cli.message.contains("Codex"), "got: {}", cli.message);
        assert!(cli.message.contains("Rule"), "got: {}", cli.message);
        assert!(cli.hint.is_some());
    }

    #[test]
    fn cli_error_display_includes_exit_code_and_hint() {
        let cli = CliError::new(4, "secrets 缺 X (在 t.json)", Some("配 X".into()));
        let s = cli.to_string();
        assert!(s.contains("exit 4"));
        assert!(s.contains("secrets 缺 X"));
        assert!(s.contains("hint: 配 X"));
    }

    #[test]
    fn core_error_converts_to_anyhow() {
        let err = CoreError::LinkFailed {
            src: "a".into(),
            dest: "b".into(),
            reason: "r".into(),
            hint: "h".into(),
        };
        let anyhow_err: anyhow::Error = err.into();
        // anyhow::Error 的 Display 走原 CoreError 的 Display。
        let s = anyhow_err.to_string();
        assert!(s.contains("链接失败"));
    }

    #[test]
    fn not_implemented_has_no_hint() {
        let err = CoreError::NotImplemented("sync engine");
        assert_eq!(err.hint(), None);
    }
}
