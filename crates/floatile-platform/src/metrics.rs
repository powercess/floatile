//! 宿主进程性能指标采样。
//!
//! 平台 I/O 收敛在本模块；调用方必须在后台线程采样，不能阻塞 UI 事件循环。

use std::time::Duration;

/// 单次宿主进程指标快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessMetrics {
    /// 进程累计获得的 CPU 时间。
    pub cpu_time: Duration,
    /// 当前驻留集大小。
    pub rss_bytes: u64,
}

/// 进程指标采样失败。
#[derive(Debug, thiserror::Error)]
pub enum MetricsError {
    #[error("process metrics are unsupported on this platform")]
    Unsupported,
    #[error("failed to read {source_name}: {source}")]
    Read {
        source_name: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid process metrics in {source_name}")]
    Invalid { source_name: &'static str },
}

/// 读取当前宿主进程的 CPU 累计时间与 RSS。
///
/// Linux 使用 `/proc/self/schedstat` 的纳秒 CPU 时间和 `/proc/self/status` 的 `VmRSS`；
/// 其他平台在实现对应原生 API 前返回 [`MetricsError::Unsupported`]。
pub fn process_metrics() -> Result<ProcessMetrics, MetricsError> {
    #[cfg(target_os = "linux")]
    {
        linux_process_metrics()
    }

    #[cfg(not(target_os = "linux"))]
    {
        Err(MetricsError::Unsupported)
    }
}

#[cfg(target_os = "linux")]
fn linux_process_metrics() -> Result<ProcessMetrics, MetricsError> {
    const SCHEDSTAT: &str = "/proc/self/schedstat";
    const STATUS: &str = "/proc/self/status";

    let schedstat = std::fs::read_to_string(SCHEDSTAT).map_err(|source| MetricsError::Read {
        source_name: SCHEDSTAT,
        source,
    })?;
    let status = std::fs::read_to_string(STATUS).map_err(|source| MetricsError::Read {
        source_name: STATUS,
        source,
    })?;

    let cpu_nanoseconds = parse_cpu_nanoseconds(&schedstat).ok_or(MetricsError::Invalid {
        source_name: SCHEDSTAT,
    })?;
    let rss_bytes = parse_rss_bytes(&status).ok_or(MetricsError::Invalid {
        source_name: STATUS,
    })?;

    Ok(ProcessMetrics {
        cpu_time: Duration::from_nanos(cpu_nanoseconds),
        rss_bytes,
    })
}

#[cfg(target_os = "linux")]
fn parse_cpu_nanoseconds(schedstat: &str) -> Option<u64> {
    schedstat.split_whitespace().next()?.parse().ok()
}

#[cfg(target_os = "linux")]
fn parse_rss_bytes(status: &str) -> Option<u64> {
    let rss_kib = status.lines().find_map(|line| {
        let value = line.strip_prefix("VmRSS:")?;
        value.split_whitespace().next()?.parse::<u64>().ok()
    })?;
    rss_kib.checked_mul(1024)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn parses_schedstat_cpu_nanoseconds() {
        assert_eq!(parse_cpu_nanoseconds("123456 789 10\n"), Some(123456));
        assert_eq!(parse_cpu_nanoseconds("invalid 789 10\n"), None);
    }

    #[test]
    fn parses_status_rss_kib_as_bytes() {
        let status = "Name:\tfloatile-shell\nVmRSS:\t  219240 kB\nThreads:\t2\n";
        assert_eq!(parse_rss_bytes(status), Some(219240 * 1024));
        assert_eq!(parse_rss_bytes("Name:\tfloatile-shell\n"), None);
    }
}
