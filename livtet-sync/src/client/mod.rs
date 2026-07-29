//! Local sync engine and a typed HTTP client for the livtet sync protocol.
//!
//! The engine speaks sea-orm directly against the `change_log` and
//! `conflicts` tables — it is the same code the desktop server uses
//! to read its own DB, and the same code the FFI uses (over the local
//! SQLite connection on the mobile device) to read/write its own.
//! The HTTP client wraps the 6 `/sync/*` routes the engine does not
//! own.

pub mod client;
pub mod engine;

pub use client::{ClientError, SyncClient};
pub use engine::SyncEngine;
