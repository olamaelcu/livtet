//! Migration runner abstraction over business + client schemas.

use sqlx::SqlitePool;

use crate::state::{apply_optimizations, sqlite_pool_options};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Catalog / inventory / reading / annotations (livtet-migration)
    Business,
    /// Device pairing / plugins / change_log (livtet-client-migration)
    Client,
}

/// Run the selected migration kinds in the correct order (Business → Client).
pub async fn run_kinds(
    pool: &SqlitePool,
    kinds: impl IntoIterator<Item = Kind>,
) -> Result<(), sqlx::Error> {
    let kinds: Vec<_> = kinds.into_iter().collect();
    let run_business = kinds.contains(&Kind::Business);
    let run_client = kinds.contains(&Kind::Client);

    if run_business {
        crate::migration::Migrator::run(pool).await?;
    }
    if run_client {
        crate::client_migration::Migrator::run(pool).await?;
    }
    Ok(())
}

/// Convenience: connect with shared pool options, set database-level pragmas,
/// and run the given migration kinds.
pub async fn connect_with_migrations(
    database_url: &str,
    kinds: impl IntoIterator<Item = Kind>,
) -> Result<SqlitePool, sqlx::Error> {
    let pool = sqlite_pool_options().connect(database_url).await?;

    // Apply database-level performance optimizations (persist in file)
    apply_optimizations(&pool).await?;

    run_kinds(&pool, kinds).await?;

    Ok(pool)
}
