//! Livtet core library — shared across all livtet crates
//!
//! Extracts all SeaORM entities from the original `livtet-tauri/src/db/` module
//! so they can be used by both the Tauri app and the Kobo sync binaries.

pub use livtet_data::{
    self as data,
    migration::{Migrator, MigratorTrait},
    sql::{Error as DbErr, SqlitePool as DatabaseConnection},
};

pub mod core;
#[cfg(feature = "covers")]
pub mod covers;
pub mod paths;
#[cfg(feature = "fake")]
pub mod seed;
pub mod user_agent;

pub use livtet_search as search;
pub use livtet_temporal_quotes as temporal_quotes;
pub use livtet_types as types;

pub use crate::core::{SharedState, get_state, init_state, is_initialized, sqlite_pool_options};
