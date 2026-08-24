//! Secure storage for tunnel credentials.
//!
//! Production Windows builds encrypt the credential payload with DPAPI
//! (`CryptProtectData` / `CryptUnprotectData`) before persisting it under the
//! current user's LocalAppData. No additional credential crate is required.
//! Tests can opt into an isolated file backend so they do not mutate the
//! developer's real protected store.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const SERVICE: &str = "com.local.workspace-mcp-manager";
const RUNTIME_USER: &str = "tunnel-runtime-api-key";
const ADMIN_USER: &str = "tunnel-admin-api-key";
const STORE_FILE: &str = "credentials.dpapi";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretSummary {
    pub has_runtime_key: bool,
    pub has_admin_key: bool,
}

#[derive(Debug, Clone)]
enum Backend {
    System(PathBuf),
    TestFile(PathBuf),
}

#[derive(Debug, Clone)]
pub struct SecretStore {
    backend: Backend,
}

impl Default for SecretStore {
    fn default() -> Self {
        Self::system()
    }
}

impl SecretStore {
    pub fn system() -> Self {
        Self {
            backend: Backend::System(system_store_path()),
        }
    }

    /// Test-only constructor. Public because integration tests live outside
    /// the crate; the desktop app never calls it.
    pub fn for_test_file(path: PathBuf) -> Self {
        Self {
            backend: Backend::TestFile(path),
        }
    }

    pub fn load_runtime_key(&self) -> Result<Option<String>, String> {
        self.load(RUNTIME_USER)
    }

    pub fn load_admin_key(&self) -> Result<Option<String>, String> {
        self.load(ADMIN_USER)
    }

    /// Replaces only non-blank values. This lets the Settings page show empty
    /// password fields without accidentally erasing an already configured key.
    pub fn save(&self, runtime_key: Option<&str>, admin_key: Option<&str>) -> Result<(), String> {
        if let Some(value) = non_blank(runtime_key) {
            self.store(RUNTIME_USER, value)?;
        }
        if let Some(value) = non_blank(admin_key) {
            self.store(ADMIN_USER, value)?;
        }
        Ok(())
    }

    pub fn summary(&self) -> Result<SecretSummary, String> {
        Ok(SecretSummary {
            has_runtime_key: self.load_runtime_key()?.is_some(),
            has_admin_key: self.load_admin_key()?.is_some(),
        })
    }

    fn load(&self, user: &str) -> Result<Option<String>, String> {
        match &self.backend {
            Backend::System(path) => system_load(path, user),
            Backend::TestFile(path) => {
                let map = read_test_map(path)?;
                Ok(match user {
                    RUNTIME_USER => map.runtime,
                    ADMIN_USER => map.admin,
                    _ => None,
                })
            }
        }
    }

    fn store(&self, user: &str, value: &str) -> Result<(), String> {
        match &self.backend {
            Backend::System(path) => system_store(path, user, value),
            Backend::TestFile(path) => {
                let mut map = read_test_map(path)?;
                match user {
                    RUNTIME_USER => map.runtime = Some(value.to_string()),
                    ADMIN_USER => map.admin = Some(value.to_string()),
                    _ => return Err("unknown secret slot".into()),
                }
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(path, serde_json::to_vec(&map).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())
            }
        }
    }
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct TestSecretMap {
    runtime: Option<String>,
    admin: Option<String>,
}

fn read_test_map(path: &Path) -> Result<TestSecretMap, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(TestSecretMap::default()),
        Err(e) => Err(e.to_string()),
    }
}

fn system_store_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join(SERVICE).join(STORE_FILE)
}

#[cfg(windows)]
#[repr(C)]
struct DataBlob {
    cb_data: u32,
    pb_data: *mut u8,
}

