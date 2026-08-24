use super::*;
use crate::error::{CoreError, ErrorCode};
use serde::Serialize;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct TextMatch {
    pub path: String,
    pub line_no: u64,
    pub line: String,
    pub col: u64,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SearchResult {
    pub matches: Vec<TextMatch>,
    pub truncated: bool,
}

pub fn search_text(
    core: &CoreServices,
    root: &Path,
    query: &str,
    path_prefix: Option<&str>,
    case_sensitive: bool,
    is_regex: bool,
    max_results: Option<usize>,
) -> Result<SearchResult, CoreError> {
    if query.is_empty() {
        return Err(CoreError::new(
            ErrorCode::InvalidArgument,
            "query must not be empty",
        ));
    }
    let cap = max_results
        .unwrap_or(MAX_SEARCH_MATCHES)
        .min(MAX_SEARCH_MATCHES);
    let matcher = {
        let pat = if is_regex {
            query.to_string()
        } else {
            regex::escape(query)
        };
        regex::RegexBuilder::new(&pat)
            .case_insensitive(!case_sensitive)
            .size_limit(10 * (1 << 20))
            .build()
            .map_err(|e| CoreError::new(ErrorCode::InvalidArgument, format!("bad query: {e}")))?
    };
    let root_c = canonical_root(root)?;
    let prefix_filter = path_prefix.map(|p| root_c.join(p));

    let mut matches = Vec::new();
    let mut truncated = false;
    'outer: for entry in walker(root).build().flatten() {
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
        if let Some(pf) = &prefix_filter {
            if !entry.path().starts_with(pf) {
                continue;
            }
        }
        let file = match std::fs::File::open(entry.path()) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        for (i, line) in BufReader::new(file).lines().enumerate() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break, // IO problem: skip rest of file
            };
            if let Some(m) = matcher.find(&line) {
                if matches.len() >= cap {
                    truncated = true;
                    break 'outer;
                }
                matches.push(TextMatch {
                    path: rel_str.clone(),
                    line_no: (i + 1) as u64,
                    line: line.chars().take(500).collect(),
                    col: line[..m.start()].chars().count() as u64 + 1,
                });
            }
        }
    }
    Ok(SearchResult { matches, truncated })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::run_tool_audited;
    use std::fs;

    fn setup() -> (crate::CoreServices, tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::Database::open(&dir.path().join("t.db")).unwrap();
        fs::create_dir_all(dir.path().join("ws")).unwrap();
        fs::write(
            dir.path().join("ws/code.rs"),
            "fn main() {}\nlet x = SECRET;\n",
        )
        .unwrap();
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
    fn literal_search_finds_line_and_col() {
        let (core, _d, ws) = setup();
        let r = run_tool_audited(&core, "search_text", &ws, false, |_, root| {
            search_text(&core, root, "SECRET", None, true, false, None)
        })
        .unwrap();
        assert_eq!(r.matches.len(), 1);
        assert_eq!(r.matches[0].path, "code.rs");
        assert_eq!(r.matches[0].line_no, 2);
        assert_eq!(r.matches[0].col, 9);
    }

    #[test]
    fn case_insensitive_when_requested() {
        let (core, _d, ws) = setup();
        let r = run_tool_audited(&core, "search_text", &ws, false, |_, root| {
            search_text(&core, root, "secret", None, false, false, None)
        })
        .unwrap();
        assert_eq!(r.matches.len(), 1);
    }

    #[test]
    fn regex_mode() {
        let (core, _d, ws) = setup();
        let r = run_tool_audited(&core, "search_text", &ws, false, |_, root| {
            search_text(&core, root, r"let \w+ =", None, true, true, None)
        })
        .unwrap();
        assert_eq!(r.matches.len(), 1);
    }

    #[test]
    fn invalid_regex_is_invalid_argument() {
        let (core, _d, ws) = setup();
        let e = run_tool_audited(&core, "search_text", &ws, false, |_, root| {
            search_text(&core, root, "(unclosed", None, true, true, None)
        })
        .unwrap_err();
        assert_eq!(e.code, crate::error::ErrorCode::InvalidArgument);
    }

    #[test]
    fn sensitive_files_never_searched() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/.env"), "SECRET=1").unwrap();
        let r = run_tool_audited(&core, "search_text", &ws, false, |_, root| {
            search_text(&core, root, "SECRET", None, true, false, None)
        })
        .unwrap();
        // only code.rs matches; .env is skipped
        assert_eq!(r.matches.len(), 1);
        assert_eq!(r.matches[0].path, "code.rs");
    }

    #[test]
    fn max_results_caps_with_truncation_flag() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/many.txt"), "hit\nhit\nhit\n").unwrap();
        let r = run_tool_audited(&core, "search_text", &ws, false, |_, root| {
            search_text(&core, root, "hit", None, true, false, Some(2))
        })
        .unwrap();
        assert_eq!(r.matches.len(), 2);
        assert!(r.truncated);
    }

    #[test]
    fn max_results_below_match_count_reports_truncated() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/a.txt"), "x hit x\ny hit y\nz hit z\n").unwrap();
        fs::write(d.path().join("ws/b.txt"), "hit here too\n").unwrap();
        let r = run_tool_audited(&core, "search_text", &ws, false, |_, root| {
            search_text(&core, root, "hit", None, true, false, Some(2))
        })
        .unwrap();
        assert_eq!(r.matches.len(), 2);
        assert!(r.truncated); // at least one more real match existed
    }

    #[test]
    fn empty_query_is_invalid_argument() {
        let (core, _d, ws) = setup();
        let e = run_tool_audited(&core, "search_text", &ws, false, |_, root| {
            search_text(&core, root, "", None, true, false, None)
        })
        .unwrap_err();
        assert_eq!(e.code, crate::error::ErrorCode::InvalidArgument);
    }

    #[test]
    fn path_prefix_narrows_search() {
        let (core, d, ws) = setup();
        fs::create_dir_all(d.path().join("ws/sub")).unwrap();
        fs::write(d.path().join("ws/sub/in.txt"), "needle\n").unwrap();
        fs::write(d.path().join("ws/out.txt"), "needle\n").unwrap();
        let r = run_tool_audited(&core, "search_text", &ws, false, |_, root| {
            search_text(&core, root, "needle", Some("sub"), true, false, None)
        })
        .unwrap();
        assert_eq!(r.matches.len(), 1);
        assert_eq!(r.matches[0].path, "sub/in.txt");
    }
}
