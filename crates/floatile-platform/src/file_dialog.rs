//! 宿主拥有的原生文件选择器。
//!
//! 对话框是阻塞式平台 API，调用方必须在非 UI 线程调用。本模块只返回选择结果，
//! 不读取包内容，也不绕过 CLI/安装器的预算、路径和原子安装校验。

use std::path::PathBuf;

use crate::PlatformError;

/// 平台文件对话框的宿主 owner；原生句柄保持私有，Shell 只能原样传回平台层。
#[derive(Debug, Clone, Copy)]
pub struct FileDialogOwner {
    #[cfg(windows)]
    hwnd: windows_sys::Win32::Foundation::HWND,
}

/// 从存活的宿主窗口取得文件对话框 owner。
pub fn file_dialog_owner(window: &winit::window::Window) -> Result<FileDialogOwner, PlatformError> {
    #[cfg(windows)]
    {
        windows_impl::owner(window)
    }

    #[cfg(not(windows))]
    {
        let _ = window;
        Ok(FileDialogOwner {})
    }
}

/// 选择一个本地 `.floatile` 插件包；用户取消返回 `Ok(None)`。
pub fn pick_floatile_package(owner: FileDialogOwner) -> Result<Option<PathBuf>, PlatformError> {
    #[cfg(windows)]
    {
        windows_impl::pick_floatile_package(owner)
    }

    #[cfg(not(windows))]
    {
        Err(PlatformError::Unsupported(
            "native Floatile package picker is not implemented on this platform",
        ))
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_impl {
    use std::ffi::OsString;
    use std::mem::size_of;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;

    use windows_sys::Win32::UI::Controls::Dialogs::{
        CommDlgExtendedError, GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR,
        OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use crate::PlatformError;
    use crate::file_dialog::FileDialogOwner;

    const MAX_DIALOG_PATH_U16: usize = 32_768;

    pub(super) fn owner(window: &winit::window::Window) -> Result<FileDialogOwner, PlatformError> {
        let handle = window
            .window_handle()
            .map_err(|error| PlatformError::Platform(error.to_string()))?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err(PlatformError::Unsupported(
                "file dialog owner is not a Win32 window",
            ));
        };
        Ok(FileDialogOwner {
            hwnd: handle.hwnd.get(),
        })
    }

    pub(super) fn pick_floatile_package(
        owner: FileDialogOwner,
    ) -> Result<Option<PathBuf>, PlatformError> {
        let filter: Vec<u16> = "Floatile 插件包 (*.floatile)\0*.floatile\0所有文件 (*.*)\0*.*\0\0"
            .encode_utf16()
            .collect();
        let title: Vec<u16> = "选择 Floatile 插件包"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let default_extension: Vec<u16> = "floatile"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut path = vec![0u16; MAX_DIALOG_PATH_U16];
        // SAFETY: OPENFILENAMEW 允许未使用字段为零；随后设置结构尺寸、指针、容量
        // 和 flags。所有指针指向在 GetOpenFileNameW 返回前保持存活的缓冲区。
        let mut options: OPENFILENAMEW = unsafe { std::mem::zeroed() };
        options.lStructSize = u32::try_from(size_of::<OPENFILENAMEW>())
            .map_err(|error| PlatformError::Platform(error.to_string()))?;
        options.hwndOwner = owner.hwnd;
        options.lpstrFilter = filter.as_ptr();
        options.nFilterIndex = 1;
        options.lpstrFile = path.as_mut_ptr();
        options.nMaxFile = u32::try_from(path.len())
            .map_err(|error| PlatformError::Platform(error.to_string()))?;
        options.lpstrTitle = title.as_ptr();
        options.lpstrDefExt = default_extension.as_ptr();
        options.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;

        // SAFETY: options 与其指向的 UTF-16 缓冲区在调用期间有效；API 仅在 path
        // 容量内写入以 NUL 结尾的所选路径。
        if unsafe { GetOpenFileNameW(&raw mut options) } == 0 {
            // SAFETY: 紧邻失败/取消的 common-dialog 调用读取扩展错误；0 表示用户取消。
            let code = unsafe { CommDlgExtendedError() };
            return if code == 0 {
                Ok(None)
            } else {
                Err(PlatformError::Platform(format!(
                    "GetOpenFileNameW failed: {code}"
                )))
            };
        }
        let len = path
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(path.len());
        Ok(Some(PathBuf::from(OsString::from_wide(&path[..len]))))
    }
}
