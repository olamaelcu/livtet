//! Client entities module.
//!
//! Contains SeaORM entity models for client-owned tables:
//! device pairing, plugins, sync change-log, and vector embeddings.

pub mod entities;

#[cfg(feature = "client")]
pub mod embedding;

pub use entities::*;
