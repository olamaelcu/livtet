//! Migration runner abstraction over business + client schemas.

use sqlx::{AssertSqlSafe, SqlitePool};

use crate::state::sqlite_pool_options;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Catalog / inventory / reading / annotations (livtet-migration)
    Business,
}

/// Convenience: connect with shared pool options, set database-level pragmas,
/// and run the given migration kinds.
pub async fn connect_with_migrations(
    database_url: &str,
    kinds: impl IntoIterator<Item = &Kind>,
) -> Result<SqlitePool, sqlx::Error> {
    let pool = sqlite_pool_options().connect(database_url).await?;

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

    let kinds = Vec::from_iter(kinds);

    if kinds.contains(&&Kind::Business) {
        crate::migration::Migrator::run(&pool).await?;
    }

    Ok(pool)
}
