use std::{
    fs,
    path::{Path, PathBuf},
};

use lc_core::{
    confinement,
    error::{CoreError, ErrorCode},
    ops::sha256_hex,
    workspace_repo, CoreServices,
};
use serde::Serialize;

use super::mcp::BrowserUploadSource;

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BrowserDownloadInfo {
    pub download_id: String,
    pub filename: String,
    pub bytes: u64,
    pub completed: bool,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct BrowserSavedDownloadOutput {
    pub workspace_id: String,
    pub path: String,
    pub bytes: usize,
    pub sha256: String,
}

/// Files Local Workspaces itself places in the session artifact root. These are
/// never downloads and must not be staged into `downloads/` or exposed as such.
const INTERNAL_ROOT_FILES: &[&str] = &[];
const COMPLETED_MARKER: &str = ".completed.json";
const INCOMPLETE_SUFFIX: &str = ".part";

fn denied(code: ErrorCode, message: &str) -> CoreError {
    CoreError::new(code, message)
}

fn require_workspace(
    core: &CoreServices,
    workspace_id: &str,
    write: bool,
) -> Result<lc_core::models::Workspace, CoreError> {
    let ws = workspace_repo::get_workspace(&core.db, workspace_id)?;
    if !core.unrestricted.get() {
        if write && !ws.can_write {
            return Err(denied(
                ErrorCode::BrowserDownloadDenied,
                "workspace Write permission is required for browser download export",
            ));
        }
        if !write && !ws.can_read {
            return Err(denied(
                ErrorCode::BrowserUploadDenied,
                "workspace Read permission is required for browser upload",
            ));
        }
    }
    Ok(ws)
}

fn safe_workspace_path(
    core: &CoreServices,
    root: &Path,
    rel: &str,
    error_code: ErrorCode,
) -> Result<PathBuf, CoreError> {
    if Path::new(rel).is_absolute() {
        return Err(denied(
            error_code,
            "browser file paths must be workspace-relative",
        ));
    }
    lc_core::ops::vet_path(core, root, rel)
        .map(|(path, _)| path)
        .map_err(|_| {
            denied(
                error_code,
                "browser file path is not allowed in the approved workspace",
            )
        })
}

/// Reduce a file extension to a short, ASCII-alphanumeric suffix. The original
/// workspace filename is never forwarded upstream; only an opaque id plus, when
/// useful for MIME detection, a sanitized extension. Cleaning stops at the first
/// unsafe character so unsafe characters are never silently spliced out.
fn safe_extension(path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| {
            value
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .take(16)
                .collect::<String>()
        })
        .unwrap_or_default();
    if ext.is_empty() {
        String::new()
    } else {
        format!(".{ext}")
    }
}

