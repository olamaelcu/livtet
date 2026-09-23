//! Migration runner abstraction over business + client schemas.

use std::str::FromStr;

use sqlx::{AssertSqlSafe, SqlitePool, sqlite::SqliteConnectOptions};

use crate::state::sqlite_pool_options;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Catalog / inventory / reading / annotations (livtet-migration)
    Business,
    /// Sync change-log / device pairing / client settings (client_migration)
    Client,
}

/// Run the selected migration kinds in dependency order (Business → Client).
///
/// Business and client migrate independently: each owns its own migration
/// table (`core_migrations` / `client_migrations`), so running one kind
/// never touches the other's bookkeeping.
pub async fn run_kinds(pool: &SqlitePool, kinds: &[Kind]) -> Result<(), sqlx::Error> {
    if kinds.contains(&Kind::Business) {
        crate::migration::Migrator::run(pool).await?;
    }
    if kinds.contains(&Kind::Client) {
        crate::client_migration::Migrator::run(pool).await?;
    }
    Ok(())
}

/// Convenience: connect with shared pool options, set database-level pragmas,
/// and run the given migration kinds.
///
/// `database_url` may be a bare file path or a `sqlite:` URL; file is created if missing.
pub async fn connect_with_migrations(
    database_url: &str,
    kinds: impl IntoIterator<Item = &Kind>,
) -> Result<SqlitePool, sqlx::Error> {
    let normalized = if database_url.starts_with("sqlite:") {
        database_url.to_string()
    } else {
        format!("sqlite:{}", database_url)
    };
    let opts = SqliteConnectOptions::from_str(&normalized)?.create_if_missing(true);
    let pool = sqlite_pool_options().connect_with(opts).await?;

    // Apply database-level performance optimizations (persist in file)
    sqlx::query(AssertSqlSafe("PRAGMA journal_mode = WAL"))
        .execute(&pool)
        .await?;
    sqlx::query(AssertSqlSafe("PRAGMA synchronous = NORMAL"))
        .execute(&pool)
        .await?;
    sqlx::query(AssertSqlSafe("PRAGMA temp_store = MEMORY"))
        .execute(&pool)
        .await?;
    sqlx::query(AssertSqlSafe("PRAGMA auto_vacuum = INCREMENTAL"))
        .execute(&pool)
        .await?;

    let kinds: Vec<Kind> = kinds.into_iter().copied().collect();
    run_kinds(&pool, &kinds).await?;

    Ok(pool)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn connect_with_migrations_creates_missing_file() {
        let dir = camino_tempfile::Utf8TempDir::new().unwrap();
        let db_path = dir.path().join("missing.db").to_string(); // bare path, file does NOT exist
        let pool = super::connect_with_migrations(&db_path, &[super::Kind::Business]).await;
        assert!(
            pool.is_ok(),
            "bare path without pre-existing file should create DB: {pool:?}"
        );
    }

    #[tokio::test]
    async fn connect_with_migrations_creates_missing_file_with_sqlite_prefix() {
        let dir = camino_tempfile::Utf8TempDir::new().unwrap();
        let db_path = format!("sqlite:{}", dir.path().join("missing2.db")); // sqlite: URL, file does NOT exist
        let pool = super::connect_with_migrations(&db_path, &[super::Kind::Business]).await;
        assert!(
            pool.is_ok(),
            "sqlite: URL without pre-existing file should create DB: {pool:?}"
        );
    }

    #[tokio::test]
    async fn connect_with_migrations_memory_still_works() {
        let pool =
            super::connect_with_migrations("sqlite::memory:", &[super::Kind::Business]).await;
        assert!(pool.is_ok(), "sqlite::memory: should still work: {pool:?}");
    }

    #[tokio::test]
    async fn connect_with_migrations_runs_client_kind() {
        let pool = super::connect_with_migrations("sqlite::memory:", &[super::Kind::Client]).await;
        assert!(
            pool.is_ok(),
            "client migrations should run on their own: {pool:?}"
        );
    }

    #[tokio::test]
    async fn connect_with_migrations_runs_both_kinds() {
        let kinds = [super::Kind::Business, super::Kind::Client];
        let pool = super::connect_with_migrations("sqlite::memory:", &kinds).await;
        assert!(pool.is_ok(), "business + client should migrate: {pool:?}");
    }
}
