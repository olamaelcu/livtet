//! `livtet-iroh` — peer discovery and node identity for sync.
//!
//! Replaces the manual pairing URI (`livtet://sync`) with `iroh`-based
//! discovery and node identity (`public key` / `node id`).
//! Transport stays hybrid: `iroh` handles pairing/discovery; HTTP
//! (`livtet-sync-http`) continues to serve `/sync/*` data routes.
//!
//! Consumed by `livtet-sync-http`; exposed to mobile through future
//! `livtet-ffi` updates.

pub mod discovery;
pub mod identity;

pub use discovery::{DiscoveryError, DiscoveryService};
pub use iroh::SecretKey;
pub use identity::{InvalidNodeId, NodeId, PeerIdentity};
