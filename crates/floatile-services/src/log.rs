//! 限速、截断的插件日志（固有能力 log:write）。

use std::time::{Duration, Instant};

use crate::errors::LogError;

/// 日志级别（与 WIT `host-log.log-level` 对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// 单条消息长度上限（字符）。
pub const MAX_MESSAGE_CHARS: usize = 4096;
/// 默认每分钟消息上限。
pub const DEFAULT_MAX_PER_MINUTE: u32 = 60;

pub struct LogService {
    plugin: String,
    instance: u64,
    max_per_minute: u32,
    window_start: Instant,
    window_count: u32,
}

impl LogService {
    pub fn new(plugin: impl Into<String>, instance: u64) -> Self {
        Self {
            plugin: plugin.into(),
            instance,
            max_per_minute: DEFAULT_MAX_PER_MINUTE,
            window_start: Instant::now(),
            window_count: 0,
        }
    }

    /// 记录一条插件日志；限速与长度上限在这里强制。
    pub fn log(&mut self, level: LogLevel, message: &str) -> Result<(), LogError> {
        let now = Instant::now();
        if now.duration_since(self.window_start) >= Duration::from_secs(60) {
            self.window_start = now;
            self.window_count = 0;
        }
        if self.window_count >= self.max_per_minute {
            return Err(LogError::RateExceeded);
        }
        self.window_count += 1;

        let message: String = message.chars().take(MAX_MESSAGE_CHARS).collect();
        match level {
            LogLevel::Debug => {
                tracing::event!(target: "floatile::plugin-log", tracing::Level::DEBUG, plugin_id = %self.plugin, instance_id = self.instance, message = %message)
            }
            LogLevel::Info => {
                tracing::event!(target: "floatile::plugin-log", tracing::Level::INFO, plugin_id = %self.plugin, instance_id = self.instance, message = %message)
            }
            LogLevel::Warn => {
                tracing::event!(target: "floatile::plugin-log", tracing::Level::WARN, plugin_id = %self.plugin, instance_id = self.instance, message = %message)
            }
            LogLevel::Error => {
                tracing::event!(target: "floatile::plugin-log", tracing::Level::ERROR, plugin_id = %self.plugin, instance_id = self.instance, message = %message)
            }
        }
        Ok(())
    }
}
