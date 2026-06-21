//! 测试用环境变量守卫：恢复前值；`HOME` 加锁避免并行单测互相污染。

use std::sync::Mutex;

static HOME_TEST_LOCK: Mutex<()> = Mutex::new(());

pub struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
    _home_lock: Option<std::sync::MutexGuard<'static, ()>>,
}

impl EnvGuard {
    pub fn set(key: &'static str, value: &str) -> Self {
        let home_lock = if key == "HOME" {
            Some(HOME_TEST_LOCK.lock().expect("HOME test lock"))
        } else {
            None
        };
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self {
            key,
            prev,
            _home_lock: home_lock,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}
