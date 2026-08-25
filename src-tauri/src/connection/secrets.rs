//! Capability secret: 256-bit OS-random generation and DPAPI-protected
//! persistence under `<app-data>/connection-secret.bin` (current-user scope).

use std::path::PathBuf;

/// Generates a 256-bit URL-safe capability token (no padding, no `/+=`).
fn generate_capability() -> Result<String, String> {
    use base64::Engine;
    use rand::TryRng;

    let mut bytes = [0u8; 32];
    // SysRng is rand 0.10's direct OS CSPRNG interface (formerly OsRng).
    rand::rngs::SysRng
        .try_fill_bytes(&mut bytes)
        .map_err(|e| format!("OS RNG unavailable: {e}"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(windows)]
fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    // CryptProtectData's signature takes `*mut CRYPT_INTEGER_BLOB`, so the
    // binding must be mutable even though DPAPI only reads it.
    let in_blob = CRYPT_INTEGER_BLOB {
        cbData: plain.len() as u32,
        pbData: plain.as_ptr() as *mut u8,
    };
    let mut out_blob = CRYPT_INTEGER_BLOB::default();
    // SAFETY: both blobs are valid for the call; the output blob is allocated
    // by DPAPI and freed with LocalFree in `blob_to_vec`. Current-user scope:
    // no CRYPTPROTECT_LOCAL_MACHINE flag is passed.
    let ok = unsafe {
        CryptProtectData(
            &in_blob,
            None, // no description string
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out_blob,
        )
    };
    ok.map_err(|e| format!("CryptProtectData failed: {e}"))?;
    Ok(unsafe { blob_to_vec(out_blob) })
}

#[cfg(windows)]
unsafe fn blob_to_vec(blob: windows::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB) -> Vec<u8> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    let slice = std::slice::from_raw_parts(blob.pbData, blob.cbData as usize);
    let data = slice.to_vec();
    let _ = LocalFree(Some(HLOCAL(blob.pbData as *mut _)));
    data
}

#[cfg(windows)]
fn unprotect(enc: &[u8]) -> Result<Vec<u8>, String> {
    use windows::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let in_blob = CRYPT_INTEGER_BLOB {
        cbData: enc.len() as u32,
        pbData: enc.as_ptr() as *mut u8,
    };
    let mut out_blob = CRYPT_INTEGER_BLOB::default();
    // SAFETY: same contract as `protect`; `in_blob` is only read.
    let ok = unsafe {
        CryptUnprotectData(
            &in_blob,
            None, // no description output
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out_blob,
        )
    };
    ok.map_err(|e| format!("CryptUnprotectData failed: {e}"))?;
    Ok(unsafe { blob_to_vec(out_blob) })
}

#[cfg(not(windows))]
fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
    Err("DPAPI storage is Windows-only".into())
}

#[cfg(not(windows))]
fn unprotect(_enc: &[u8]) -> Result<Vec<u8>, String> {
    Err("DPAPI storage is Windows-only".into())
}

/// Persists the capability secret encrypted at rest for the current user.
pub struct CapabilitySecretStore {
    file: PathBuf,
}

impl CapabilitySecretStore {
    pub fn new(app_data_dir: PathBuf) -> Self {
        Self {
            file: app_data_dir.join("connection-secret.bin"),
        }
    }

    /// Returns the stored secret, generating and persisting one if absent.
    pub fn load_or_create(&self) -> Result<String, String> {
        if self.file.exists() {
            let enc = std::fs::read(&self.file).map_err(|e| format!("read secret: {e}"))?;
            let plain = unprotect(&enc)?;
            return String::from_utf8(plain).map_err(|_| "stored secret is not UTF-8".into());
        }
        let token = generate_capability()?;
        self.persist(&token)?;
        Ok(token)
    }

    /// Replaces the stored secret with a freshly generated one.
    pub fn rotate(&self) -> Result<String, String> {
        let token = generate_capability()?;
        self.persist(&token)?;
        Ok(token)
    }

    /// Deletes the stored secret; the next load_or_create mints a new one.
    pub fn clear(&self) -> Result<(), String> {
        match std::fs::remove_file(&self.file) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("clear secret: {e}")),
        }
    }

    fn persist(&self, token: &str) -> Result<(), String> {
        if let Some(parent) = self.file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create app-data dir: {e}"))?;
        }
        let enc = protect(token.as_bytes())?;
        lc_core::ops::write_file::atomic_write(&self.file, &enc)
            .map_err(|e| format!("persist secret: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_capability_has_at_least_256_bits_of_source_entropy() {
        let a = generate_capability().unwrap();
        let b = generate_capability().unwrap();
        assert_ne!(a, b);
        assert!(!a.contains('/'));
        assert!(!a.contains('+'));
        assert!(!a.contains('='));
        assert!(a.len() >= 43);
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_roundtrip_hides_plaintext() {
        let plain = b"capability-secret-example";
        let enc = protect(plain).unwrap();
        assert!(!String::from_utf8_lossy(&enc).contains("capability-secret-example"));
        assert_eq!(unprotect(&enc).unwrap(), plain);
    }

    #[cfg(windows)]
    #[test]
    fn lifecycle_load_or_create_is_stable_then_rotate_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        let store = CapabilitySecretStore::new(dir.path().to_path_buf());

        let first = store.load_or_create().unwrap();
        let again = store.load_or_create().unwrap();
        assert_eq!(first, again);

        let rotated = store.rotate().unwrap();
        assert_ne!(first, rotated);
        assert_eq!(store.load_or_create().unwrap(), rotated);

        store.clear().unwrap();
        let fresh = store.load_or_create().unwrap();
        assert_ne!(fresh, rotated);
    }
}
