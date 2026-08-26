use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use lc_core::error::{CoreError, ErrorCode};
use serde::{Deserialize, Serialize};

use super::types::{
    MAX_ARTIFACT_AGE_DAYS, MAX_ARTIFACT_TOTAL_BYTES, MAX_COMPLETED_ARTIFACT_SESSIONS,
};

const COMPLETED_MARKER: &str = ".completed.json";

#[derive(Debug, Clone)]
pub struct ArtifactStore {
    root: PathBuf,
    max_completed_sessions: usize,
    max_age: Duration,
    max_total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompletionMarker {
    completed_at_unix_secs: u64,
}

#[derive(Debug)]
struct CompletedSession {
    path: PathBuf,
    completed_at: SystemTime,
    bytes: u64,
}

impl ArtifactStore {
    pub fn new(root: PathBuf) -> Result<Self, CoreError> {
        Self::with_limits(
            root,
            MAX_COMPLETED_ARTIFACT_SESSIONS,
            Duration::from_secs(MAX_ARTIFACT_AGE_DAYS * 24 * 60 * 60),
            MAX_ARTIFACT_TOTAL_BYTES,
        )
    }

    pub fn with_limits(
        root: PathBuf,
        max_completed_sessions: usize,
        max_age: Duration,
        max_total_bytes: u64,
    ) -> Result<Self, CoreError> {
        std::fs::create_dir_all(&root).map_err(browser_artifact_io)?;
        let root = dunce::canonicalize(&root).map_err(browser_artifact_io)?;
        Ok(Self {
            root,
            max_completed_sessions,
            max_age,
            max_total_bytes,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn create_session(&self, session_id: &str) -> Result<PathBuf, CoreError> {
        validate_session_id(session_id)?;
        let path = self.root.join(session_id);
        ensure_direct_child(&self.root, &path)?;
        std::fs::create_dir_all(path.join("downloads")).map_err(browser_artifact_io)?;
        std::fs::create_dir_all(path.join("traces")).map_err(browser_artifact_io)?;
        std::fs::create_dir_all(path.join("screenshots")).map_err(browser_artifact_io)?;
        std::fs::create_dir_all(path.join("diagnostics")).map_err(browser_artifact_io)?;
        Ok(path)
    }

    pub fn mark_completed(&self, session_id: &str, when: SystemTime) -> Result<(), CoreError> {
        validate_session_id(session_id)?;
        let session = self.root.join(session_id);
        ensure_direct_child(&self.root, &session)?;
        if !session.is_dir() {
            return Err(CoreError::new(
                ErrorCode::BrowserArtifactFailed,
                "browser artifact session does not exist",
            ));
        }
        let secs = when
            .duration_since(UNIX_EPOCH)
            .map_err(|_| {
                CoreError::new(
                    ErrorCode::BrowserArtifactFailed,
                    "browser artifact completion timestamp is before Unix epoch",
                )
            })?
            .as_secs();
        let marker = CompletionMarker {
            completed_at_unix_secs: secs,
        };
        let json = serde_json::to_vec(&marker).map_err(|e| {
            CoreError::new(
                ErrorCode::BrowserArtifactFailed,
                format!("failed to serialize browser artifact marker: {e}"),
            )
        })?;
        std::fs::write(session.join(COMPLETED_MARKER), json).map_err(browser_artifact_io)
    }

    pub fn cleanup(&self, now: SystemTime) -> Result<(), CoreError> {
        let mut completed = self.completed_sessions()?;
        completed.sort_by_key(|entry| entry.completed_at);

        for entry in &completed {
            if now
                .duration_since(entry.completed_at)
                .is_ok_and(|age| age > self.max_age)
            {
                remove_session(&entry.path)?;
            }
        }

        completed = self.completed_sessions()?;
        completed.sort_by_key(|entry| entry.completed_at);
        while completed.len() > self.max_completed_sessions {
            let oldest = completed.remove(0);
            remove_session(&oldest.path)?;
        }

        completed = self.completed_sessions()?;
        completed.sort_by_key(|entry| entry.completed_at);
        let mut total = completed.iter().map(|entry| entry.bytes).sum::<u64>();
        while total > self.max_total_bytes && !completed.is_empty() {
            let oldest = completed.remove(0);
            total = total.saturating_sub(oldest.bytes);
            remove_session(&oldest.path)?;
        }
        Ok(())
    }

    fn completed_sessions(&self) -> Result<Vec<CompletedSession>, CoreError> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&self.root).map_err(browser_artifact_io)? {
            let entry = entry.map_err(browser_artifact_io)?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            ensure_direct_child(&self.root, &path)?;
            let marker_path = path.join(COMPLETED_MARKER);
            let marker_bytes = match std::fs::read(&marker_path) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(browser_artifact_io(e)),
            };
            let marker: CompletionMarker = serde_json::from_slice(&marker_bytes).map_err(|e| {
                CoreError::new(
                    ErrorCode::BrowserArtifactFailed,
                    format!("invalid browser artifact marker: {e}"),
                )
            })?;
            out.push(CompletedSession {
                path: path.clone(),
                completed_at: UNIX_EPOCH + Duration::from_secs(marker.completed_at_unix_secs),
                bytes: dir_payload_size(&path)?,
            });
        }
        Ok(out)
    }
}

fn validate_session_id(session_id: &str) -> Result<(), CoreError> {
    if session_id.is_empty()
        || !session_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(CoreError::new(
            ErrorCode::BrowserArtifactFailed,
            "invalid browser artifact session id",
        ));
    }
    Ok(())
}

