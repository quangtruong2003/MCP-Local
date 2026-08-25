//! Bounded workspace command and PowerShell execution.
//!
//! `policy` decides what may run (approved-command allowlist, cwd
//! confinement, sanitized environment); `runner` performs the actual spawn
//! with timeout, bounded output capture, and Job Object kill-tree teardown.

// Nothing consumes this API until Task 13C wires it into MCP tools; the
// module-level allow mirrors the targeted allows used in connection/sidecar.rs.
#![allow(dead_code)]

pub(crate) mod policy;
pub(crate) mod resolver;
pub(crate) mod runner;
