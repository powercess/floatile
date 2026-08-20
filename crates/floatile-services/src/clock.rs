//! 只读 wall clock（固有能力 clock:read）。

/// 当前时间快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockSnapshot {
    pub unix_millis: u64,
    /// UTC 偏移（分钟）；P0 未接入系统时区，固定为 0（UTC）。
    pub utc_offset_minutes: i32,
}

pub struct Clock;

impl Clock {
    pub fn now(&self) -> ClockSnapshot {
        let unix_millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0);
        ClockSnapshot {
            unix_millis,
            utc_offset_minutes: 0,
        }
    }
}
