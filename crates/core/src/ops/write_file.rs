use super::*;
use crate::error::{CoreError, ErrorCode};
use serde::Serialize;
use std::path::Path;

/// Hard bound on content accepted by `create_file`.
pub const MAX_CREATE_BYTES: usize = 10 * 1024 * 1024;

/// Durability: writes `bytes` to a unique temp file in the target's directory,
/// then renames it over the target so readers never see a partial file.
pub fn atomic_write(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = target.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "target has no parent dir")
    })?;
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, bytes)?;
    match std::fs::rename(&tmp, target) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CreateResult {
    pub path: String,
    pub revision: String,
}

pub fn create_file(
    core: &CoreServices,
    root: &Path,
    rel: &str,
    content: &str,
) -> Result<CreateResult, CoreError> {
    let (resolved, rel_path) = vet_path(core, root, rel)?;
    if content.len() > MAX_CREATE_BYTES {
        return Err(CoreError::new(
            ErrorCode::InvalidArgument,
            format!("content exceeds {MAX_CREATE_BYTES} bytes"),
        ));
    }
    if resolved.exists() {
        return Err(CoreError::new(
            ErrorCode::FileExists,
            format!("{rel} already exists"),
        ));
    }
    if let Some(parent) = resolved.parent() {
        std::fs::create_dir_all(parent).map_err(CoreError::io)?;
    }
    atomic_write(&resolved, content.as_bytes()).map_err(CoreError::io)?;
    Ok(CreateResult {
        path: rel_path.to_string_lossy().replace('\\', "/"),
        revision: crate::ops::sha256_hex(content.as_bytes()),
    })
}

#[derive(Debug, Serialize)]
pub struct PatchArgs {
    pub rel: String,
    pub old_string: String,
    pub new_string: String,
    /// sha256 of the file as returned by the last read; the patch is refused
    /// with FILE_CHANGED_SINCE_READ unless the on-disk content still hashes to
    /// this value.
    pub base_revision: String,
}

#[derive(Debug, Serialize)]
pub struct PatchResult {
    pub path: String,
    pub revision: String,
    pub bytes_changed: i64,
}

