//! SQLite DDL(对齐 ARCHITECTURE §3 `store` 模块边界)。
//!
//! ## 阶段(W9 落地)
//! - `projects`(本阶段):id / name / root_path / registered_at
//!
//! ## 阶段(W10 续实装)
//! - `items`:扫到的资产(Skill / Rule / Mcp / Agent)
//! - `targets`:per-item × per-platform 链接状态(Linked / Unlinked / Retracted / Missing / Failed)
//! - `events`:watcher 触发的事件流(供 UDS 订阅)
//! - `secrets_meta`:secrets.env 变量名 + 来源(只元数据,不存明文)
//!
//! 所有 DDL 都用 `IF NOT EXISTS`,`Store::migrate` 调用即可,重复执行幂等。

/// W9 阶段全部 DDL(按依赖顺序)。
///
/// 注意:`projects` 表被 `items` 等后续表用外键引用时,务必先建 projects;
/// 后续 W10 加新表时,append 到本数组**末尾**即可,不要改已有条目(保持历史
/// `Store::migrate` 调用幂等)。
pub const DDL: &[&str] = &[
    // ── W9 阶段 ─────────────────────────────────────────────────
    // 项目注册表(主源,唯一的事实源之一,PRD §3.1)
    //
    // 设计要点:
    // - `name` UNIQUE:同台机器不重名;冲突时 `add` 返回 `ProjectExists`
    // - `registered_at` 用 ISO-8601 字符串(`chrono::Utc::now().to_rfc3339()`),避免时区歧义
    // - 暂不引用 `items`:W10 引入外键
    r#"
    CREATE TABLE IF NOT EXISTS projects (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        name          TEXT NOT NULL UNIQUE,
        root_path     TEXT NOT NULL,
        registered_at TEXT NOT NULL
    )
    "#,
    // Git 同步与 GUI 偏好（KV）
    r#"
    CREATE TABLE IF NOT EXISTS settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    )
    "#,
    // 投影账本是生成物所有权的辅助证据；缺失时 core 会安全降级为 Foreign。
    r#"
    CREATE TABLE IF NOT EXISTS projection_ledger (
        scope_key          TEXT NOT NULL,
        kind               TEXT NOT NULL,
        name               TEXT NOT NULL,
        surface_json       TEXT NOT NULL,
        mode               TEXT NOT NULL,
        source_path        TEXT NOT NULL,
        target_path        TEXT NOT NULL,
        entry_key          TEXT,
        source_fingerprint TEXT NOT NULL,
        entry_fingerprint  TEXT,
        target_fingerprint TEXT NOT NULL,
        applied_at         TEXT NOT NULL,
        PRIMARY KEY (scope_key, kind, name, surface_json)
    )
    "#,
];
