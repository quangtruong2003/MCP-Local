use super::*;
use crate::error::{CoreError, ErrorCode};
use crate::pipeline::Audited;
use globset::{GlobBuilder, GlobSetBuilder};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct FindResult {
    pub paths: Vec<String>,
    pub truncated: bool,
}

pub fn find_files(
    core: &CoreServices,
    root: &Path,
    pattern: &str,
) -> Result<Audited<FindResult>, CoreError> {
    let glob = GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .map_err(|e| CoreError::new(ErrorCode::InvalidArgument, format!("bad glob: {e}")))?;
    let set = GlobSetBuilder::new()
        .add(glob)
        .build()
        .map_err(|e| CoreError::new(ErrorCode::InvalidArgument, e.to_string()))?;
    let root_c = canonical_root(root)?;

    let mut paths = Vec::new();
    let mut truncated = false;
    for entry in walker(root).build().flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let rel = match entry.path().strip_prefix(&root_c) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if core.sensitive.is_blocked(rel) {
            continue;
        }
        if set.is_match(rel) {
            if paths.len() >= MAX_FIND_PATHS {
                truncated = true;
                break;
            }
            paths.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    // Bounded by MAX_FIND_PATHS; paths only — never file contents.
    let audited_paths = serde_json::to_value(&paths).unwrap_or_default();
    Ok(
        Audited::new(FindResult { paths, truncated }).with_detail(serde_json::json!({
            "paths": audited_paths,
            "truncated": truncated,
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::run_tool_audited;
    use std::fs;

    fn setup() -> (crate::CoreServices, tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::Database::open(&dir.path().join("t.db")).unwrap();
        fs::create_dir_all(dir.path().join("ws/src")).unwrap();
        fs::write(dir.path().join("ws/src/main.rs"), "").unwrap();
        fs::write(dir.path().join("ws/README.md"), "").unwrap();
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
        };
        (core, dir, ws.id)
    }

    #[test]
    fn glob_finds_rs_files() {
        let (core, _d, ws) = setup();
        let r = run_tool_audited(&core, "find_files", &ws, false, |_, root| {
            find_files(&core, root, "**/*.rs")
        })
        .unwrap();
        assert_eq!(r.paths, vec!["src/main.rs".to_string()]);
    }

    #[test]
    fn node_modules_excluded() {
        let (core, d, ws) = setup();
        fs::create_dir_all(d.path().join("ws/node_modules/leftpad")).unwrap();
        fs::write(d.path().join("ws/node_modules/leftpad/index.js"), "").unwrap();
        let r = run_tool_audited(&core, "find_files", &ws, false, |_, root| {
            find_files(&core, root, "**/*.js")
        })
        .unwrap();
        assert!(r.paths.is_empty());
    }

    #[test]
    fn bad_glob_is_invalid_argument() {
        let (core, _d, ws) = setup();
        let e = run_tool_audited(&core, "find_files", &ws, false, |_, root| {
            find_files(&core, root, "[unclosed")
        })
        .unwrap_err();
        assert_eq!(e.code, crate::error::ErrorCode::InvalidArgument);
    }
}
