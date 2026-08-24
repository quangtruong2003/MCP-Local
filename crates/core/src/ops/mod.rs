pub mod find_files;
pub mod list_dir;
pub mod read_file;
pub mod search_text;
pub mod write_file;

use crate::error::{CoreError, ErrorCode};
use crate::CoreServices;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Hard bounds on read-only op results (spec section 6 error model: results
/// that cannot be truncated sensibly fail with RESULT_TOO_LARGE instead).
pub const MAX_READ_BYTES: u64 = 262_144;
/// Upper bound on the size of a single file `read_file` will load at all;
/// larger files fail fast with `RESULT_TOO_LARGE` instead of being read into
/// memory. Files up to this size are still window-truncated to
/// [`MAX_READ_BYTES`] as before.
pub const MAX_INPUT_FILE_BYTES: u64 = 10 * 1024 * 1024;
pub const MAX_LIST_ENTRIES: usize = 5_000;
pub const MAX_FIND_PATHS: usize = 500;
pub const MAX_SEARCH_MATCHES: usize = 200;
pub const MAX_READ_FILES: usize = 8;

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex(&h.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn canonical_root(root: &Path) -> Result<PathBuf, CoreError> {
    dunce::canonicalize(root).map_err(CoreError::io)
}

/// Confinement + sensitive check shared by every op.
/// Returns (resolved absolute path, path relative to canonical root).
pub fn vet_path(
    core: &CoreServices,
    root: &Path,
    rel: &str,
) -> Result<(PathBuf, PathBuf), CoreError> {
    let resolved = crate::confinement::ensure_inside(root, &root.join(rel))?;
    let rel_path = resolved
        .strip_prefix(canonical_root(root)?)
        .map_err(|_| {
            CoreError::new(
                ErrorCode::PathOutsideWorkspace,
                "resolved path lost its root",
            )
        })?
        .to_path_buf();
    if core.sensitive.is_blocked(&rel_path) {
        return Err(CoreError::new(
            ErrorCode::SensitiveFileBlocked,
            format!("{rel} matches the sensitive-file deny list"),
        ));
    }
    Ok((resolved, rel_path))
}

/// Walker that respects .gitignore and skips hidden entries.
pub fn walker(root: &Path) -> ignore::WalkBuilder {
    let mut b = ignore::WalkBuilder::new(root);
    b.hidden(true).git_ignore(true);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_is_stable_hex() {
        assert_eq!(sha256_hex(b"hello").len(), 64);
        assert_eq!(sha256_hex(b"hello"), sha256_hex(b"hello"));
        assert_ne!(sha256_hex(b"hello"), sha256_hex(b"hellp"));
    }
}
