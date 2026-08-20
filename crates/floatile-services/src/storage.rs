//! 插件私有 KV 存储（声明能力 storage:read/write）。
//!
//! P0 切片为进程内 per-instance 存储，按实例配额记账；键 scope 由
//! `PermissionBroker` 在每次调用时裁决（本服务不做 scope 判断，避免可绕过的
//! 双入口）。SQLite 持久化在 store 切片接入。

use std::collections::BTreeMap;

use crate::errors::StorageError;

pub struct StorageService {
    data: BTreeMap<String, String>,
    total_bytes: usize,
    max_bytes: usize,
}

impl StorageService {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            data: BTreeMap::new(),
            total_bytes: 0,
            max_bytes,
        }
    }

    fn valid_key(key: &str) -> Result<(), StorageError> {
        if key.is_empty() || key.contains('\0') || key.chars().count() > 256 {
            return Err(StorageError::InvalidKey);
        }
        Ok(())
    }

    pub fn get(&self, key: &str) -> Result<Option<String>, StorageError> {
        Self::valid_key(key)?;
        Ok(self.data.get(key).cloned())
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<(), StorageError> {
        Self::valid_key(key)?;
        let old_len = self.data.get(key).map_or(0, String::len);
        let new_total = self.total_bytes - old_len + value.len();
        if new_total > self.max_bytes {
            return Err(StorageError::QuotaExceeded);
        }
        self.total_bytes = new_total;
        self.data.insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    pub fn delete(&mut self, key: &str) -> Result<(), StorageError> {
        Self::valid_key(key)?;
        if let Some(value) = self.data.remove(key) {
            self.total_bytes -= value.len();
        }
        Ok(())
    }
}
