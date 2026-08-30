//! Platform credential storage primitives.
//!
//! Callers receive borrowed secret bytes only for the duration of a callback. Windows stores
//! generic credentials in the current user's Credential Manager. Other targets explicitly report
//! unavailable until their platform keyring adapters are implemented.

const TARGET_PREFIX: &str = "Floatile/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlatformCredentialError {
    #[error("platform credential store is unavailable")]
    Unavailable,
    #[error("platform credential was not found")]
    NotFound,
    #[error("platform credential input is invalid")]
    InvalidInput,
    #[error("platform credential operation failed ({0})")]
    Platform(u32),
}

#[cfg(windows)]
fn target(reference: &str) -> Result<Vec<u16>, PlatformCredentialError> {
    if reference.is_empty() || reference.contains('\0') {
        return Err(PlatformCredentialError::InvalidInput);
    }
    Ok(format!("{TARGET_PREFIX}{reference}")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect())
}

#[cfg(windows)]
#[allow(unsafe_code)]
pub fn credential_put(reference: &str, secret: &[u8]) -> Result<(), PlatformCredentialError> {
    use windows_sys::Win32::Security::Credentials::{
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredWriteW,
    };

    if secret.is_empty() || secret.len() > u32::MAX as usize {
        return Err(PlatformCredentialError::InvalidInput);
    }
    let mut target = target(reference)?;
    // SAFETY: `CREDENTIALW` is a plain Win32 FFI structure whose null/zero fields are accepted for
    // generic credentials. Target and blob pointers remain valid for the synchronous call, sizes
    // match their backing buffers, and Windows copies the data before returning.
    let mut credential: CREDENTIALW = unsafe { std::mem::zeroed() };
    credential.Type = CRED_TYPE_GENERIC;
    credential.TargetName = target.as_mut_ptr();
    credential.CredentialBlobSize = secret.len() as u32;
    credential.CredentialBlob = secret.as_ptr().cast_mut();
    credential.Persist = CRED_PERSIST_LOCAL_MACHINE;
    // SAFETY: all pointer fields used by `CredWriteW` satisfy the lifetime and size invariants
    // established above; unused optional fields are null.
    if unsafe { CredWriteW(&raw const credential, 0) } == 0 {
        return Err(last_error());
    }
    Ok(())
}

