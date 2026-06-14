//! ai-config 守护进程占位(Phase 2 实装)。

#![allow(dead_code)]

use ai_config_bus::Bus;
use ai_config_store::Store;
use ai_config_watcher::Watcher;

pub struct Daemon {
    pub watcher: Watcher,
    pub bus: Bus,
    pub store: Store,
}

impl Daemon {
    pub fn new() -> Self {
        // Phase 2 实装:Store::open() 真正持久化;W9 起 store 已可独立打开
        // (daemon 本进程持有)。此处用 open() 的 stub 版本(暂时不接 watcher)
        // 保持与原 `Daemon::new` 签名兼容。
        let store = Store::open().unwrap_or_else(|e| {
            eprintln!("daemon 启动时打开 store 失败: {e};回退到 in-memory stub");
            Store::open_at(std::path::Path::new(":memory:")).expect("in-memory store 永远可开")
        });
        Self {
            watcher: Watcher::placeholder(),
            bus: Bus::placeholder(),
            store,
        }
    }
}

impl Default for Daemon {
    fn default() -> Self {
        Self::new()
    }
}

fn main() {
    let _daemon = Daemon::new();
    eprintln!(
        "ai-configd v{} (Phase 0 占位;Phase 2 起实装 watcher + bus + IPC)",
        env!("CARGO_PKG_VERSION")
    );
    eprintln!("Phase 0 不进入事件循环;退出");
}
