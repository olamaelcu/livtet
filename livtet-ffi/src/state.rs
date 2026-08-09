//! Shared database pool state for the FFI layer.
//!
//! Provides a global pool registry keyed by database path so the app
//! initializes once and all FFI exports reuse the same connection pool.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use livtet_data::sql::SqlitePool;
use once_cell::sync::Lazy;

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

/// Wipe every user-data table in the local database.
///
/// Reads `sqlite_master` for non-internal tables and `DROP TABLE`s each
/// inside a single transaction, all on the existing pool. We use
/// `DROP TABLE IF EXISTS` rather than `DELETE FROM` so that all indexes
/// (including `CREATE UNIQUE INDEX` statements from sea-orm migrations)
/// are dropped along with their table — otherwise the next `init_inner`
/// fails to re-apply a migration with "index already exists".
///
/// Migration tracking tables (`core_migrations`, `client_migrations`)
/// are deliberately skipped so the migration runner doesn't try to
/// re-apply already-applied migrations against dropped tables.
///
/// Returns the list of table names that were dropped, in the order
/// `sqlite_master` returned them.
#[tracing::instrument(skip_all, err)]
pub async fn wipe_user_tables() -> Result<Vec<String>, crate::MobileError> {
    use livtet_data::sql::{AssertSqlSafe, Row};

    let state = get_state().map_err(crate::MobileError::from)?;
    let pool: &SqlitePool = &state.pool;

    // 1. Fetch all user-data table names from sqlite_master, excluding
    //    the migration tracking tables and sqlite-internal tables.
    let rows = livtet_data::sql::query(
        "SELECT name FROM sqlite_master \
         WHERE type='table' AND name NOT LIKE 'sqlite_%' \
           AND name NOT IN ('core_migrations', 'client_migrations') \
         ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| crate::MobileError::Database(format!("wipe: list tables: {e}")))?;
    let names: Vec<String> = rows
        .into_iter()
        .map(|r| r.try_get::<String, _>(0))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| crate::MobileError::Database(format!("wipe: read name: {e}")))?;

    // 2. Wipe sqlite_master directly with `writable_schema = ON`.
    //    This is the most robust approach: SQLite normally prevents
    //    `DROP TABLE` from violating foreign-key references, and the
    //    `PRAGMA foreign_keys = OFF` trick isn't enough on every
    //    SQLite build (e.g. when a sea-orm migration defines a
    //    referenced table that's already been dropped). Going through
    //    `writable_schema` lets us nuke every user-table row from
    //    `sqlite_master` in one shot; the engine then unlinks the
    //    tables and their indexes on the next access.
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| crate::MobileError::Database(format!("wipe: begin tx: {e}")))?;
    livtet_data::sql::query(AssertSqlSafe("PRAGMA writable_schema = ON"))
        .execute(&mut *tx)
        .await
        .map_err(|e| crate::MobileError::Database(format!("wipe: PRAGMA writable_schema=ON: {e}")))?;
    livtet_data::sql::query(AssertSqlSafe(
        "DELETE FROM sqlite_master \
         WHERE type='table' AND name NOT LIKE 'sqlite_%' \
           AND name NOT IN ('core_migrations', 'client_migrations')",
    ))
    .execute(&mut *tx)
    .await
    .map_err(|e| crate::MobileError::Database(format!("wipe: DELETE sqlite_master: {e}")))?;
    livtet_data::sql::query(AssertSqlSafe("PRAGMA writable_schema = OFF"))
        .execute(&mut *tx)
        .await
        .map_err(|e| crate::MobileError::Database(format!("wipe: PRAGMA writable_schema=OFF: {e}")))?;
    // VACUUM is overkill here (and slow); the next migration run will
    // simply `CREATE TABLE IF NOT EXISTS` for everything.
    tx.commit()
        .await
        .map_err(|e| crate::MobileError::Database(format!("wipe: commit: {e}")))?;
    Ok(names)
}

fn get_state() -> Result<&'static livtet_data::SharedState, livtet_data::CoreError> {
    livtet_data::get_state()
}
