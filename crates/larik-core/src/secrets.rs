use std::collections::HashMap;
use std::sync::Mutex;

/// Stable target key used to store one workspace's provider password in the secret store.
pub fn secret_target(space_id: &str, key: &str) -> String {
    format!("Larik.SqlStudio/space/{space_id}/{key}")
}

/// Legacy target key written by the Tauri app via `keyring` v3 (`windows-native`).
/// `keyring::Entry::new("larik-sql-studio", &format!("space-password/{space_id}"))`
/// maps to Windows Credential Manager target `"{user}.{service}"`.
pub fn legacy_tauri_secret_target(space_id: &str) -> String {
    format!("space-password/{space_id}.larik-sql-studio")
}

/// Decodes a credential blob written by `keyring` v3 on Windows (`set_password` encodes UTF-16LE),
/// falling back to UTF-8 if the blob is not valid UTF-16LE.
pub fn decode_legacy_keyring_blob(bytes: &[u8]) -> anyhow::Result<String> {
    if bytes.is_empty() {
        return Ok(String::new());
    }
    if bytes.len() % 2 == 0 {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        if let Ok(decoded) = String::from_utf16(&units) {
            return Ok(decoded.trim_end_matches('\0').to_string());
        }
    }
    String::from_utf8(bytes.to_vec())
        .map_err(|e| anyhow::anyhow!("Legacy credential blob is neither valid UTF-16LE nor UTF-8: {e}"))
}

pub trait SecretBackend: Send + Sync {
    fn get(&self, target: &str) -> anyhow::Result<Option<String>>;
    fn set(&self, target: &str, value: &str) -> anyhow::Result<()>;
    fn delete(&self, target: &str) -> anyhow::Result<()>;

    /// Reads a legacy `keyring` v3 Windows Credential Manager entry (stored as UTF-16LE).
    /// Non-Windows / in-memory backends default to `self.get(target)`.
    fn get_legacy_keyring(&self, target: &str) -> anyhow::Result<Option<String>> {
        self.get(target)
    }
}

/// Windows Credential Manager backend (`CRED_TYPE_GENERIC`, `CRED_PERSIST_LOCAL_MACHINE`).
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsCredentialBackend;

#[cfg(windows)]
mod win_impl {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    use anyhow::Context;
    use windows_sys::Win32::Foundation::{ERROR_NOT_FOUND, GetLastError};
    use windows_sys::Win32::Security::Credentials::{
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
        CredReadW, CredWriteW,
    };

    use super::decode_legacy_keyring_blob;

