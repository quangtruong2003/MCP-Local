//! Connection subsystem: capability secret storage (this task) plus the
//! legacy Cloudflare-era manager that Tasks 7-12 will replace.

mod legacy;
pub mod secrets;

pub use legacy::*;
pub use secrets::CapabilitySecretStore;
