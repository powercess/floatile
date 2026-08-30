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
    /// 当前进程虚拟地址/提交占用；平台无法提供时为 0。
    pub virtual_bytes: u64,
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
/// Windows 使用 `GetProcessTimes` 与 `K32GetProcessMemoryInfo`；Linux 使用
/// `/proc/self/schedstat` 与 `/proc/self/status`；macOS 使用
/// `task_info(MACH_TASK_BASIC_INFO)` 与 `getrusage`。
pub fn process_metrics() -> Result<ProcessMetrics, MetricsError> {
    #[cfg(windows)]
    {
        windows_process_metrics()
    }

    #[cfg(target_os = "linux")]
    {
        linux_process_metrics()
    }

    #[cfg(target_os = "macos")]
    {
        macos_process_metrics()
    }

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Err(MetricsError::Unsupported)
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn windows_process_metrics() -> Result<ProcessMetrics, MetricsError> {
    use std::mem::size_of;

    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    let process = unsafe { GetCurrentProcess() };
    let empty_filetime = || FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut creation = empty_filetime();
    let mut exit = empty_filetime();
    let mut kernel = empty_filetime();
    let mut user = empty_filetime();
    // SAFETY: `GetCurrentProcess` returns a valid pseudo-handle for this process; all FILETIME
    // pointers reference initialized writable values for the duration of the call.
    let times_ok =
        unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) };
    if times_ok == 0 {
        return Err(MetricsError::Invalid {
            source_name: "GetProcessTimes",
        });
    }

    let mut memory = PROCESS_MEMORY_COUNTERS {
        cb: u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS>()).unwrap_or(u32::MAX),
        PageFaultCount: 0,
        PeakWorkingSetSize: 0,
        WorkingSetSize: 0,
        QuotaPeakPagedPoolUsage: 0,
        QuotaPagedPoolUsage: 0,
        QuotaPeakNonPagedPoolUsage: 0,
        QuotaNonPagedPoolUsage: 0,
        PagefileUsage: 0,
        PeakPagefileUsage: 0,
    };
    // SAFETY: the pseudo-handle is valid and `memory` has the exact structure size supplied in
    // `cb`; the API writes only within that structure.
    let memory_ok = unsafe { K32GetProcessMemoryInfo(process, &mut memory, memory.cb) };
    if memory_ok == 0 {
        return Err(MetricsError::Invalid {
            source_name: "K32GetProcessMemoryInfo",
        });
    }

    let filetime_ticks =
        |value: FILETIME| (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime);
    let cpu_ticks = filetime_ticks(kernel).saturating_add(filetime_ticks(user));
    Ok(ProcessMetrics {
        cpu_time: Duration::from_nanos(cpu_ticks.saturating_mul(100)),
        rss_bytes: u64::try_from(memory.WorkingSetSize).unwrap_or(u64::MAX),
        virtual_bytes: u64::try_from(memory.PagefileUsage).unwrap_or(u64::MAX),
    })
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn macos_process_metrics() -> Result<ProcessMetrics, MetricsError> {
    use mach2::task::task_info;
    use mach2::task_info::{MACH_TASK_BASIC_INFO_COUNT, mach_task_basic_info};
    use mach2::traps::mach_task_self;

    // `resident_size` 以字节计；`user_time`/`system_time` 为累计 CPU 时间。
    let mut info = mach_task_basic_info::default();
    let mut count = MACH_TASK_BASIC_INFO_COUNT;
    // SAFETY: info 尺寸由 MACH_TASK_BASIC_INFO_COUNT 表达；task 为当前进程的 task。
    let kern = unsafe {
        task_info(
            mach_task_self(),
            mach2::task_info::MACH_TASK_BASIC_INFO,
            &mut info as *mut mach_task_basic_info as *mut _,
            &mut count,
        )
    };
    if kern != mach2::kern_return::KERN_SUCCESS {
        return Err(MetricsError::Invalid {
            source_name: "mach task_info",
        });
    }

    let user_usec = u64::try_from(info.user_time.seconds)
        .unwrap_or(u64::MAX)
        .saturating_mul(1_000_000)
        .saturating_add(info.user_time.microseconds.max(0) as u64);
    let system_usec = u64::try_from(info.system_time.seconds)
        .unwrap_or(u64::MAX)
        .saturating_mul(1_000_000)
        .saturating_add(info.system_time.microseconds.max(0) as u64);

    Ok(ProcessMetrics {
        cpu_time: Duration::from_micros(user_usec.saturating_add(system_usec)),
        rss_bytes: info.resident_size,
        virtual_bytes: info.virtual_size,
    })
}

