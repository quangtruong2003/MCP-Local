//! Connection subsystem: capability secret storage, the verified state
//! model, and the legacy Cloudflare-era manager that Tasks 7-12 replace.

mod legacy;
pub mod health;
pub mod manager;
pub mod secrets;
pub mod sidecar;
pub mod state;
pub(crate) mod windows_job;

pub use legacy::*;
pub use secrets::CapabilitySecretStore;
pub use state::{ConnectionState, ConnectionStatus};
