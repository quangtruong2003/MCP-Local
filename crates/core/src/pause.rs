//! Kill switch: process-wide flag consulted before every MCP request.

use std::sync::atomic::{AtomicBool, Ordering};

/// Global pause flag. When set, every tool call is denied before any
/// workspace lookup or execution.
#[derive(Default)]
pub struct PauseFlag(AtomicBool);

impl PauseFlag {
    pub fn is_paused(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub fn set(&self, paused: bool) {
        self.0.store(paused, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_not_paused() {
        let f = PauseFlag::default();
        assert!(!f.is_paused());
    }

    #[test]
    fn set_toggles_state() {
        let f = PauseFlag::default();
        f.set(true);
        assert!(f.is_paused());
        f.set(false);
        assert!(!f.is_paused());
    }
}
