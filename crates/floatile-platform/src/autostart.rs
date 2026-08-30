//! Current-user desktop autostart integration.
//!
//! Windows uses the current user's `Run` registry key and launches the exact host executable with
//! `--background`. Other targets report unsupported until their native login-item adapters exist.

use std::path::Path;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AutostartState {
    Disabled,
    Enabled,
    Stale,
    Unavailable,
    #[default]
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AutostartError {
    #[error("desktop autostart is unsupported")]
    Unsupported,
    #[error("desktop autostart path is invalid")]
    InvalidPath,
    #[error("desktop autostart operation failed ({0})")]
    Platform(u32),
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn command(executable: &Path) -> Result<Vec<u16>, AutostartError> {
    use std::os::windows::ffi::OsStrExt;

    let path = executable.as_os_str().encode_wide().collect::<Vec<_>>();
    if path.is_empty() || path.contains(&0) {
        return Err(AutostartError::InvalidPath);
    }
    let mut value = Vec::with_capacity(path.len().saturating_add(18));
    value.push(u16::from(b'"'));
    value.extend(path);
    value.extend("\" --background".encode_utf16());
    value.push(0);
    Ok(value)
}

#[cfg(windows)]
const MAX_RUN_VALUE_BYTES: u32 = 32 * 1024;

#[cfg(windows)]
#[allow(unsafe_code)]
pub fn autostart_state(executable: &Path) -> Result<AutostartState, AutostartError> {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_INVALID_DATA};
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};

    let key = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    let name = wide("Floatile");
    let mut byte_len = 0u32;
    // SAFETY: key/name are NUL-terminated. A null data pointer with a valid size pointer is the
    // documented size query form; no registry data is written during this call.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut byte_len,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(AutostartState::Disabled);
    }
    if status != 0 {
        return Err(AutostartError::Platform(status));
    }
    if byte_len == 0 || byte_len > MAX_RUN_VALUE_BYTES || !byte_len.is_multiple_of(2) {
        return Err(AutostartError::Platform(ERROR_INVALID_DATA));
    }
    let mut actual = vec![0u16; byte_len as usize / 2];
    // SAFETY: the byte count exactly covers `actual`; pointers remain live for the synchronous
    // call, and `RRF_RT_REG_SZ` rejects non-string registry types.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            actual.as_mut_ptr().cast(),
            &mut byte_len,
        )
    };
    if status != 0 {
        return Err(AutostartError::Platform(status));
    }
    actual.truncate(byte_len as usize / 2);
    Ok(if actual == command(executable)? {
        AutostartState::Enabled
    } else {
        AutostartState::Stale
    })
}

#[cfg(windows)]
#[allow(unsafe_code)]
pub fn set_autostart(executable: &Path, enabled: bool) -> Result<(), AutostartError> {
    use windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW,
    };

    let key = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    let name = wide("Floatile");
    let status = if enabled {
        let value = command(executable)?;
        let byte_len = u32::try_from(value.len().saturating_mul(2))
            .map_err(|_| AutostartError::InvalidPath)?;
        // SAFETY: key/name/value are NUL-terminated and remain live for the synchronous call;
        // byte_len covers the full UTF-16 REG_SZ value including its terminator.
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                REG_SZ,
                value.as_ptr().cast(),
                byte_len,
            )
        }
    } else {
        // SAFETY: key/name are NUL-terminated and remain live for the synchronous call.
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) }
    };
    if status == 0 || (!enabled && status == ERROR_FILE_NOT_FOUND) {
        Ok(())
    } else {
        Err(AutostartError::Platform(status))
    }
}

#[cfg(not(windows))]
pub fn autostart_state(_executable: &Path) -> Result<AutostartState, AutostartError> {
    Ok(AutostartState::Unsupported)
}

#[cfg(not(windows))]
pub fn set_autostart(_executable: &Path, _enabled: bool) -> Result<(), AutostartError> {
    Err(AutostartError::Unsupported)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_command_quotes_the_exact_executable_and_uses_background_mode() {
        use std::os::windows::ffi::OsStringExt;

        let encoded = command(Path::new(r"C:\Program Files\Floatile\floatile-shell.exe")).unwrap();
        let decoded = std::ffi::OsString::from_wide(&encoded[..encoded.len() - 1]);
        assert_eq!(
            decoded,
            r#""C:\Program Files\Floatile\floatile-shell.exe" --background"#
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn unsupported_targets_degrade_explicitly() {
        assert_eq!(
            autostart_state(Path::new("/tmp/floatile")),
            Ok(AutostartState::Unsupported)
        );
        assert_eq!(
            set_autostart(Path::new("/tmp/floatile"), true),
            Err(AutostartError::Unsupported)
        );
    }
}
