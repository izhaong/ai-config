//! 测试用环境变量守卫：恢复前值；串行化进程级环境变量修改，避免并行单测互相污染。

use std::sync::Mutex;

static PROCESS_ENV_LOCK: Mutex<()> = Mutex::new(());

/// 为只读取进程环境（尤其是 HOME）的测试取得与 EnvGuard 同一把锁。
///
/// 读取方也必须参与互斥，否则并行测试可在 adapter 和 `dirs::home_dir()` 两次读取之间
/// 观察到不同的 HOME。
pub fn read_guard() -> std::sync::MutexGuard<'static, ()> {
    PROCESS_ENV_LOCK.lock().expect("process env test lock")
}

pub struct EnvGuard {
    values: Vec<EnvValue>,
    _process_env_lock: std::sync::MutexGuard<'static, ()>,
}

struct EnvValue {
    key: &'static str,
    prev: Option<String>,
}

impl EnvGuard {
    pub fn set(key: &'static str, value: &str) -> Self {
        Self::set_many(&[(key, Some(value))])
    }

    pub fn unset(key: &'static str) -> Self {
        Self::set_many(&[(key, None)])
    }

    pub fn set_many(values: &[(&'static str, Option<&str>)]) -> Self {
        let process_env_lock = read_guard();
        let values = values
            .iter()
            .map(|(key, value)| {
                let prev = std::env::var(key).ok();
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
                EnvValue { key, prev }
            })
            .collect();
        Self {
            values,
            _process_env_lock: process_env_lock,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for value in self.values.iter().rev() {
            match &value.prev {
                Some(prev) => std::env::set_var(value.key, prev),
                None => std::env::remove_var(value.key),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn read_guard_serializes_environment_mutation() {
        let read_guard = read_guard();
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _guard = EnvGuard::set("AGENT_MANAGER_TEST_ENV_LOCK", "set");
            tx.send(()).unwrap();
        });

        assert!(rx.recv_timeout(Duration::from_millis(30)).is_err());
        drop(read_guard);
        rx.recv_timeout(Duration::from_secs(1)).unwrap();
        worker.join().unwrap();
    }
}
