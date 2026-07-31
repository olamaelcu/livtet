//! Livtet database crate — consolidated from multiple source crates.
//!
//! ## Module structure
//!
//! - [`entities`] — SeaORM entity models for the server catalog schema
//! - [`client_entities`] — SeaORM entity models for client-owned tables
//!   (device pairing, plugins, sync change-log)
//! - [`migration`] — Database migrations for the server catalog schema
//! - [`client_migration`] — Database migrations for client-owned tables
//! - [`migrator`] — Migration runner abstraction over business + client schemas
//! - [`state`] — Connection pool, SQLite pragmas, `SharedState` + global init/get
//! - [`seed`] — Database seeding for development/testing (behind `fake` feature)
//! - [`test_db`] — Test-only `TestDb` helper
//! - [`error`] — `CoreError` / `CoreResult`
//!
//! ## Features
//!
//! All DB modules compile unconditionally. The `server` / `client` / `core` /
//! `backup` / `plugin` features are empty compatibility no-ops so existing
//! consumer `features = [...]` lists keep resolving. The `fake` feature enables
//! [`seed`].

pub mod constraint;
pub mod db_error;
pub mod error;
pub mod index;
pub mod migrator;
pub mod primary_key;
pub mod unique_index;
pub mod state;
pub mod test_db;

pub mod client_entities;
pub mod client_migration;
pub mod entities;
pub mod migration;

#[cfg(feature = "fake")]
pub mod seed;

// Re-exports for ergonomic use
pub use constraint::Constraint;
pub use db_error::ConstraintViolation;
pub use entities::*;
pub use error::{CoreError, Result as CoreResult};
pub use index::NamedIndex;
pub use migrator::{Kind, connect_with_migrations, run_kinds};
pub use primary_key::PrimaryKey;
pub use unique_index::UniqueIndex;
#[cfg(feature = "fake")]
pub use seed::{SeedConfig, SeedResult, seed_database};
pub use state::{
    SharedState, apply_optimizations, get_state, init_state, is_initialized, optimize_and_close,
    sqlite_pool_options,
};
pub use test_db::TestDb;

pub use sea_orm as orm;
pub use sqlx as sql;
