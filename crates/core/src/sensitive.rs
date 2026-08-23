use globset::{Glob, GlobSet, GlobSetBuilder};
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
            fb.add(Glob::new(p).unwrap());
        }
        let mut db = GlobSetBuilder::new();
        for name in BLOCKED_DIR_NAMES {
            db.add(Glob::new(&format!("**/{name}")).unwrap());
        }
        Self {
            files: fb.build().unwrap(),
            dirs: db.build().unwrap(),
        }
    }

    /// `rel` must be relative to the workspace root. Checks the full relative
    /// path AND every ancestor directory (so `node_modules/x/y` is caught by
    /// the `node_modules` dir rule).
    pub fn is_blocked(&self, rel: &Path) -> bool {
        for anc in rel.ancestors() {
            if anc.as_os_str().is_empty() {
                break;
            }
            if self.files.is_match(anc) || self.dirs.is_match(anc) {
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
}