#[cfg(windows)]
#[allow(non_snake_case)]
#[link(name = "Crypt32")]
unsafe extern "system" {
    fn CryptProtectData(
        data_in: *const DataBlob,
        description: *const u16,
        optional_entropy: *const DataBlob,
        reserved: *mut std::ffi::c_void,
        prompt: *const std::ffi::c_void,
        flags: u32,
        data_out: *mut DataBlob,
    ) -> i32;

    fn CryptUnprotectData(
        data_in: *const DataBlob,
        description: *mut *mut u16,
        optional_entropy: *const DataBlob,
        reserved: *mut std::ffi::c_void,
        prompt: *const std::ffi::c_void,
        flags: u32,
        data_out: *mut DataBlob,
    ) -> i32;
}

#[cfg(windows)]
#[allow(non_snake_case)]
#[link(name = "Kernel32")]
unsafe extern "system" {
    fn LocalFree(memory: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
}

#[cfg(windows)]
fn dpapi_transform(bytes: &[u8], protect: bool) -> Result<Vec<u8>, String> {
    let length = u32::try_from(bytes.len()).map_err(|_| "credential payload is too large")?;
    let input = DataBlob {
        cb_data: length,
        pb_data: bytes.as_ptr() as *mut u8,
    };
    let mut output = DataBlob {
        cb_data: 0,
        pb_data: std::ptr::null_mut(),
    };
    const UI_FORBIDDEN: u32 = 0x1;
    let ok = unsafe {
        if protect {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 {
        return Err(format!(
            "Windows DPAPI operation failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let result = if output.cb_data == 0 || output.pb_data.is_null() {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(output.pb_data, output.cb_data as usize) }.to_vec()
    };
    if !output.pb_data.is_null() {
        unsafe {
            let _ = LocalFree(output.pb_data.cast());
        }
    }
    Ok(result)
}

#[cfg(windows)]
fn read_system_map(path: &Path) -> Result<TestSecretMap, String> {
    match std::fs::read(path) {
        Ok(ciphertext) => {
            let plaintext = dpapi_transform(&ciphertext, false)?;
            serde_json::from_slice(&plaintext).map_err(|e| e.to_string())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(TestSecretMap::default()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(windows)]
fn system_load(path: &Path, user: &str) -> Result<Option<String>, String> {
    let map = read_system_map(path)?;
    Ok(match user {
        RUNTIME_USER => map.runtime,
        ADMIN_USER => map.admin,
        _ => None,
    })
}

#[cfg(windows)]
fn system_store(path: &Path, user: &str, value: &str) -> Result<(), String> {
    let mut map = read_system_map(path)?;
    match user {
        RUNTIME_USER => map.runtime = Some(value.to_string()),
        ADMIN_USER => map.admin = Some(value.to_string()),
        _ => return Err("unknown secret slot".into()),
    }
    let plaintext = serde_json::to_vec(&map).map_err(|e| e.to_string())?;
    let ciphertext = dpapi_transform(&plaintext, true)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, ciphertext).map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn system_load(_path: &Path, _user: &str) -> Result<Option<String>, String> {
    Err("secure credential storage is supported by this app on Windows builds".into())
}

#[cfg(not(windows))]
fn system_store(_path: &Path, _user: &str, _value: &str) -> Result<(), String> {
    Err("secure credential storage is supported by this app on Windows builds".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_values_are_not_replacements() {
        assert_eq!(non_blank(Some("  ")), None);
        assert_eq!(non_blank(Some(" x ")), Some("x"));
    }

    #[test]
    fn service_name_is_stable() {
        assert_eq!(SERVICE, "com.local.workspace-mcp-manager");
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_backend_roundtrips_without_plaintext_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.dpapi");
        let store = SecretStore {
            backend: Backend::System(path.clone()),
        };
        store
            .save(Some("runtime-super-secret"), Some("admin-super-secret"))
            .unwrap();
        let raw = std::fs::read(path).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("super-secret"));
        assert_eq!(
            store.load_runtime_key().unwrap().as_deref(),
            Some("runtime-super-secret")
        );
        assert_eq!(
            store.load_admin_key().unwrap().as_deref(),
            Some("admin-super-secret")
        );
    }
}