    fn to_wide_null(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    fn read_raw_blob(target: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let target_w = to_wide_null(target);
        let mut cred_ptr: *mut CREDENTIALW = ptr::null_mut();

        // SAFETY: `target_w` is null-terminated UTF-16 and `cred_ptr` receives a Windows-allocated
        // `CREDENTIALW` pointer that we immediately copy from and free with `CredFree`.
        let ok = unsafe { CredReadW(target_w.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred_ptr) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            if err == ERROR_NOT_FOUND {
                return Ok(None);
            }
            return Err(std::io::Error::from_raw_os_error(err as i32))
                .with_context(|| format!("CredReadW failed for {target}"));
        }

        if cred_ptr.is_null() {
            return Ok(None);
        }

        // SAFETY: `cred_ptr` is valid until `CredFree`.
        let blob = unsafe {
            let cred = &*cred_ptr;
            let bytes = if cred.CredentialBlobSize == 0 || cred.CredentialBlob.is_null() {
                Vec::new()
            } else {
                std::slice::from_raw_parts(
                    cred.CredentialBlob,
                    cred.CredentialBlobSize as usize,
                )
                .to_vec()
            };
            CredFree(cred_ptr as *const core::ffi::c_void);
            bytes
        };

        Ok(Some(blob))
    }

    pub fn read_secret(target: &str) -> anyhow::Result<Option<String>> {
        let Some(bytes) = read_raw_blob(target)? else {
            return Ok(None);
        };
        if bytes.is_empty() {
            return Ok(Some(String::new()));
        }
        String::from_utf8(bytes)
            .map(Some)
            .with_context(|| format!("Credential blob for {target} is not valid UTF-8"))
    }

    pub fn read_legacy_keyring_secret(target: &str) -> anyhow::Result<Option<String>> {
        let Some(bytes) = read_raw_blob(target)? else {
            return Ok(None);
        };
        decode_legacy_keyring_blob(&bytes)
            .map(Some)
            .with_context(|| format!("Failed to decode legacy keyring blob for {target}"))
    }

    pub fn write_secret(target: &str, value: &str) -> anyhow::Result<()> {
        let mut target_w = to_wide_null(target);
        let mut user_w = to_wide_null("larik");
        let mut blob = value.as_bytes().to_vec();
        let blob_len = u32::try_from(blob.len())
            .with_context(|| format!("Secret value for {target} exceeds u32 length"))?;

        let cred = CREDENTIALW {
            Flags: 0,
            Type: CRED_TYPE_GENERIC,
            TargetName: target_w.as_mut_ptr(),
            Comment: ptr::null_mut(),
            LastWritten: unsafe { core::mem::zeroed() },
            CredentialBlobSize: blob_len,
            CredentialBlob: if blob.is_empty() {
                ptr::null_mut()
            } else {
                blob.as_mut_ptr()
            },
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            AttributeCount: 0,
            Attributes: ptr::null_mut(),
            TargetAlias: ptr::null_mut(),
            UserName: user_w.as_mut_ptr(),
        };

        // SAFETY: All pointers in `cred` remain alive across `CredWriteW`.
        let ok = unsafe { CredWriteW(&cred, 0) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            return Err(std::io::Error::from_raw_os_error(err as i32))
                .with_context(|| format!("CredWriteW failed for {target}"));
        }

        Ok(())
    }

    pub fn delete_secret(target: &str) -> anyhow::Result<()> {
        let target_w = to_wide_null(target);
        // SAFETY: `target_w` is null-terminated UTF-16.
        let ok = unsafe { CredDeleteW(target_w.as_ptr(), CRED_TYPE_GENERIC, 0) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            if err == ERROR_NOT_FOUND {
                return Ok(());
            }
            return Err(std::io::Error::from_raw_os_error(err as i32))
                .with_context(|| format!("CredDeleteW failed for {target}"));
        }
        Ok(())
    }
}

impl SecretBackend for WindowsCredentialBackend {
    fn get(&self, target: &str) -> anyhow::Result<Option<String>> {
        #[cfg(windows)]
        {
            win_impl::read_secret(target)
        }
        #[cfg(not(windows))]
        {
            let _ = target;
            Ok(None)
        }
    }

    fn set(&self, target: &str, value: &str) -> anyhow::Result<()> {
        #[cfg(windows)]
        {
            win_impl::write_secret(target, value)
        }
        #[cfg(not(windows))]
        {
            let _ = (target, value);
            Ok(())
        }
    }

    fn delete(&self, target: &str) -> anyhow::Result<()> {
        #[cfg(windows)]
        {
            win_impl::delete_secret(target)
        }
        #[cfg(not(windows))]
        {
            let _ = target;
            Ok(())
        }
    }

    fn get_legacy_keyring(&self, target: &str) -> anyhow::Result<Option<String>> {
        #[cfg(windows)]
        {
            win_impl::read_legacy_keyring_secret(target)
        }
        #[cfg(not(windows))]
        {
            let _ = target;
            Ok(None)
        }
    }
}

/// Deterministic in-memory secret backend for unit tests.
#[derive(Debug, Default)]
pub struct InMemorySecretBackend {
    entries: Mutex<HashMap<String, String>>,
}

impl InMemorySecretBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.lock().map(|g| g.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl SecretBackend for InMemorySecretBackend {
    fn get(&self, target: &str) -> anyhow::Result<Option<String>> {
        let guard = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("InMemorySecretBackend lock poisoned"))?;
        Ok(guard.get(target).cloned())
    }

    fn set(&self, target: &str, value: &str) -> anyhow::Result<()> {
        let mut guard = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("InMemorySecretBackend lock poisoned"))?;
        guard.insert(target.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, target: &str) -> anyhow::Result<()> {
        let mut guard = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("InMemorySecretBackend lock poisoned"))?;
        guard.remove(target);
        Ok(())
    }
}
