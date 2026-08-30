//! 桌面宿主单实例守卫。
//!
//! Windows 使用命名互斥体保证同一用户会话中只运行一个 Floatile 宿主。
//! 其他平台暂未接入原生实现，调用方必须显式处理 [`SingleInstanceState::Unsupported`]。

/// 单实例获取结果。
#[derive(Debug)]
pub enum SingleInstanceState {
    /// 当前进程持有宿主实例守卫；守卫释放前其他进程不得成为主实例。
    Acquired(SingleInstanceGuard),
    /// 当前用户会话中已有 Floatile 宿主实例。
    AlreadyRunning,
    /// 当前平台尚未实现单实例约束。
    Unsupported,
}

/// 持有平台单实例资源的 RAII 守卫。
#[derive(Debug)]
pub struct SingleInstanceGuard {
    #[cfg(windows)]
    mutex: windows_sys::Win32::Foundation::HANDLE,
    #[cfg(windows)]
    activation_event: windows_sys::Win32::Foundation::HANDLE,
}

/// 单实例资源获取失败。
#[derive(Debug, thiserror::Error)]
pub enum SingleInstanceError {
    #[error("单实例名称不能为空")]
    EmptyName,
    #[error("单实例名称不能包含 NUL 字符")]
    NameContainsNul,
    #[error("Windows 单实例资源操作失败: {code}")]
    Platform { code: u32 },
}

impl SingleInstanceGuard {
    /// 非阻塞消费一次“再次启动”激活请求。
    ///
    /// Windows 使用手动复位命名事件；其他平台的 Unsupported 路径不会产生 guard。
    pub fn take_activation_request(&self) -> Result<bool, SingleInstanceError> {
        #[cfg(windows)]
        {
            windows_impl::take_activation_request(self.activation_event)
        }

        #[cfg(not(windows))]
        {
            Ok(false)
        }
    }
}

/// 尝试取得当前桌面宿主的单实例所有权。
///
/// `name` 是宿主维护的稳定标识，不得包含 NUL。Windows 实现使用 `Local\\` 命名空间，
/// 因此约束只覆盖当前登录会话，不会阻止其他 Windows 用户运行自己的 Floatile。
pub fn acquire_single_instance(name: &str) -> Result<SingleInstanceState, SingleInstanceError> {
    validate_name(name)?;

    #[cfg(windows)]
    {
        windows_impl::acquire(name)
    }

    #[cfg(not(windows))]
    {
        Ok(SingleInstanceState::Unsupported)
    }
}

fn validate_name(name: &str) -> Result<(), SingleInstanceError> {
    if name.is_empty() {
        return Err(SingleInstanceError::EmptyName);
    }
    if name.contains('\0') {
        return Err(SingleInstanceError::NameContainsNul);
    }
    Ok(())
}

