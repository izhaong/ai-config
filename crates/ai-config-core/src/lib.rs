//! ai-config 业务逻辑核心。
//!
//! 模块边界(对齐 ARCHITECTURE §3 与 plan §2.2.1):
//! - `model`     : 内存数据类型(Skill / Rule / McpServer / Agent / SymlinkTarget / SyncStatus / Project / SyncAction)
//! - `source`    : 资产源解析(skills/ rules/ mcp/servers/ agents/ 目录扫描)
//! - `platform`  : 4 平台适配器(Cursor / Codex / Claude / Hermes)
//! - `link`      : 链接(symlink / junction / hardlink 三态 + 幂等)
//! - `template`  : MCP 模板渲染(secrets 注入 + 原子 rename)
//! - `secrets`   : secrets.env 读写(0600,不进 store)
//! - `sync`      : 同步引擎(source + override + enabled platforms → SyncAction)
//! - `error`     : thiserror 产品错误
//!
//! 硬约束:本 crate 不依赖 Tauri / GUI / notify / UDS 任何东西;不读 secrets 明文到 log。
//!
//! 完整架构说明(包含 ASCII 图)见 [`docs/product/ARCHITECTURE.md`](../../docs/product/ARCHITECTURE.md)。

pub mod error;
pub mod link;
pub mod mcp_json;
pub mod model;
pub mod paths;
pub mod platform;
pub mod secrets;
pub mod source;
pub mod sync;
pub mod template;
