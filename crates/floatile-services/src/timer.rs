//! 一次性计时器服务（声明能力 timer:schedule）。
//!
//! v1 计时器为一次性：到期投递一次 `Timer(id)` 到插件实例队列，周期由插件在
//! 每次 tick 后重新 schedule。配额：每分钟上限与活跃计时器上限。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::errors::TimerError;

/// 计时器到期投递 sink：把 timer-id 送到实例事件队列。
pub type TimerSink = Arc<dyn Fn(u32) + Send + Sync>;

/// 默认每分钟 schedule 上限（permission-model §1.2）。
pub const DEFAULT_MAX_PER_MINUTE: u32 = 60;
/// 默认活跃计时器上限。
pub const DEFAULT_MAX_ACTIVE: u32 = 8;

struct ActiveTimer {
    cancelled: Arc<AtomicBool>,
}

pub struct TimerService {
    next_id: u32,
    active: HashMap<u32, ActiveTimer>,
    sink: TimerSink,
    max_per_minute: u32,
    max_active: u32,
    window_start: Instant,
    window_count: u32,
}

impl TimerService {
    pub fn new(sink: TimerSink) -> Self {
        Self {
            next_id: 1,
            active: HashMap::new(),
            sink,
            max_per_minute: DEFAULT_MAX_PER_MINUTE,
            max_active: DEFAULT_MAX_ACTIVE,
            window_start: Instant::now(),
            window_count: 0,
        }
    }

    /// 设置配额（来自实例 grant；不得高于插件授权上限）。
    pub fn set_quota(&mut self, max_per_minute: u32, max_active: u32) {
        self.max_per_minute = max_per_minute;
        self.max_active = max_active;
    }

    /// 请求在 `delay_ms` 后投递一次 `Timer(id)`。
    pub fn schedule(&mut self, delay_ms: u64) -> Result<u32, TimerError> {
        if delay_ms == 0 {
            return Err(TimerError::InvalidDelay);
        }
        let now = Instant::now();
        if now.duration_since(self.window_start) >= Duration::from_secs(60) {
            self.window_start = now;
            self.window_count = 0;
        }
        if self.window_count >= self.max_per_minute {
            return Err(TimerError::BudgetExceeded);
        }
        if self.active.len() >= self.max_active as usize {
            return Err(TimerError::BudgetExceeded);
        }
        self.window_count += 1;

        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);

        let cancelled = Arc::new(AtomicBool::new(false));
        self.active.insert(
            id,
            ActiveTimer {
                cancelled: cancelled.clone(),
            },
        );

        let sink = self.sink.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            if !cancelled.load(Ordering::SeqCst) {
                (sink)(id);
            }
        });
        Ok(id)
    }

    /// 取消一个未到期的计时器。
    pub fn cancel(&mut self, id: u32) -> Result<(), TimerError> {
        match self.active.remove(&id) {
            Some(timer) => {
                timer.cancelled.store(true, Ordering::SeqCst);
                Ok(())
            }
            None => Err(TimerError::InvalidTimerId),
        }
    }

    /// 计时器事件已投递并由实例处理完毕，释放槽位。
    pub fn complete(&mut self, id: u32) {
        self.active.remove(&id);
    }
}
