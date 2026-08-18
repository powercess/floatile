//! 宿主进程指标服务（声明能力 system:cpu / system:memory）。
//!
//! 只暴露本进程占用，不暴露整机信息；按采样频率上限限速。

use std::time::{Duration, Instant};

use floatile_platform::process_metrics;

use crate::errors::MetricsError;

/// 内存快照（KiB）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemorySnapshot {
    pub rss_kib: u64,
    pub virtual_kib: u64,
}

pub struct MetricsService {
    /// 上次采样 (wall, cpu_time)，用于计算 CPU%。
    last: Option<(Instant, Duration)>,
    last_sample: Option<Instant>,
    min_interval: Duration,
}

impl MetricsService {
    pub fn new(sample_rate_hz: u32) -> Self {
        let interval = if sample_rate_hz == 0 {
            Duration::ZERO
        } else {
            Duration::from_secs(1) / sample_rate_hz
        };
        Self {
            last: None,
            last_sample: None,
            min_interval: interval,
        }
    }

    fn rate_limited(&mut self) -> Result<(), MetricsError> {
        let now = Instant::now();
        if let Some(prev) = self.last_sample
            && now.duration_since(prev) < self.min_interval
        {
            return Err(MetricsError::RateExceeded);
        }
        self.last_sample = Some(now);
        Ok(())
    }

    /// 本进程 CPU 占用（0.0..100.0，相对两次采样间隔）。
    pub fn cpu_percent(&mut self) -> Result<f64, MetricsError> {
        self.rate_limited()?;
        let sample = process_metrics().map_err(|_| MetricsError::Unavailable)?;
        let now = Instant::now();
        let Some((prev_now, prev_cpu)) = self.last else {
            self.last = Some((now, sample.cpu_time));
            return Ok(0.0);
        };
        let wall = now.duration_since(prev_now).as_secs_f64();
        self.last = Some((now, sample.cpu_time));
        if wall <= 0.0 {
            return Ok(0.0);
        }
        let cpu = sample.cpu_time.as_secs_f64() - prev_cpu.as_secs_f64();
        Ok((cpu / wall * 100.0).clamp(0.0, 100.0 * 100.0))
    }

    /// 本进程内存快照。虚拟内存采样未实现，`virtual_kib` 暂为 0。
    pub fn memory(&self) -> Result<MemorySnapshot, MetricsError> {
        let sample = process_metrics().map_err(|_| MetricsError::Unavailable)?;
        Ok(MemorySnapshot {
            rss_kib: sample.rss_bytes / 1024,
            virtual_kib: 0,
        })
    }
}
