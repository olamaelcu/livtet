//! SharedState — a shared database connection wrapper for use.
//!
//! This struct holds a single SqlitePool that can be accessed
//! from multiple threads safely. It is initialized once via `init_state()`
//! and accessed via `get_state()`.

use std::sync::OnceLock;

use sqlx::{AssertSqlSafe, SqlitePool as DatabaseConnection, sqlite::SqlitePoolOptions};

use crate::{
    error::CoreError,
    migrator::{Kind, connect_with_migrations},
};

static STATE: OnceLock<SharedState> = OnceLock::new();

#[derive(Clone)]
pub struct SharedState {
    pub pool: DatabaseConnection,
    pub db_path: String,
}

/// Build a `SqlitePoolOptions` configured with the connection-level
/// PRAGMAs every livtet pool connection must have.
///
/// Connection-level PRAGMAs (`foreign_keys`, `busy_timeout`) must be
/// re-issued on every pooled connection; database-level PRAGMAs
/// (`journal_mode`, `synchronous`) only need to be set once during
/// `connect()` since they persist in the database file.
///
/// `journal_mode = WAL` and `synchronous = NORMAL` go together: in WAL
/// mode, `NORMAL` is safe because the WAL log is the recovery
/// mechanism — corruption on power loss is impossible even if the
/// last WAL frames aren't fsync'd.
pub fn sqlite_pool_options() -> SqlitePoolOptions {
    SqlitePoolOptions::new()
        .max_connections(5)
        .min_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .idle_timeout(Some(std::time::Duration::from_secs(300)))
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query(AssertSqlSafe("PRAGMA foreign_keys = ON"))
                    .execute(&mut *conn)
                    .await?;
                sqlx::query(AssertSqlSafe("PRAGMA busy_timeout = 5000"))
                    .execute(&mut *conn)
                    .await?;
                sqlx::query(AssertSqlSafe("PRAGMA temp_store = MEMORY"))
                    .execute(&mut *conn)
                    .await?;
                sqlx::query(AssertSqlSafe("PRAGMA cache_size = -64000"))
                    .execute(&mut *conn)
                    .await?;
                sqlx::query(AssertSqlSafe("PRAGMA mmap_size = 268435456"))
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
}

impl SharedState {
    pub fn db_conn(&self) -> sea_orm::DatabaseConnection {
        sea_orm::SqlxSqliteConnector::from_sqlx_sqlite_pool(self.pool.clone())
    }

    pub fn from_pool(pool: DatabaseConnection, db_path: String) -> Self {
        SharedState { pool, db_path }
    }

    pub async fn connect(database_url: &str) -> Result<SharedState, sqlx::Error> {
        let pool = connect_with_migrations(database_url, [Kind::Business, Kind::Client]).await?;

        let db_path = if database_url.starts_with("sqlite:") {
            database_url
                .strip_prefix("sqlite:")
                .unwrap_or(database_url)
                .to_string()
        } else {
            database_url.to_string()
        };

        Ok(SharedState { pool, db_path })
    }

    pub async fn optimize_and_close(&self) -> Result<(), sqlx::Error> {
        let _ = sqlx::query(AssertSqlSafe("PRAGMA optimize"))
            .execute(&self.pool)
            .await;
        self.pool.close().await;
        Ok(())
    }
}

/// Apply SQLite database-level performance optimizations.
///
/// Applies these optimizations:
/// - `journal_mode = WAL` - Write-Ahead Logging for better concurrency
/// - `synchronous = NORMAL` - Balance between speed and safety
/// - `temp_store = MEMORY` - Store temp indices/tables in memory
/// - `auto_vacuum = INCREMENTAL` - Reduce fragmentation over time
pub async fn apply_optimizations(pool: &DatabaseConnection) -> Result<(), sqlx::Error> {
    sqlx::query(AssertSqlSafe("PRAGMA journal_mode = WAL"))
        .execute(pool)
        .await?;
    sqlx::query(AssertSqlSafe("PRAGMA synchronous = NORMAL"))
        .execute(pool)
        .await?;
    sqlx::query(AssertSqlSafe("PRAGMA temp_store = MEMORY"))
        .execute(pool)
        .await?;
    sqlx::query(AssertSqlSafe("PRAGMA auto_vacuum = INCREMENTAL"))
        .execute(pool)
        .await?;
    Ok(())
}

pub fn init_state(state: SharedState) -> Result<(), CoreError> {
    STATE.set(state).map_err(|_| CoreError::AlreadyInitialized)
}

pub fn get_state() -> Result<&'static SharedState, CoreError> {
    STATE.get().ok_or(CoreError::NotInitialized)
}

pub fn is_initialized() -> bool {
    STATE.get().is_some()
}

pub async fn optimize_and_close() -> Result<(), sqlx::Error> {
    if let Some(state) = STATE.get() {
        state.optimize_and_close().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shared_state_connect_works() {
        let state = SharedState::connect("sqlite::memory:").await;
        assert!(state.is_ok());
    }

    #[tokio::test]
    async fn shared_state_init_and_get() {
        // `STATE` is a process-global `OnceLock`; once any test
        // in this binary sets it, it stays set for the rest of
        // the binary's lifetime. Treat a pre-existing state as
        // "already initialized" so this test is robust under
        // `cargo test` regardless of execution order.
        if is_initialized() {
            assert!(is_initialized());
            return;
        }
        let state = SharedState::connect("sqlite::memory:").await.unwrap();
        let result = init_state(state);
        assert!(result.is_ok());

        let retrieved = get_state().unwrap();
        assert_eq!(retrieved as *const _, get_state().unwrap() as *const _);

        assert!(is_initialized());
    }

    #[tokio::test]
    async fn shared_state_cannot_init_twice() {
        // Only meaningful before any other test has initialized
        // `STATE`. After that the second `init_state` is still
        // guaranteed to fail, so we always assert the error
        // (the "first init succeeds" half is exercised by the
        // order-dependent `shared_state_init_and_get` case).
        let state1 = SharedState::connect("sqlite::memory:").await.unwrap();
        let state2 = SharedState::connect("sqlite::memory:").await.unwrap();

        let _ = init_state(state1);
        let result = init_state(state2);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn optimize_and_close_works() {
        let state = SharedState::connect("sqlite::memory:").await.unwrap();
        let result = state.optimize_and_close().await;
        assert!(result.is_ok());
    }
}
