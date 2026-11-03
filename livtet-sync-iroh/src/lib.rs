//! Framed sync protocol over `iroh` streams.
//!
//! This replaces the retired HTTP transport, which made the receiver open a
//! TCP connection to the sender. Every operation here runs on a
//! bidirectional QUIC stream under [`livtet_iroh::SYNC_ALPN`], dialled by node
//! id, so a peer behind a firewall, CGNAT, or an isolated access point is
//! reachable through a direct path, a hole-punched one, or the relay — never by
//! an inbound port this side has to open.
//!
//! See `docs/adr/0007-sync-over-iroh-streams.md`.

pub mod client;
pub mod server;
pub mod wire;

pub use client::{DialError, Exchange, call, dial};
pub use iroh::endpoint::Connection;
pub use livtet_iroh::SYNC_ALPN;
pub use server::{FileBody, Handler, Outcome, spawn as spawn_server};
pub use wire::{
    FileHeader, Identity, MAX_FRAME_BYTES, Request, Response, WireError, read_frame, write_frame,
};
