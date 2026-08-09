//! Livtet core library — shared across all livtet crates
//!
//! Extracts all SeaORM entities from the original `livtet-tauri/src/db/` module
//! so they can be used by both the Tauri app and the Kobo sync binaries.

pub use livtet_data::migration::{Migrator, MigratorTrait};
pub use livtet_data::sql::{Error as DbErr, SqlitePool as DatabaseConnection};

pub mod core;
pub mod cover;
pub mod error;
pub mod migrator;
pub mod paths;
pub mod quotes;
#[cfg(feature = "fake")]
pub mod seed;
pub mod user_agent;

pub use livtet_covers as covers;
pub use livtet_covers::{
    CacheKey, CachedCover, CoverError, CoverFetcher, CoverResult, CoverStorage, FetchError,
    FetchedCover,
};
pub use livtet_data as data;
pub use livtet_search as search;
pub use quotes::{EmptyMessage, Greeting, Period};
pub use livtet_types::{Address, DbId, DiskPath, Urn, now_primitive, CommonLanguages, Isbn};
#[cfg(feature = "fake")]
pub use seed::{SeedConfig, SeedResult, seed_database};

pub use crate::{
    core::{
        SharedState, apply_optimizations, get_state, init_state, is_initialized,
        sqlite_pool_options,
    },
    error::{CoreError, Result as CoreResult},
};
