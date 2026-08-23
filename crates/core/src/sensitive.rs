use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use std::path::Path;

/// Glob patterns (matched against workspace-relative paths) for files that
/// must never be read or exposed. Hard-coded by design: overriding the
/// deny-list requires a code change, not configuration.
pub const FILE_PATTERNS: &[&str] = &[
    "**/.env",
    "**/.env.*",
    "**/*.pem",
    "**/*.key",
    "**/id_rsa*",
    "**/.git/config",
    "**/credentials",
    "**/credentials.*",
    "**/*.pfx",
    "**/*.p12",
    "**/secrets.*",
];

/// Directory names that are blocked entirely at any depth.
pub const BLOCKED_DIR_NAMES: &[&str] = &["node_modules"];

pub struct SensitiveMatcher {
    files: GlobSet,
    dirs: GlobSet,
}

impl SensitiveMatcher {
    pub fn new() -> Self {
        let mut fb = GlobSetBuilder::new();
        for p in FILE_PATTERNS {
            // Case-insensitive: Windows/macOS filesystems are case-insensitive.
            fb.add(GlobBuilder::new(p).case_insensitive(true).build().unwrap());
        }
        let mut db = GlobSetBuilder::new();
        for name in BLOCKED_DIR_NAMES {
            db.add(
                GlobBuilder::new(&format!("**/{name}"))
                    .case_insensitive(true)
                    .build()
                    .unwrap(),
            );
        }
        Self {
            files: fb.build().unwrap(),
            dirs: db.build().unwrap(),
        }
    }

    /// `rel` must be relative to the workspace root. Checks the full relative
    /// path AND every ancestor directory (so `node_modules/x/y` is caught by
    /// the `node_modules` dir rule). Separators are normalized to `/` before
    /// matching so Windows-style `\` paths match the same as `/` paths on any
    /// platform.
    pub fn is_blocked(&self, rel: &Path) -> bool {
        for anc in rel.ancestors() {
            if anc.as_os_str().is_empty() {
                break;
            }
            let normalized = anc.to_string_lossy().replace('\\', "/");
            if self.files.is_match(&normalized) || self.dirs.is_match(&normalized) {
                return true;
            }
        }
        false
    }
}

impl Default for SensitiveMatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn m() -> SensitiveMatcher {
        SensitiveMatcher::new()
    }

    #[test]
    fn env_files_blocked_at_any_depth() {
        assert!(m().is_blocked(Path::new(".env")));
        assert!(m().is_blocked(Path::new(".env.local")));
        assert!(m().is_blocked(Path::new("sub/dir/.env")));
    }

    #[test]
    fn lookalikes_allowed() {
        assert!(!m().is_blocked(Path::new("env.txt")));
        assert!(!m().is_blocked(Path::new("environment.ts")));
        assert!(!m().is_blocked(Path::new("src/keymaster.rs")));
    }

    #[test]
    fn key_material_blocked() {
        assert!(m().is_blocked(Path::new("server.pem")));
        assert!(m().is_blocked(Path::new("deep/a/b/id_rsa_backup")));
        assert!(m().is_blocked(Path::new("cert.p12")));
    }

    #[test]
    fn git_config_blocked_but_git_head_allowed() {
        assert!(m().is_blocked(Path::new(".git/config")));
        assert!(!m().is_blocked(Path::new(".git/HEAD")));
    }

    #[test]
    fn node_modules_blocked_entirely() {
        assert!(m().is_blocked(Path::new("node_modules")));
        assert!(m().is_blocked(Path::new("node_modules/pkg/index.js")));
        assert!(m().is_blocked(Path::new("web/node_modules/pkg/index.js")));
    }

    #[test]
    fn normal_files_allowed() {
        assert!(!m().is_blocked(Path::new("src/main.rs")));
        assert!(!m().is_blocked(Path::new("README.md")));
    }

    #[test]
    fn backslash_separated_paths_blocked() {
        // Windows callers build relative paths with PathBuf, producing `\`.
        assert!(m().is_blocked(Path::new(r"sub\dir\.env")));
        assert!(m().is_blocked(Path::new(r"web\node_modules\pkg\index.js")));
    }

    #[test]
    fn case_variant_names_blocked() {
        // Windows filesystems are case-insensitive.
        assert!(m().is_blocked(Path::new("Node_Modules/pkg/index.js")));
        assert!(m().is_blocked(Path::new(".ENV")));
        assert!(m().is_blocked(Path::new("SERVER.PEM")));
    }
}
