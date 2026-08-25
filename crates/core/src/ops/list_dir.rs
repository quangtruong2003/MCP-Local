use super::*;
use crate::error::{CoreError, ErrorCode};
use crate::pipeline::Audited;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DirEntry {
    pub name: String,
    pub kind: &'static str,
    pub size: u64,
    pub modified: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ListDirResult {
    pub entries: Vec<DirEntry>,
    pub truncated: bool,
}

pub fn list_dir(
    core: &CoreServices,
    root: &Path,
    rel: &str,
    recursive: bool,
) -> Result<Audited<ListDirResult>, CoreError> {
    let (resolved, _) = vet_path(core, root, rel)?;
    if !resolved.is_dir() {
        return Err(CoreError::new(
            ErrorCode::FileNotFound,
            format!("{rel} is not a directory"),
        ));
    }
    let root_c = canonical_root(root)?;
    let mut entries: Vec<DirEntry> = Vec::new();
    let mut truncated = false;

    // Sensitive entries are hidden entirely (default-deny: do not leak names).
    let push = |entries: &mut Vec<DirEntry>, path: &Path, truncated: &mut bool| {
        if entries.len() >= MAX_LIST_ENTRIES {
            *truncated = true;
            return;
        }
        let rel_to_root = match path.strip_prefix(&root_c) {
            Ok(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => return,
        };
        if !core.unrestricted.get() && core.sensitive.is_blocked(&rel_to_root) {
            return;
        }
        let md = std::fs::symlink_metadata(path).ok();
        entries.push(DirEntry {
            name: rel_to_root.to_string_lossy().replace('\\', "/"),
            kind: if path.is_dir() { "dir" } else { "file" },
            size: md.as_ref().map(|m| m.len()).unwrap_or(0),
            modified: md.and_then(|m| m.modified().ok()).map(|t| {
                let t: time::OffsetDateTime = t.into();
                t.format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_default()
            }),
        });
    };

    if recursive {
        for entry in walker(&resolved).build().flatten().skip(1) {
            push(&mut entries, entry.path(), &mut truncated);
            if truncated {
                break;
            }
        }
    } else {
        for entry in std::fs::read_dir(&resolved)
            .map_err(CoreError::io)?
            .flatten()
        {
            push(&mut entries, &entry.path(), &mut truncated);
            if truncated {
                break;
            }
        }
    }
    let entries_count = entries.len();
    Ok(Audited::new(ListDirResult { entries, truncated })
        .with_path(rel)
        .with_detail(serde_json::json!({
            "entries": entries_count,
            "truncated": truncated,
        })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::run_tool_audited;
    use std::fs;

    fn setup() -> (crate::CoreServices, tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::Database::open(&dir.path().join("t.db")).unwrap();
        fs::create_dir_all(dir.path().join("ws/sub")).unwrap();
        fs::write(dir.path().join("ws/a.txt"), "hello").unwrap();
        fs::write(dir.path().join("ws/.env"), "SECRET=1").unwrap();
        let ws = crate::workspace_repo::insert_workspace(
            &db,
            "W",
            &dir.path().join("ws").to_string_lossy(),
        )
        .unwrap();
        let core = crate::CoreServices {
            db,
            pause: Default::default(),
            sensitive: crate::sensitive::SensitiveMatcher::new(),
            unrestricted: crate::pause::Flag::default(),
        };
        (core, dir, ws.id)
    }

    #[test]
    fn lists_entries_without_sensitive_content_leak() {
        let (core, _d, ws) = setup();
        let r = run_tool_audited(&core, "list_directory", &ws, false, |_, root| {
            list_dir(&core, root, ".", false)
        })
        .unwrap();
        let names: Vec<&str> = r.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"sub"));
        // sensitive file is hidden from listings entirely (default-deny)
        assert!(!names.contains(&".env"));
        assert!(!r.truncated);
    }

    #[test]
    fn recursive_lists_nested() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/sub/b.txt"), "x").unwrap();
        let r = run_tool_audited(&core, "list_directory", &ws, false, |_, root| {
            list_dir(&core, root, ".", true)
        })
        .unwrap();
        let names: Vec<&str> = r.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"sub/b.txt"));
    }

    #[test]
    fn listing_a_file_is_file_not_found() {
        let (core, _d, ws) = setup();
        let e = run_tool_audited(&core, "list_directory", &ws, false, |_, root| {
            list_dir(&core, root, "a.txt", false)
        })
        .unwrap_err();
        assert_eq!(e.code, crate::error::ErrorCode::FileNotFound);
    }

    #[test]
    fn missing_dir_is_file_not_found() {
        let (core, _d, ws) = setup();
        let e = run_tool_audited(&core, "list_directory", &ws, false, |_, root| {
            list_dir(&core, root, "nope", false)
        })
        .unwrap_err();
        assert_eq!(e.code, crate::error::ErrorCode::FileNotFound);
    }

    #[test]
    fn rel_outside_workspace_rejected() {
        let (core, _d, ws) = setup();
        let e = run_tool_audited(&core, "list_directory", &ws, false, |_, root| {
            list_dir(&core, root, "..", false)
        })
        .unwrap_err();
        assert_eq!(e.code, crate::error::ErrorCode::PathOutsideWorkspace);
    }
}
