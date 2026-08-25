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

/// Generic atomic runtime flag with intent-revealing names. Used for
/// opt-in behavior switches (e.g. unrestricted access) that, unlike
/// [`PauseFlag`], RELAX gates instead of applying them.
#[derive(Default)]
pub struct Flag(AtomicBool);

impl Flag {
    pub fn get(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub fn set(&self, value: bool) {
        self.0.store(value, Ordering::SeqCst);
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

    #[test]
    fn flag_defaults_off_and_toggles() {
        let f = Flag::default();
        assert!(!f.get());
        f.set(true);
        assert!(f.get());
        f.set(false);
        assert!(!f.get());
    }
}
