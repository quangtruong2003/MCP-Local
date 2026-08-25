use super::*;
use crate::error::{CoreError, ErrorCode};
use crate::pipeline::Audited;
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
    if let Err(e) = std::fs::write(&tmp, bytes) {
        // Do not leak a partial temp file when the write itself fails.
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    match replace_file(&tmp, target) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Replaces `target` with `tmp`. On Windows, `std::fs::rename` fails when the
/// destination exists, so use `MoveFileExW` with REPLACE_EXISTING explicitly;
/// WRITE_THROUGH flushes the move before returning. On other platforms a plain
/// rename already replaces atomically.
#[cfg(windows)]
fn replace_file(tmp: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    fn wide(p: &Path) -> Vec<u16> {
        p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    let from = wide(tmp);
    let to = wide(target);
    // SAFETY: both pointers are null-terminated wide strings valid for the
    // duration of the call; MoveFileExW does not retain them.
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
                | windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(tmp: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::rename(tmp, target)
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct CreateResult {
    pub path: String,
    pub revision: String,
}

pub fn create_file(
    core: &CoreServices,
    root: &Path,
    rel: &str,
    content: &str,
) -> Result<Audited<CreateResult>, CoreError> {
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
    let content_len = content.len();
    let path_str = rel_path.to_string_lossy().replace('\\', "/");
    Ok(Audited::new(CreateResult {
        path: path_str.clone(),
        revision: crate::ops::sha256_hex(content.as_bytes()),
    })
    .with_path(path_str)
    .with_detail(serde_json::json!({ "bytes": content_len })))
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

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PatchResult {
    pub path: String,
    pub revision: String,
    pub bytes_changed: i64,
}

pub fn apply_patch(
    core: &CoreServices,
    root: &Path,
    args: &PatchArgs,
) -> Result<Audited<PatchResult>, CoreError> {
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
    let bytes_changed = patched.len() as i64 - text.len() as i64;
    let path_str = rel_path.to_string_lossy().replace('\\', "/");
    Ok(Audited::new(PatchResult {
        path: path_str.clone(),
        revision: crate::ops::sha256_hex(patched.as_bytes()),
        bytes_changed,
    })
    .with_path(path_str)
    .with_detail(serde_json::json!({ "bytes_changed": bytes_changed })))
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
            unrestricted: crate::pause::Flag::default(),
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

    #[test]
    fn patch_missing_file_is_file_not_found() {
        let (core, _d, ws) = setup();
        let e = run_tool_audited(&core, "apply_patch", &ws, true, |_, root| {
            apply_patch(
                &core,
                root,
                &PatchArgs {
                    rel: "ghost.txt".into(),
                    old_string: "a".into(),
                    new_string: "b".into(),
                    base_revision: "0".repeat(64),
                },
            )
        })
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::FileNotFound);
    }

    #[test]
    fn patch_non_utf8_is_invalid_argument() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/bin.dat"), [0xFF, 0xFE, 0x00]).unwrap();
        let rev = crate::ops::sha256_hex(&[0xFF, 0xFE, 0x00]);
        let e = run_tool_audited(&core, "apply_patch", &ws, true, |_, root| {
            apply_patch(
                &core,
                root,
                &PatchArgs {
                    rel: "bin.dat".into(),
                    old_string: "a".into(),
                    new_string: "b".into(),
                    base_revision: rev,
                },
            )
        })
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArgument);
        // File untouched.
        assert_eq!(
            fs::read(d.path().join("ws/bin.dat")).unwrap(),
            vec![0xFF, 0xFE, 0x00]
        );
    }

    #[test]
    fn patch_on_sensitive_file_blocked() {
        let (core, d, ws) = setup();
        // vet_path runs before anything else, so even an existing .env is
        // refused without reading it.
        fs::write(d.path().join("ws/.env"), "SECRET=1").unwrap();
        let e = run_tool_audited(&core, "apply_patch", &ws, true, |_, root| {
            apply_patch(
                &core,
                root,
                &PatchArgs {
                    rel: ".env".into(),
                    old_string: "SECRET=1".into(),
                    new_string: "SECRET=2".into(),
                    base_revision: crate::ops::sha256_hex(b"SECRET=1"),
                },
            )
        })
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::SensitiveFileBlocked);
    }

    #[test]
    fn patch_noop_old_equals_new_keeps_revision_stable() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/f.txt"), "same text\n").unwrap();
        let rev = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
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
                    old_string: "same text\n".into(),
                    new_string: "same text\n".into(),
                    base_revision: rev.clone(),
                },
            )
        })
        .unwrap();
        assert_eq!(r.bytes_changed, 0);
        assert_eq!(r.revision, rev); // content unchanged -> same hash as pre-read revision
        assert_eq!(
            fs::read_to_string(d.path().join("ws/f.txt")).unwrap(),
            "same text\n"
        );
    }

    #[test]
    fn create_into_nested_dirs_makes_parents() {
        let (core, d, ws) = setup();
        let r = run_tool_audited(&core, "create_file", &ws, true, |_, root| {
            create_file(&core, root, "a/b/c/new.txt", "deep")
        })
        .unwrap();
        assert_eq!(r.path, "a/b/c/new.txt");
        assert_eq!(
            fs::read_to_string(d.path().join("ws/a/b/c/new.txt")).unwrap(),
            "deep"
        );
    }

    #[test]
    fn atomic_write_replaces_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("existing.txt");
        std::fs::write(&target, b"old").unwrap();

        atomic_write(&target, b"new").unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"new");
    }

    #[test]
    fn atomic_write_cleans_up_temp_file_when_rename_fails() {
        let dir = tempfile::tempdir().unwrap();
        // Target is a directory: temp write succeeds, rename over it fails,
        // and the temp file must not be left behind.
        let target = dir.path().join("adir");
        std::fs::create_dir(&target).unwrap();
        assert!(atomic_write(&target, b"data").is_err());
        assert_eq!(count_tmp_files(dir.path()), 0);
        assert!(target.is_dir(), "original target must be untouched");
    }

    #[test]
    fn atomic_write_propagates_write_failure_without_leaking() {
        let dir = tempfile::tempdir().unwrap();
        // Parent of the target does not exist: fs::write of the temp file
        // fails and the error must propagate (no partial state).
        let target = dir.path().join("missing_sub").join("f.txt");
        assert!(atomic_write(&target, b"data").is_err());
        assert_eq!(count_tmp_files(dir.path()), 0);
    }

    fn count_tmp_files(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .count()
    }
}
