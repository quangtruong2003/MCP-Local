//! Connection subsystem: capability secret storage, the verified state
//! model, and the legacy Cloudflare-era manager that Tasks 7-12 replace.

pub mod health;
mod legacy;
pub mod manager;
pub mod secrets;
pub mod sidecar;
pub mod state;

pub use legacy::*;
pub use secrets::CapabilitySecretStore;
pub use state::{ConnectionState, ConnectionStatus};
