use super::*;
use crate::error::{CoreError, ErrorCode};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize, Clone, schemars::JsonSchema)]
pub struct ReadResult {
    pub content: String,
    pub revision: String,
    pub total_lines: u64,
    pub truncated: bool,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct FileResult {
    pub path: String,
    /// Ok on success; Err carries a JSON string of `{code, message}`.
    pub result: Result<ReadResult, String>,
}

pub fn read_file(
    core: &CoreServices,
    root: &Path,
    rel: &str,
    offset_line: Option<u64>,
    max_lines: Option<u64>,
) -> Result<ReadResult, CoreError> {
    let (resolved, _) = vet_path(core, root, rel)?;
    let meta = std::fs::metadata(&resolved)
        .map_err(|_| CoreError::new(ErrorCode::FileNotFound, format!("{rel} not found")))?;
    if meta.is_dir() {
        return Err(CoreError::new(
            ErrorCode::InvalidArgument,
            format!("{rel} is a directory"),
        ));
    }
    // Fail fast on pathologically large files instead of loading them whole.
    if meta.len() > MAX_INPUT_FILE_BYTES {
        return Err(CoreError::new(
            ErrorCode::ResultTooLarge,
            format!("{rel} exceeds {MAX_INPUT_FILE_BYTES} bytes and cannot be read"),
        ));
    }
    let bytes = std::fs::read(&resolved).map_err(CoreError::io)?;
    let revision = sha256_hex(&bytes);
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let total = lines.len() as u64;

    let start = offset_line.unwrap_or(0) as usize;
    if start >= lines.len() {
        return Ok(ReadResult {
            content: String::new(),
            revision,
            total_lines: total,
            truncated: false,
        });
    }
    let window: Vec<&str> = lines[start..]
        .iter()
        .copied()
        .take(max_lines.unwrap_or(u64::MAX) as usize)
        .collect();

    let mut content = window.concat();
    let mut truncated = start + window.len() < lines.len();
    if content.len() > MAX_READ_BYTES as usize {
        // cut at MAX_READ_BYTES on a char boundary
        let mut cut = MAX_READ_BYTES as usize;
        while cut > 0 && !content.is_char_boundary(cut) {
            cut -= 1;
        }
        content.truncate(cut);
        truncated = true;
    }
    Ok(ReadResult {
        content,
        revision,
        total_lines: total,
        truncated,
    })
}

pub fn read_files(
    core: &CoreServices,
    root: &Path,
    rels: &[String],
) -> Result<Vec<FileResult>, CoreError> {
    if rels.len() > MAX_READ_FILES {
        return Err(CoreError::new(
            ErrorCode::InvalidArgument,
            format!("read_files accepts at most {MAX_READ_FILES} paths per call"),
        ));
    }
    Ok(rels
        .iter()
        .map(|rel| {
            let result = read_file(core, root, rel, None, None).map_err(|e| {
                serde_json::json!({"code": e.code.as_str(), "message": e.message}).to_string()
            });
            FileResult {
                path: rel.clone(),
                result,
            }
        })
        .collect())
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
    fn reads_file_with_revision() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/f.txt"), "line1\nline2\n").unwrap();
        let r = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, "f.txt", None, None)
        })
        .unwrap();
        assert_eq!(r.content, "line1\nline2\n");
        assert_eq!(r.total_lines, 2);
        assert!(!r.truncated);
        // revision is sha256 of full content — stable across reads
        let again = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, "f.txt", None, None)
        })
        .unwrap();
        assert_eq!(r.revision, again.revision);
        assert_eq!(r.revision.len(), 64);
    }

    #[test]
    fn offset_and_max_lines_window() {
        let (core, d, ws) = setup();
        let content = (1..=10)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(d.path().join("ws/f.txt"), content).unwrap();
        let r = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, "f.txt", Some(2), Some(3)) // 0-based offset
        })
        .unwrap();
        assert_eq!(r.content, "line3\nline4\nline5\n");
        assert!(r.truncated);
        assert_eq!(r.total_lines, 10);
    }

    #[test]
    fn big_file_truncated_at_byte_bound() {
        let (core, d, ws) = setup();
        let big = "x".repeat(300_000);
        fs::write(d.path().join("ws/big.txt"), &big).unwrap();
        let r = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, "big.txt", None, None)
        })
        .unwrap();
        assert!(r.content.len() <= 262_144);
        assert!(r.truncated);
    }

    #[test]
    fn sensitive_file_blocked() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/.env"), "SECRET").unwrap();
        let e = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, ".env", None, None)
        })
        .unwrap_err();
        assert_eq!(e.code, crate::error::ErrorCode::SensitiveFileBlocked);
    }

    #[test]
    fn missing_file_is_file_not_found() {
        let (core, _d, ws) = setup();
        let e = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, "ghost.txt", None, None)
        })
        .unwrap_err();
        assert_eq!(e.code, crate::error::ErrorCode::FileNotFound);
    }

    #[test]
    fn crlf_line_endings_counted_correctly() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/win.txt"), "one\r\ntwo\r\nthree\r\n").unwrap();
        let r = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, "win.txt", None, None)
        })
        .unwrap();
        assert_eq!(r.total_lines, 3);
        // split_inclusive keeps \r intact; content round-trips byte-for-byte.
        assert_eq!(r.content, "one\r\ntwo\r\nthree\r\n");
    }

    #[test]
    fn oversized_file_rejected_early_with_result_too_large() {
        let (core, d, ws) = setup();
        let big = "x".repeat(crate::ops::MAX_INPUT_FILE_BYTES as usize + 1);
        fs::write(d.path().join("ws/huge.txt"), &big).unwrap();
        let e = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, "huge.txt", None, None)
        })
        .unwrap_err();
        assert_eq!(e.code, crate::error::ErrorCode::ResultTooLarge);
    }

    #[test]
    fn reading_a_directory_is_invalid_argument() {
        let (core, d, ws) = setup();
        fs::create_dir(d.path().join("ws/adir")).unwrap();
        let e = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, "adir", None, None)
        })
        .unwrap_err();
        assert_eq!(e.code, crate::error::ErrorCode::InvalidArgument);
    }

    #[test]
    fn offset_past_end_returns_empty_not_error() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/f.txt"), "only\n").unwrap();
        let r = run_tool_audited(&core, "read_file", &ws, false, |_, root| {
            read_file(&core, root, "f.txt", Some(50), None)
        })
        .unwrap();
        assert_eq!(r.content, "");
        assert!(!r.truncated);
        assert_eq!(r.total_lines, 1);
    }

    #[test]
    fn read_files_caps_at_8() {
        let (core, _d, ws) = setup();
        let paths: Vec<String> = (0..9).map(|i| format!("f{i}.txt")).collect();
        let e = run_tool_audited(&core, "read_files", &ws, false, |_, root| {
            read_files(&core, root, &paths)
        })
        .unwrap_err();
        // too many paths is malformed client input -> INVALID_ARGUMENT
        assert_eq!(e.code, crate::error::ErrorCode::InvalidArgument);
    }

    #[test]
    fn read_files_per_file_results() {
        let (core, d, ws) = setup();
        fs::write(d.path().join("ws/ok.txt"), "fine").unwrap();
        let r = run_tool_audited(&core, "read_files", &ws, false, |_, root| {
            read_files(
                &core,
                root,
                &["ok.txt".into(), ".env".into(), "ghost.txt".into()],
            )
        })
        .unwrap();
        assert!(r[0].result.is_ok());
        let err1 = r[1].result.as_ref().unwrap_err();
        assert!(err1.contains("SENSITIVE_FILE_BLOCKED"));
        let err2 = r[2].result.as_ref().unwrap_err();
        assert!(err2.contains("FILE_NOT_FOUND"));
    }
}