#[cfg(windows)]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        windows_impl::close(self.activation_event);
        windows_impl::close(self.mutex);
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_impl {
    use super::{SingleInstanceError, SingleInstanceGuard, SingleInstanceState};
    use windows_sys::Win32::Foundation::{
        ERROR_ALREADY_EXISTS, GetLastError, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::Threading::{
        CreateEventW, CreateMutexW, ResetEvent, SetEvent, WaitForSingleObject,
    };

    pub(super) fn close(handle: windows_sys::Win32::Foundation::HANDLE) {
        // SAFETY: handle 由成功的 CreateMutexW 返回，调用方保证只关闭一次。
        let _ = unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
    }

    pub(super) fn acquire(name: &str) -> Result<SingleInstanceState, SingleInstanceError> {
        let event_name = format!("{name}.Activation");
        let wide_event_name: Vec<u16> = event_name
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: 名称缓冲区 NUL 结尾且在调用期间有效；手动复位事件保存启动请求，
        // 直到主实例的 UI 定时器显式消费。
        let activation_event =
            unsafe { CreateEventW(std::ptr::null(), 1, 0, wide_event_name.as_ptr()) };
        if activation_event == 0 {
            // SAFETY: 读取紧邻失败 CreateEventW 的线程本地错误码。
            let code = unsafe { GetLastError() };
            return Err(SingleInstanceError::Platform { code });
        }
        let wide_name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: wide_name 是本函数持有的 NUL 结尾 UTF-16 缓冲区；不传安全描述符，
        // CreateMutexW 只在调用期间读取名称并返回由守卫独占管理的 HANDLE。
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide_name.as_ptr()) };
        if handle == 0 {
            // SAFETY: 读取紧邻失败 CreateMutexW 调用产生的线程本地错误码。
            let code = unsafe { GetLastError() };
            close(activation_event);
            return Err(SingleInstanceError::Platform { code });
        }

        // SAFETY: 读取紧邻成功 CreateMutexW 调用产生的线程本地状态码；
        // ERROR_ALREADY_EXISTS 表示本调用只取得了已有互斥体的额外句柄。
        let code = unsafe { GetLastError() };
        if code == ERROR_ALREADY_EXISTS {
            // SAFETY: activation_event 是当前会话中同名的有效事件句柄。
            let signaled = unsafe { SetEvent(activation_event) };
            // SAFETY: 必须在 CloseHandle 之前读取紧邻 SetEvent 的错误码。
            let signal_error = unsafe { GetLastError() };
            close(handle);
            close(activation_event);
            if signaled == 0 {
                return Err(SingleInstanceError::Platform { code: signal_error });
            }
            return Ok(SingleInstanceState::AlreadyRunning);
        }

        Ok(SingleInstanceState::Acquired(SingleInstanceGuard {
            mutex: handle,
            activation_event,
        }))
    }

    pub(super) fn take_activation_request(
        activation_event: windows_sys::Win32::Foundation::HANDLE,
    ) -> Result<bool, SingleInstanceError> {
        // SAFETY: activation_event 由 guard 持有并在该调用期间保持有效；超时 0 不阻塞。
        let status = unsafe { WaitForSingleObject(activation_event, 0) };
        match status {
            WAIT_TIMEOUT => Ok(false),
            WAIT_OBJECT_0 => {
                // SAFETY: 手动复位事件仍由 guard 持有，消费后复位供下次启动使用。
                if unsafe { ResetEvent(activation_event) } == 0 {
                    // SAFETY: 读取紧邻失败 ResetEvent 的线程本地错误码。
                    let code = unsafe { GetLastError() };
                    Err(SingleInstanceError::Platform { code })
                } else {
                    Ok(true)
                }
            }
            code => Err(SingleInstanceError::Platform { code }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_embedded_nul_names() {
        assert!(matches!(
            acquire_single_instance(""),
            Err(SingleInstanceError::EmptyName)
        ));
        assert!(matches!(
            acquire_single_instance("Local\\Floatile\0Other"),
            Err(SingleInstanceError::NameContainsNul)
        ));
    }

    #[cfg(not(windows))]
    #[test]
    fn unsupported_platform_is_explicit() {
        assert!(matches!(
            acquire_single_instance("Local\\Floatile.Test"),
            Ok(SingleInstanceState::Unsupported)
        ));
    }

    #[cfg(windows)]
    #[test]
    fn second_windows_acquisition_is_rejected_until_guard_drops() {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let name = format!("Local\\Floatile.Test.{}.{id}", std::process::id());

        {
            let first = acquire_single_instance(&name);
            let Ok(SingleInstanceState::Acquired(first)) = first else {
                panic!("first acquisition must succeed");
            };
            assert!(matches!(first.take_activation_request(), Ok(false)));
            assert!(matches!(
                acquire_single_instance(&name),
                Ok(SingleInstanceState::AlreadyRunning)
            ));
            assert!(matches!(first.take_activation_request(), Ok(true)));
            assert!(matches!(first.take_activation_request(), Ok(false)));
        }
        assert!(matches!(
            acquire_single_instance(&name),
            Ok(SingleInstanceState::Acquired(_))
        ));
    }
}
