//! Connection subsystem: capability secret storage, the verified state
//! model, and the legacy Cloudflare-era manager that Tasks 7-12 replace.

mod legacy;
pub mod secrets;
pub mod state;

pub use legacy::*;
pub use secrets::CapabilitySecretStore;
pub use state::{ConnectionState, ConnectionStatus};
