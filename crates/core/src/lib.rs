pub mod audit;
pub mod confinement;
pub mod db;
pub mod error;
pub mod models;
pub mod ops;
pub mod pause;
pub mod pipeline;
pub mod sensitive;
pub mod workspace_repo;

use error::CoreError;
use pause::PauseFlag;
use sensitive::SensitiveMatcher;
use std::path::Path;

/// Shared services handed to both the MCP server and Tauri commands.
pub struct CoreServices {
    pub db: Database,
    pub pause: PauseFlag,
    pub sensitive: SensitiveMatcher,
    /// Opt-in unrestricted access: when set, capability gates and the
    /// sensitive-file deny list are relaxed for every workspace. The pause
    /// kill switch and audit recording stay active regardless.
    pub unrestricted: pause::Flag,
}

use db::Database;

impl CoreServices {
    /// Opens the database at `db_path` and initializes the kill switch, the
    /// sensitive-file matcher, and the (default-off) unrestricted flag.
    pub fn open(db_path: &Path) -> Result<Self, CoreError> {
        Ok(Self {
            db: Database::open(db_path)?,
            pause: PauseFlag::default(),
            sensitive: SensitiveMatcher::new(),
            unrestricted: pause::Flag::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use error::ErrorCode;

    #[test]
    fn opens_services_from_db_path() {
        let dir = tempfile::tempdir().unwrap();
        let core = CoreServices::open(&dir.path().join("t.db")).unwrap();
        assert!(!core.pause.is_paused());
        assert!(!core.unrestricted.get());
        let err = workspace_repo::get_workspace(&core.db, "ghost").unwrap_err();
        assert_eq!(err.code, ErrorCode::WorkspaceNotFound);
    }
}