pub fn apply_patch(
    core: &CoreServices,
    root: &Path,
    args: &PatchArgs,
) -> Result<PatchResult, CoreError> {
    let (resolved, rel_path) = vet_path(core, root, &args.rel)?;
    let bytes = std::fs::read(&resolved)
        .map_err(|_| CoreError::new(ErrorCode::FileNotFound, format!("{} not found", args.rel)))?;
    let text = String::from_utf8(bytes.clone()).map_err(|_| {
        CoreError::new(
            ErrorCode::InvalidArgument,
            format!("{} is not valid UTF-8", args.rel),
        )
    })?;

    // Conflict detection: the caller must have seen exactly these bytes.
    let actual_rev = crate::ops::sha256_hex(&bytes);
    if actual_rev != args.base_revision {
        return Err(CoreError::new(
            ErrorCode::FileChangedSinceRead,
            "file changed since it was read; re-read and retry",
        )
        .with_detail(serde_json::json!({
            "expected": args.base_revision, "actual": actual_rev })));
    }

    // Anchor must exist and be unambiguous.
    let count = text.matches(&args.old_string).count();
    match count {
        0 => {
            return Err(CoreError::new(
                ErrorCode::InvalidPatch,
                "anchor not found in file; re-read the file first",
            ))
        }
        n if n > 1 => {
            return Err(CoreError::new(
                ErrorCode::InvalidPatch,
                format!(
                    "anchor matches {n} times; include more surrounding context to make it unique"
                ),
            ))
        }
        _ => {}
    }

    let patched = text.replacen(&args.old_string, &args.new_string, 1);
    atomic_write(&resolved, patched.as_bytes()).map_err(CoreError::io)?;
    Ok(PatchResult {
        path: rel_path.to_string_lossy().replace('\\', "/"),
        revision: crate::ops::sha256_hex(patched.as_bytes()),
        bytes_changed: patched.len() as i64 - text.len() as i64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::ops::read_file::read_file;
    use crate::pipeline::run_tool_audited;
    use crate::workspace_repo::update_capabilities;
    use std::fs;

    fn setup() -> (crate::CoreServices, tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::Database::open(&dir.path().join("t.db")).unwrap();
        fs::create_dir_all(dir.path().join("ws")).unwrap();
        let ws = crate::workspace_repo::insert_workspace(
            &db,
            "W",
            &dir.path().join("ws").to_string_lossy(),
        )
        .unwrap();
        // write tools need can_write=true (defaults false)
        update_capabilities(&db, &ws.id, true, true).unwrap();
        let core = crate::CoreServices {
            db,
            pause: Default::default(),
            sensitive: crate::sensitive::SensitiveMatcher::new(),
        };
        (core, dir, ws.id)
    }

    #[test]
    fn create_then_read_back() {
        let (core, _d, ws) = setup();
        let r = run_tool_audited(&core, "create_file", &ws, true, |_, root| {
            create_file(&core, root, "new.txt", "hello")
        })
        .unwrap();
        assert_eq!(r.revision.len(), 64);
        let on_disk = fs::read_to_string(_d.path().join("ws/new.txt")).unwrap();
        assert_eq!(on_disk, "hello");
    }

    #[test]
    fn create_existing_is_file_exists() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/x.txt"), "old").unwrap();
        let e = run_tool_audited(&core, "create_file", &ws, true, |_, root| {
            create_file(&core, root, "x.txt", "new")
        })
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::FileExists);
        // original untouched (atomicity)
        assert_eq!(
            fs::read_to_string(d.path().join("ws/x.txt")).unwrap(),
            "old"
        );
    }

    #[test]
    fn create_too_large_rejected() {
        let (core, _d, ws) = setup();
        let big = "x".repeat(10 * 1024 * 1024 + 1);
        let e = run_tool_audited(&core, "create_file", &ws, true, |_, root| {
            create_file(&core, root, "big.txt", &big)
        })
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArgument);
    }

    #[test]
    fn create_into_sensitive_path_blocked() {
        let (core, _d, ws) = setup();
        let e = run_tool_audited(&core, "create_file", &ws, true, |_, root| {
            create_file(&core, root, ".env", "SECRET=1")
        })
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::SensitiveFileBlocked);
    }

    #[test]
    fn patch_replaces_unique_anchor() {
        let (core, d, ws) = setup();
        fs::write(
            d.path().join("ws/f.txt"),
            "fn main() {\n    println!(\"hi\");\n}\n",
        )
        .unwrap();
        let rev0 = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, "f.txt", None, None)
        })
        .unwrap()
        .revision;
        let r = run_tool_audited(&core, "apply_patch", &ws, true, |_, root| {
            apply_patch(
                &core,
                root,
                &PatchArgs {
                    rel: "f.txt".into(),
                    old_string: "println!(\"hi\");".into(),
                    new_string: "println!(\"hello\");".into(),
                    base_revision: rev0,
                },
            )
        })
        .unwrap();
        assert_ne!(r.revision.len(), 0);
        assert_eq!(r.bytes_changed, 3); // "hi" (2) -> "hello" (5): +3 bytes
        let now = fs::read_to_string(d.path().join("ws/f.txt")).unwrap();
        assert_eq!(now, "fn main() {\n    println!(\"hello\");\n}\n");
    }

    #[test]
    fn patch_ambiguous_anchor_rejected() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/f.txt"), "a\na\n").unwrap();
        let rev = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, "f.txt", None, None)
        })
        .unwrap()
        .revision;
        let e = run_tool_audited(&core, "apply_patch", &ws, true, |_, root| {
            apply_patch(
                &core,
                root,
                &PatchArgs {
                    rel: "f.txt".into(),
                    old_string: "a".into(),
                    new_string: "b".into(),
                    base_revision: rev,
                },
            )
        })
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidPatch);
    }

    #[test]
    fn patch_missing_anchor_is_invalid_patch() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/f.txt"), "real content\n").unwrap();
        let rev = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, "f.txt", None, None)
        })
        .unwrap()
        .revision;
        let e = run_tool_audited(&core, "apply_patch", &ws, true, |_, root| {
            apply_patch(
                &core,
                root,
                &PatchArgs {
                    rel: "f.txt".into(),
                    old_string: "not present".into(),
                    new_string: "b".into(),
                    base_revision: rev,
                },
            )
        })
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidPatch);
        assert_eq!(
            fs::read_to_string(d.path().join("ws/f.txt")).unwrap(),
            "real content\n"
        );
    }

    #[test]
    fn patch_stale_revision_rejected() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/f.txt"), "v1").unwrap();
        let stale_rev = crate::ops::sha256_hex(b"v0");
        let e = run_tool_audited(&core, "apply_patch", &ws, true, |_, root| {
            apply_patch(
                &core,
                root,
                &PatchArgs {
                    rel: "f.txt".into(),
                    old_string: "v1".into(),
                    new_string: "v2".into(),
                    base_revision: stale_rev,
                },
            )
        })
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::FileChangedSinceRead);
        assert_eq!(fs::read_to_string(d.path().join("ws/f.txt")).unwrap(), "v1");
    }

    #[test]
    fn patch_crlf_file_preserves_line_endings() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/f.txt"), "one\r\ntwo\r\n").unwrap();
        let rev = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, "f.txt", None, None)
        })
        .unwrap()
        .revision;
        run_tool_audited(&core, "apply_patch", &ws, true, |_, root| {
            apply_patch(
                &core,
                root,
                &PatchArgs {
                    rel: "f.txt".into(),
                    old_string: "two".into(),
                    new_string: "TWO".into(),
                    base_revision: rev,
                },
            )
        })
        .unwrap();
        assert_eq!(
            fs::read_to_string(d.path().join("ws/f.txt")).unwrap(),
            "one\r\nTWO\r\n"
        );
    }
}
