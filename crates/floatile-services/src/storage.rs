//! 插件私有 KV 存储（声明能力 storage:read/write）。
//!
//! P0 切片为进程内 per-instance 存储，按实例配额记账；键 scope 由
//! `PermissionBroker` 在每次调用时裁决（本服务不做 scope 判断，避免可绕过的
//! 双入口）。SQLite 持久化在 store 切片接入。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::errors::StorageError;

#[derive(Clone)]
pub struct StorageService {
    state: Arc<Mutex<StorageState>>,
}

struct StorageState {
    data: BTreeMap<String, String>,
    total_bytes: usize,
    max_bytes: usize,
}

impl StorageService {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(StorageState {
                data: BTreeMap::new(),
                total_bytes: 0,
                max_bytes,
            })),
        }
    }

    pub(crate) fn valid_key(key: &str) -> Result<(), StorageError> {
        if key.is_empty() || key.contains('\0') || key.chars().count() > 256 {
            return Err(StorageError::InvalidKey);
        }
        Ok(())
    }

    pub fn get(&self, key: &str) -> Result<Option<String>, StorageError> {
        Self::valid_key(key)?;
        Ok(lock(&self.state).data.get(key).cloned())
    }

    pub fn set(&self, key: &str, value: &str) -> Result<(), StorageError> {
        Self::valid_key(key)?;
        let mut state = lock(&self.state);
        let old_len = state.data.get(key).map_or(0, String::len);
        let new_total = state.total_bytes - old_len + value.len();
        if new_total > state.max_bytes {
            return Err(StorageError::QuotaExceeded);
        }
        state.total_bytes = new_total;
        state.data.insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    pub fn delete(&self, key: &str) -> Result<(), StorageError> {
        Self::valid_key(key)?;
        let mut state = lock(&self.state);
        if let Some(value) = state.data.remove(key) {
            state.total_bytes -= value.len();
        }
        Ok(())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
