use crate::error::{CoreError, ErrorCode};
use std::path::{Path, PathBuf};

/// Canonicalize, following symlinks/junctions. For a target that does not
/// exist, canonicalize the nearest existing ancestor and re-append the
/// non-existent tail components.
pub fn resolve(target: &Path) -> std::io::Result<PathBuf> {
    if let Ok(p) = dunce::canonicalize(target) {
        return Ok(p);
    }
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    if let Some(name) = target.file_name() {
        suffix.push(name.to_os_string());
    }
    for anc in target.ancestors().skip(1) {
        match dunce::canonicalize(anc) {
            Ok(base) => {
                let mut resolved = base;
                for part in suffix.iter().rev() {
                    resolved.push(part);
                }
                return Ok(resolved);
            }
            Err(_) => {
                if let Some(name) = anc.file_name() {
                    suffix.push(name.to_os_string());
                }
            }
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("no existing ancestor for {}", target.display()),
    ))
}

fn starts_with_components(path: &Path, prefix: &Path) -> bool {
    let mut a = path.components();
    let mut b = prefix.components();
    loop {
        match (b.next(), a.next()) {
            (None, _) => return true,
            (Some(pb), Some(pa)) if pa == pb => continue,
            _ => return false,
        }
    }
}

/// Resolve `target` and enforce that it stays inside canonical `root`.
pub fn ensure_inside(root: &Path, target: &Path) -> Result<PathBuf, CoreError> {
    let resolved = resolve(target).map_err(CoreError::io)?;
    let root_c = dunce::canonicalize(root).map_err(CoreError::io)?;
    if starts_with_components(&resolved, &root_c) {
        Ok(resolved)
    } else {
        Err(CoreError::new(
            ErrorCode::PathOutsideWorkspace,
            format!("{} resolves outside the workspace", target.display()),
        )
        .with_detail(serde_json::json!({ "requested": target.to_string_lossy() })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use std::fs;

    fn setup() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ws");
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("file.txt"), "hello").unwrap();
        (dir, root)
    }

    #[test]
    fn existing_file_inside_resolves() {
        let (_d, root) = setup();
        let p = ensure_inside(&root, &root.join("file.txt")).unwrap();
        assert!(p.ends_with("file.txt"));
    }

    #[test]
    fn traversal_escape_rejected() {
        let (_d, root) = setup();
        let e = ensure_inside(&root, &root.join("..").join("outside.txt")).unwrap_err();
        assert_eq!(e.code, ErrorCode::PathOutsideWorkspace);
    }

    #[test]
    fn absolute_path_outside_rejected() {
        let (_d, root) = setup();
        let e = ensure_inside(
            &root,
            std::path::Path::new(r"C:\Windows\system32\config.sys"),
        )
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::PathOutsideWorkspace);
    }

    #[test]
    fn nonexistent_nested_path_inside_allowed() {
        let (_d, root) = setup();
        let p = ensure_inside(&root, &root.join("sub").join("new").join("f.txt")).unwrap();
        assert!(p.starts_with(dunce::canonicalize(&root).unwrap()));
    }

    #[test]
    fn case_insensitive_windows_match() {
        let (_d, root) = setup();
        let upper = root.to_string_lossy().to_uppercase();
        let p = ensure_inside(
            &root,
            std::path::Path::new(&upper).join("file.txt").as_path(),
        )
        .unwrap();
        assert!(p.ends_with("file.txt"));
    }

    #[test]
    fn symlink_escape_rejected() {
        let (d, root) = setup();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(d.path(), root.join("link")).unwrap();
        }
        #[cfg(windows)]
        {
            // junction: mklink /J via cmd (no admin needed for junctions to dirs)
            let out = std::process::Command::new("cmd")
                .args([
                    "/C",
                    "mklink",
                    "/J",
                    &root.join("link").to_string_lossy(),
                    &d.path().to_string_lossy(),
                ])
                .output()
                .unwrap();
            assert!(out.status.success(), "junction creation failed: {:?}", out);
        }
        let e = ensure_inside(&root, &root.join("link").join("secret.txt")).unwrap_err();
        assert_eq!(e.code, ErrorCode::PathOutsideWorkspace);
    }

    #[test]
    fn root_itself_is_inside() {
        let (_d, root) = setup();
        let p = ensure_inside(&root, &root).unwrap();
        assert_eq!(p, dunce::canonicalize(&root).unwrap());
    }

    #[test]
    fn resolve_keeps_full_tail_for_nonexistent_nested_path() {
        let (_d, root) = setup();
        let target = root.join("sub").join("new").join("f.txt");
        let p = resolve(&target).unwrap();
        assert!(p.ends_with(std::path::Path::new("new").join("f.txt").as_path()));
    }

    #[test]
    fn resolve_single_component_tail_is_preserved() {
        let (_d, root) = setup();
        let target = root.join("notes.md");
        let canonical_root = dunce::canonicalize(&root).unwrap();
        let p = ensure_inside(&root, &target).unwrap();
        assert_eq!(p, canonical_root.join("notes.md"));
    }

    #[test]
    fn sibling_prefix_directory_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        let evil = dir.path().join("ws-evil");
        fs::create_dir_all(ws.join("sub")).unwrap();
        fs::create_dir_all(&evil).unwrap();
        fs::write(evil.join("file.txt"), "secret").unwrap();
        let e = ensure_inside(&ws, &evil.join("file.txt")).unwrap_err();
        assert_eq!(e.code, ErrorCode::PathOutsideWorkspace);
    }

    #[test]
    fn nested_double_traversal_rejected() {
        let (_d, root) = setup();
        let e = ensure_inside(&root, &root.join("sub").join("..").join("..").join("x"))
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::PathOutsideWorkspace);
    }

    #[test]
    fn internal_parent_hop_back_inside_allowed() {
        let (_d, root) = setup();
        // sub/../file.txt normalizes back inside the workspace.
        let p = ensure_inside(&root, &root.join("sub").join("..").join("file.txt")).unwrap();
        assert!(p.ends_with("file.txt"));
    }

    #[test]
    fn exact_canonical_root_is_inside() {
        let (_d, root) = setup();
        let canonical = dunce::canonicalize(&root).unwrap();
        let p = ensure_inside(&root, &canonical).unwrap();
        assert_eq!(p, canonical);
    }
}
