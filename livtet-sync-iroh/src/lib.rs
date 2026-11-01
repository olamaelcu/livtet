//! Framed sync protocol over `iroh` streams.
//!
//! This replaces the HTTP transport (`livtet-sync-http`) that made the receiver
//! open a TCP connection to the sender. Every operation here runs on a
//! bidirectional QUIC stream under [`livtet_iroh::SYNC_ALPN`], dialled by node
//! id, so a peer behind a firewall, CGNAT, or an isolated access point is
//! reachable through a direct path, a hole-punched one, or the relay — never by
//! an inbound port this side has to open.
//!
//! See `docs/adr/0007-sync-over-iroh-streams.md`.

pub mod wire;

pub use livtet_iroh::SYNC_ALPN;
pub use wire::{
    FileHeader, Identity, MAX_FRAME_BYTES, Request, Response, WireError, read_frame, write_frame,
};