fn ensure_direct_child(root: &Path, path: &Path) -> Result<(), CoreError> {
    if path.parent() == Some(root) {
        Ok(())
    } else {
        Err(CoreError::new(
            ErrorCode::BrowserArtifactFailed,
            "browser artifact path escaped its sandbox",
        ))
    }
}

fn dir_payload_size(path: &Path) -> Result<u64, CoreError> {
    let mut total = 0u64;
    for entry in std::fs::read_dir(path).map_err(browser_artifact_io)? {
        let entry = entry.map_err(browser_artifact_io)?;
        let child = entry.path();
        if child
            .file_name()
            .is_some_and(|name| name == COMPLETED_MARKER)
        {
            continue;
        }
        let metadata = entry.metadata().map_err(browser_artifact_io)?;
        if metadata.is_dir() {
            total = total.saturating_add(dir_payload_size(&child)?);
        } else if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        }
    }
    Ok(total)
}

fn remove_session(path: &Path) -> Result<(), CoreError> {
    std::fs::remove_dir_all(path).map_err(browser_artifact_io)
}

fn browser_artifact_io(error: std::io::Error) -> CoreError {
    CoreError::new(
        ErrorCode::BrowserArtifactFailed,
        format!("browser artifact operation failed: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn write_bytes(path: &std::path::Path, bytes: usize) {
        std::fs::write(path, vec![0u8; bytes]).unwrap();
    }

    #[test]
    fn cleanup_prunes_old_completed_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(dir.path().join("artifacts")).unwrap();
        let now = SystemTime::now();
        let old = store.create_session("old").unwrap();
        write_bytes(&old.join("payload.bin"), 16);
        store
            .mark_completed("old", now - Duration::from_secs(8 * 24 * 60 * 60))
            .unwrap();
        let fresh = store.create_session("fresh").unwrap();
        write_bytes(&fresh.join("payload.bin"), 16);
        store.mark_completed("fresh", now).unwrap();

        store.cleanup(now).unwrap();
        assert!(!old.exists());
        assert!(fresh.exists());
    }

    #[test]
    fn cleanup_keeps_at_most_ten_completed_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(dir.path().join("artifacts")).unwrap();
        let now = SystemTime::now();
        for i in 0..12u64 {
            let id = format!("s{i}");
            let path = store.create_session(&id).unwrap();
            write_bytes(&path.join("payload.bin"), 1);
            store
                .mark_completed(&id, now - Duration::from_secs((12 - i) * 60))
                .unwrap();
        }
        store.cleanup(now).unwrap();
        let retained = std::fs::read_dir(store.root()).unwrap().count();
        assert_eq!(retained, 10);
    }

    #[test]
    fn cleanup_prunes_oldest_sessions_to_size_budget() {
        let dir = tempfile::tempdir().unwrap();
        let store = ArtifactStore::with_limits(
            dir.path().join("artifacts"),
            10,
            Duration::from_secs(7 * 24 * 60 * 60),
            25,
        )
        .unwrap();
        let now = SystemTime::now();
        for (id, age, bytes) in [("a", 3, 10usize), ("b", 2, 10), ("c", 1, 10)] {
            let path = store.create_session(id).unwrap();
            write_bytes(&path.join("payload.bin"), bytes);
            store
                .mark_completed(id, now - Duration::from_secs(age * 60))
                .unwrap();
        }
        store.cleanup(now).unwrap();
        assert!(!store.root().join("a").exists());
        assert!(store.root().join("b").exists());
        assert!(store.root().join("c").exists());
    }

    #[test]
    fn session_paths_cannot_escape_artifact_root() {
        let dir = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(dir.path().join("artifacts")).unwrap();
        assert!(store.create_session("../escape").is_err());
        assert!(!dir.path().join("escape").exists());
    }
}
