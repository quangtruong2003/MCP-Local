//! Tailscale Funnel connection subsystem: capability secret storage,
//! verified public-health state, sidecar lifecycle, and connection manager.

pub mod health;
pub mod manager;
pub mod secrets;
pub mod sidecar;
pub mod state;

pub use secrets::CapabilitySecretStore;
pub use state::{ConnectionState, ConnectionStatus};
