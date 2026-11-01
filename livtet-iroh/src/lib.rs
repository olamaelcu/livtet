//! `livtet-iroh` — peer discovery and node identity for sync.
//!
//! Replaces the manual pairing URI (`livtet://sync`) with `iroh`-based
//! discovery and node identity (`public key` / `node id`).
//! Sync data itself runs over framed requests on bidirectional `iroh` streams
//! under [`SYNC_ALPN`], so neither side needs a reachable TCP port; see
//! `livtet-sync-iroh`.

pub mod discovery;
pub mod identity;

pub use discovery::{DiscoveryError, DiscoveryService, SYNC_ALPN};
pub use identity::{InvalidNodeId, NodeId, PeerIdentity};
pub use iroh::SecretKey;
