pub mod error;
pub mod models;

/// Shared services handed to both the MCP server and Tauri commands.
/// Gains fields in later tasks (db, pause, audit).
#[derive(Default)]
pub struct CoreServices;

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_compiles() {
        assert!(true);
    }
}