#[cfg(target_os = "linux")]
fn linux_process_metrics() -> Result<ProcessMetrics, MetricsError> {
    const TASKS: &str = "/proc/self/task";
    const TASK_SCHEDSTAT: &str = "/proc/self/task/*/schedstat";
    const STATUS: &str = "/proc/self/status";

    let tasks = std::fs::read_dir(TASKS).map_err(|source| MetricsError::Read {
        source_name: TASKS,
        source,
    })?;
    let mut cpu_nanoseconds = 0u64;
    let mut sampled_threads = 0usize;
    for task in tasks {
        let task = task.map_err(|source| MetricsError::Read {
            source_name: TASKS,
            source,
        })?;
        let path = task.path().join("schedstat");
        let schedstat = match std::fs::read_to_string(path) {
            Ok(value) => value,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source) => {
                return Err(MetricsError::Read {
                    source_name: TASK_SCHEDSTAT,
                    source,
                });
            }
        };
        let thread_cpu = parse_cpu_nanoseconds(&schedstat).ok_or(MetricsError::Invalid {
            source_name: TASK_SCHEDSTAT,
        })?;
        cpu_nanoseconds = cpu_nanoseconds
            .checked_add(thread_cpu)
            .ok_or(MetricsError::Invalid {
                source_name: TASK_SCHEDSTAT,
            })?;
        sampled_threads += 1;
    }
    if sampled_threads == 0 {
        return Err(MetricsError::Invalid {
            source_name: TASK_SCHEDSTAT,
        });
    }
    let status = std::fs::read_to_string(STATUS).map_err(|source| MetricsError::Read {
        source_name: STATUS,
        source,
    })?;

    let rss_bytes = parse_rss_bytes(&status).ok_or(MetricsError::Invalid {
        source_name: STATUS,
    })?;

    Ok(ProcessMetrics {
        cpu_time: Duration::from_nanos(cpu_nanoseconds),
        rss_bytes,
        virtual_bytes: parse_virtual_bytes(&status).unwrap_or_default(),
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

#[cfg(target_os = "linux")]
fn parse_virtual_bytes(status: &str) -> Option<u64> {
    let virtual_kib = status.lines().find_map(|line| {
        let value = line.strip_prefix("VmSize:")?;
        value.split_whitespace().next()?.parse::<u64>().ok()
    })?;
    virtual_kib.checked_mul(1024)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Barrier};

    #[test]
    fn parses_schedstat_cpu_nanoseconds() {
        assert_eq!(parse_cpu_nanoseconds("123456 789 10\n"), Some(123456));
        assert_eq!(parse_cpu_nanoseconds("invalid 789 10\n"), None);
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn process_cpu_includes_worker_threads() {
        let barrier = Arc::new(Barrier::new(3));
        let stop = Arc::new(AtomicBool::new(false));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                let stop = Arc::clone(&stop);
                std::thread::spawn(move || {
                    barrier.wait();
                    while !stop.load(Ordering::Relaxed) {
                        std::hint::spin_loop();
                    }
                })
            })
            .collect();
        barrier.wait();
        let before = process_metrics().expect("process metrics before worker load");
        std::thread::sleep(Duration::from_millis(100));
        let after = process_metrics().expect("process metrics after worker load");
        stop.store(true, Ordering::Relaxed);
        for worker in workers {
            worker.join().expect("worker should stop");
        }
        assert!(
            after.cpu_time > before.cpu_time,
            "worker CPU must be included in process metrics"
        );
    }

    #[test]
    fn parses_status_rss_kib_as_bytes() {
        let status =
            "Name:\tfloatile-shell\nVmSize:\t  400000 kB\nVmRSS:\t  219240 kB\nThreads:\t2\n";
        assert_eq!(parse_rss_bytes(status), Some(219240 * 1024));
        assert_eq!(parse_virtual_bytes(status), Some(400000 * 1024));
        assert_eq!(parse_rss_bytes("Name:\tfloatile-shell\n"), None);
    }
}

#[cfg(all(test, windows))]
#[allow(clippy::expect_used)]
mod windows_tests {
    use super::*;

    #[test]
    fn samples_current_process_cpu_rss_and_virtual_memory() {
        let first = process_metrics().expect("Windows process metrics should be available");
        std::thread::sleep(Duration::from_millis(10));
        let second = process_metrics().expect("repeated Windows process metrics should work");
        assert!(second.cpu_time >= first.cpu_time);
        assert!(second.rss_bytes > 0);
        assert!(second.virtual_bytes > 0);
    }
}