#[cfg(windows)]
#[allow(unsafe_code)]
pub fn credential_delete(reference: &str) -> Result<bool, PlatformCredentialError> {
    use windows_sys::Win32::Security::Credentials::{CRED_TYPE_GENERIC, CredDeleteW};

    let target = target(reference)?;
    // SAFETY: target is NUL-terminated and remains alive for the synchronous Win32 call.
    if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0 {
        return Ok(true);
    }
    match last_error() {
        PlatformCredentialError::NotFound => Ok(false),
        error => Err(error),
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
pub fn credential_with_secret(
    reference: &str,
    use_secret: &mut dyn FnMut(&[u8]),
) -> Result<(), PlatformCredentialError> {
    use windows_sys::Win32::Security::Credentials::{
        CRED_TYPE_GENERIC, CREDENTIALW, CredFree, CredReadW,
    };

    let target = target(reference)?;
    let mut raw: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: target is NUL-terminated, the output pointer is valid, and a successful call returns
    // a Credential Manager allocation that remains valid until the matching `CredFree` below.
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut raw) } == 0 {
        return Err(last_error());
    }
    if raw.is_null() {
        return Err(PlatformCredentialError::Unavailable);
    }
    // SAFETY: successful `CredReadW` returned a valid `CREDENTIALW`; Windows guarantees the blob
    // pointer covers `CredentialBlobSize` bytes. The slice never escapes this callback.
    let secret = unsafe {
        let credential = &*raw;
        std::slice::from_raw_parts(
            credential.CredentialBlob,
            credential.CredentialBlobSize as usize,
        )
    };
    use_secret(secret);
    // SAFETY: `raw` is the exact allocation returned by `CredReadW` and is freed once.
    unsafe { CredFree(raw.cast()) };
    Ok(())
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn last_error() -> PlatformCredentialError {
    use windows_sys::Win32::Foundation::{ERROR_NOT_FOUND, GetLastError};
    // SAFETY: `GetLastError` has no pointer or lifetime requirements and is read immediately after
    // the failing Win32 credential call on the same thread.
    let code = unsafe { GetLastError() };
    if code == ERROR_NOT_FOUND {
        PlatformCredentialError::NotFound
    } else {
        PlatformCredentialError::Platform(code)
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
pub fn credential_protect(secret: &[u8]) -> Result<Vec<u8>, PlatformCredentialError> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_LOCAL_MACHINE, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData,
    };
    if secret.is_empty() || secret.len() > u32::MAX as usize {
        return Err(PlatformCredentialError::InvalidInput);
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: secret.len() as u32,
        pbData: secret.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: input points at `secret` for its exact byte length, output is writable, optional
    // pointers are null, and UI is forbidden for this background host operation.
    if unsafe {
        CryptProtectData(
            &raw const input,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN | CRYPTPROTECT_LOCAL_MACHINE,
            &raw mut output,
        )
    } == 0
    {
        return Err(last_error());
    }
    // SAFETY: successful DPAPI output points to `cbData` bytes allocated by LocalAlloc.
    let protected =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    // SAFETY: output.pbData is the exact allocation returned by CryptProtectData.
    unsafe { LocalFree(output.pbData.cast()) };
    Ok(protected)
}

#[cfg(windows)]
#[allow(unsafe_code)]
pub fn credential_unprotect(protected: &[u8]) -> Result<Vec<u8>, PlatformCredentialError> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptUnprotectData,
    };
    if protected.is_empty() || protected.len() > u32::MAX as usize {
        return Err(PlatformCredentialError::InvalidInput);
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: protected.len() as u32,
        pbData: protected.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: input and output satisfy the DPAPI buffer contracts; optional output description
    // and entropy are omitted, and UI is forbidden.
    if unsafe {
        CryptUnprotectData(
            &raw const input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &raw mut output,
        )
    } == 0
    {
        return Err(last_error());
    }
    // SAFETY: successful DPAPI output points to `cbData` bytes allocated by LocalAlloc.
    let secret =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    // SAFETY: output.pbData is the exact allocation returned by CryptUnprotectData.
    unsafe { LocalFree(output.pbData.cast()) };
    Ok(secret)
}

#[cfg(not(windows))]
pub fn credential_put(_reference: &str, _secret: &[u8]) -> Result<(), PlatformCredentialError> {
    Err(PlatformCredentialError::Unavailable)
}

#[cfg(all(test, windows))]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn current_user_credential_manager_round_trips_or_reports_missing_logon_session() {
        let reference = format!("cred://floatile-test/platform-{}", std::process::id());
        let _ = credential_delete(&reference);
        if let Err(error) = credential_put(&reference, b"platform-vault-test") {
            assert_eq!(error, PlatformCredentialError::Platform(1312));
            return;
        }
        let mut observed = Vec::new();
        credential_with_secret(&reference, &mut |secret| observed.extend_from_slice(secret))
            .unwrap();
        assert_eq!(observed, b"platform-vault-test");
        assert!(credential_delete(&reference).unwrap());
    }

    #[test]
    fn current_user_dpapi_round_trip() {
        let protected = credential_protect(b"dpapi-vault-test").unwrap();
        assert!(
            !protected
                .windows(b"dpapi-vault-test".len())
                .any(|window| window == b"dpapi-vault-test")
        );
        assert_eq!(
            credential_unprotect(&protected).unwrap(),
            b"dpapi-vault-test"
        );
    }
}

#[cfg(not(windows))]
pub fn credential_delete(_reference: &str) -> Result<bool, PlatformCredentialError> {
    Err(PlatformCredentialError::Unavailable)
}

#[cfg(not(windows))]
pub fn credential_with_secret(
    _reference: &str,
    _use_secret: &mut dyn FnMut(&[u8]),
) -> Result<(), PlatformCredentialError> {
    Err(PlatformCredentialError::Unavailable)
}

#[cfg(not(windows))]
pub fn credential_protect(_secret: &[u8]) -> Result<Vec<u8>, PlatformCredentialError> {
    Err(PlatformCredentialError::Unavailable)
}

#[cfg(not(windows))]
pub fn credential_unprotect(_protected: &[u8]) -> Result<Vec<u8>, PlatformCredentialError> {
    Err(PlatformCredentialError::Unavailable)
}