/// Completed downloads may land in the session outputDir root rather than in
/// `downloads/`. Stage any such root-level files into `downloads/` so discovery
/// and export have one well-defined, sandbox-confined location. The move is
/// atomic (same-volume rename); the copy fallback writes a `.part` file and
/// renames it so a reader never observes a partially staged download.
pub fn stage_completed_downloads(artifact_dir: &Path) -> Result<(), CoreError> {
    let downloads = artifact_dir.join("downloads");
    fs::create_dir_all(&downloads).map_err(CoreError::io)?;
    for entry in fs::read_dir(artifact_dir).map_err(CoreError::io)? {
        let entry = entry.map_err(CoreError::io)?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.')
            || name == COMPLETED_MARKER
            || INTERNAL_ROOT_FILES.contains(&name.as_str())
        {
            continue;
        }
        let dest = downloads.join(&name);
        if dest.exists() {
            continue;
        }
        if fs::rename(&path, &dest).is_err() {
            let part = downloads.join(format!("{name}{INCOMPLETE_SUFFIX}"));
            fs::copy(&path, &part).map_err(CoreError::io)?;
            fs::rename(&part, &dest).map_err(CoreError::io)?;
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
}

fn download_id(filename: &str, bytes: &[u8]) -> String {
    let digest = sha256_hex(bytes);
    format!("d-{}-{}", &digest[..16], filename.len())
}

fn is_incomplete_name(name: &str) -> bool {
    name.ends_with(INCOMPLETE_SUFFIX)
}

pub fn list_downloads(artifact_dir: &Path) -> Result<Vec<BrowserDownloadInfo>, CoreError> {
    stage_completed_downloads(artifact_dir)?;
    let dir = artifact_dir.join("downloads");
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir).map_err(CoreError::io)? {
        let entry = entry.map_err(CoreError::io)?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let raw_name = entry.file_name().to_string_lossy().to_string();
        let incomplete = is_incomplete_name(&raw_name);
        let filename = if incomplete {
            raw_name.trim_end_matches(INCOMPLETE_SUFFIX).to_string()
        } else {
            raw_name
        };
        let bytes = fs::read(&path).map_err(CoreError::io)?;
        out.push(BrowserDownloadInfo {
            download_id: download_id(&filename, &bytes),
            filename,
            bytes: bytes.len() as u64,
            completed: !incomplete,
            // Hash only for complete downloads; an in-flight/partial artifact is
            // listed without a digest so callers cannot pin a moving target.
            sha256: if incomplete {
                None
            } else {
                Some(sha256_hex(&bytes))
            },
        });
    }
    out.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(out)
}

/// Resolve an opaque upload artifact id to a file confined to the active session
/// artifact sandbox. Currently resolves completed downloads.
fn resolve_artifact_source(artifact_dir: &Path, artifact_id: &str) -> Result<PathBuf, CoreError> {
    stage_completed_downloads(artifact_dir)?;
    let downloads = artifact_dir.join("downloads");
    for entry in fs::read_dir(&downloads).map_err(CoreError::io)? {
        let entry = entry.map_err(CoreError::io)?;
        let path = entry.path();
        if !path.is_file() || is_incomplete_name(&entry.file_name().to_string_lossy()) {
            continue;
        }
        let bytes = fs::read(&path).map_err(CoreError::io)?;
        let filename = entry.file_name().to_string_lossy().to_string();
        if download_id(&filename, &bytes) == artifact_id {
            return confinement::ensure_inside(artifact_dir, &path).map_err(|_| {
                denied(
                    ErrorCode::BrowserUploadDenied,
                    "browser artifact id escaped the active session sandbox",
                )
            });
        }
    }
    Err(denied(
        ErrorCode::BrowserUploadDenied,
        "browser artifact id was not found in the active session",
    ))
}

/// Stage upload sources into the active session sandbox and return the staged
/// absolute paths that are safe to hand to the browser runtime. Workspace
/// sources require Read permission and canonical confinement; artifact sources
/// must resolve inside the active session artifact root. The original workspace
/// filename is never forwarded.
pub fn stage_uploads(
    core: &CoreServices,
    artifact_dir: &Path,
    workspace_id: Option<&str>,
    sources: &[BrowserUploadSource],
) -> Result<Vec<PathBuf>, CoreError> {
    if sources.is_empty() || sources.len() > 16 {
        return Err(CoreError::new(
            ErrorCode::InvalidArgument,
            "browser_upload requires 1 to 16 sources",
        ));
    }
    let staging = artifact_dir.join("upload-staging");
    fs::create_dir_all(&staging).map_err(CoreError::io)?;
    let mut workspace_root: Option<PathBuf> = None;
    let mut out = Vec::with_capacity(sources.len());
    for source in sources {
        let source_path = match source {
            BrowserUploadSource::Workspace { path } => {
                let root = match &workspace_root {
                    Some(root) => root.clone(),
                    None => {
                        let id = workspace_id.ok_or_else(|| {
                            CoreError::new(
                                ErrorCode::InvalidArgument,
                                "browser_upload workspace sources require workspace_id",
                            )
                        })?;
                        let ws = require_workspace(core, id, false)?;
                        workspace_root = Some(ws.root_path.clone());
                        ws.root_path.clone()
                    }
                };
                let candidate =
                    safe_workspace_path(core, &root, path, ErrorCode::BrowserUploadDenied)?;
                if !candidate.is_file() {
                    return Err(denied(
                        ErrorCode::BrowserUploadDenied,
                        "browser upload source must be an existing file",
                    ));
                }
                candidate
            }
            BrowserUploadSource::Artifact { artifact_id } => {
                resolve_artifact_source(artifact_dir, artifact_id)?
            }
        };
        let extension = safe_extension(&source_path);
        let staged = staging.join(format!("u-{:032x}{extension}", rand::random::<u128>()));
        fs::copy(&source_path, &staged).map_err(CoreError::io)?;
        let staged = confinement::ensure_inside(artifact_dir, &staged).map_err(|_| {
            denied(
                ErrorCode::BrowserUploadDenied,
                "staged browser upload escaped the active session sandbox",
            )
        })?;
        out.push(staged);
    }
    Ok(out)
}

pub fn save_download_to_workspace(
    core: &CoreServices,
    artifact_dir: &Path,
    workspace_id: &str,
    id: &str,
    destination: &str,
) -> Result<BrowserSavedDownloadOutput, CoreError> {
    let ws = require_workspace(core, workspace_id, true)?;
    stage_completed_downloads(artifact_dir)?;
    let downloads = artifact_dir.join("downloads");
    let mut found: Option<Vec<u8>> = None;
    for entry in fs::read_dir(&downloads).map_err(CoreError::io)? {
        let entry = entry.map_err(CoreError::io)?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let raw_name = entry.file_name().to_string_lossy().to_string();
        let incomplete = is_incomplete_name(&raw_name);
        let filename = if incomplete {
            raw_name.trim_end_matches(INCOMPLETE_SUFFIX).to_string()
        } else {
            raw_name
        };
        let bytes = fs::read(&path).map_err(CoreError::io)?;
        if download_id(&filename, &bytes) == id {
            if incomplete {
                return Err(denied(
                    ErrorCode::BrowserDownloadDenied,
                    "browser download is not complete yet",
                ));
            }
            found = Some(bytes);
            break;
        }
    }
    let bytes = found.ok_or_else(|| {
        CoreError::new(
            ErrorCode::BrowserDownloadNotFound,
            "browser download id was not found in the active session",
        )
    })?;
    let target = safe_workspace_path(
        core,
        &ws.root_path,
        destination,
        ErrorCode::BrowserDownloadDenied,
    )?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(CoreError::io)?;
    }
    lc_core::ops::write_file::atomic_write(&target, &bytes).map_err(CoreError::io)?;
    Ok(BrowserSavedDownloadOutput {
        workspace_id: workspace_id.to_string(),
        path: destination.to_string(),
        bytes: bytes.len(),
        sha256: sha256_hex(&bytes),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_opaque_and_stable() {
        let a = download_id("x.txt", b"hello");
        assert!(a.starts_with("d-"));
        assert_eq!(a, download_id("x.txt", b"hello"));
        assert_ne!(a, download_id("x.txt", b"world"));
    }

    #[test]
    fn safe_extension_strips_unsafe_characters() {
        assert_eq!(safe_extension(Path::new("a/b.PNG")), ".PNG");
        assert_eq!(safe_extension(Path::new("a/b.tar.gz")), ".gz");
        assert_eq!(safe_extension(Path::new("a/b")), "");
        assert_eq!(safe_extension(Path::new("a/evil.ex e;txt")), ".ex");
    }

    #[test]
    fn staging_moves_root_downloads_into_downloads_dir() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path();
        fs::create_dir_all(artifact.join("downloads")).unwrap();
        // Completed downloads may land in the outputDir root.
        fs::write(artifact.join("report.pdf"), b"pdf-bytes").unwrap();
        // Internal/hidden files must never be treated as downloads.
        fs::write(artifact.join(".completed.json"), b"{}").unwrap();

        stage_completed_downloads(artifact).unwrap();

        assert!(artifact.join("downloads/report.pdf").exists());
        assert!(!artifact.join("downloads/.completed.json").exists());

        let list = list_downloads(artifact).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].filename, "report.pdf");
        assert!(list[0].completed);
        assert!(list[0].sha256.is_some());
    }

    #[test]
    fn incomplete_downloads_are_listed_without_hash() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path();
        let downloads = artifact.join("downloads");
        fs::create_dir_all(&downloads).unwrap();
        fs::write(downloads.join("done.txt"), b"complete").unwrap();
        fs::write(downloads.join("busy.txt.part"), b"partial").unwrap();

        let list = list_downloads(artifact).unwrap();
        let done = list.iter().find(|d| d.filename == "done.txt").unwrap();
        let busy = list.iter().find(|d| d.filename == "busy.txt").unwrap();
        assert!(done.completed);
        assert!(done.sha256.is_some());
        assert!(!busy.completed);
        assert!(busy.sha256.is_none());
    }
}
