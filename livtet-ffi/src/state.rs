//! Shared database pool state for the FFI layer.
//!
//! Provides a global pool registry keyed by database path so the app
//! initializes once and all FFI exports reuse the same connection pool.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use once_cell::sync::Lazy;
use livtet_data::sql::SqlitePool;

static POOL_REGISTRY: Lazy<Mutex<HashMap<String, Arc<SqlitePool>>>> =
    Lazy::new(|| Mutex::new(std::collections::HashMap::new()));

/// Initialize the shared database pool for the given path.
/// Idempotent - subsequent calls with the same path succeed silently.
/// For tests, pass ":memory:" for an in-memory SQLite database.
/// Returns the pool (either existing or newly created).
#[tracing::instrument(ret, err)]
pub async fn init_db_pool(database_path: &str) -> Result<Arc<SqlitePool>, crate::MobileError> {
    {
        let registry = POOL_REGISTRY
            .lock()
            .map_err(|_| crate::MobileError::RegistryLocked)?;
        if let Some(pool) = registry.get(database_path) {
            return Ok(pool.clone());
        }
    }

    let db_url = if database_path == ":memory:" {
        "sqlite::memory:".to_string()
    } else {
        format!("sqlite://{database_path}?mode=rwc")
    };

    let pool = livtet_core::sqlite_pool_options()
        .connect(&db_url)
        .await
        .map_err(|e| crate::MobileError::Database(format!("pool open: {e}")))?;

    // Apply database-level optimizations (WAL, synchronous=NORMAL, auto_vacuum=INCREMENTAL).
    // These persist in the database file per SQLite's semantics and are needed on
    // mobile/FFI paths where connect_with_migrations is not called.
    livtet_core::apply_optimizations(&pool)
        .await
        .map_err(|e| crate::MobileError::Database(format!("apply_optimizations: {e}")))?;

    let arc_pool = Arc::new(pool);
    POOL_REGISTRY
        .lock()
        .map_err(|_| crate::MobileError::RegistryLocked)?
        .insert(database_path.to_string(), arc_pool.clone());
    Ok(arc_pool)
}

/// Check if a pool has been initialized.
pub fn is_initialized() -> bool {
    !POOL_REGISTRY.lock().unwrap().is_empty()
}

/// Clear all pools. Used for tests.
#[cfg(test)]
pub fn clear_pools() {
    POOL_REGISTRY.lock().unwrap().clear();
}
