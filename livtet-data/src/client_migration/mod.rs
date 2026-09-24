//! Client-owned database migrations.
//!
//! Migrations create client-side tables: sync change-log, device pairing,
//! session tokens, and client settings.

pub mod m0001_change_log;
pub mod m0002_pairing_tables;
pub mod m0003_session_tokens;
pub mod m0004_client_settings;
pub mod m0005_sync_triggers;
pub mod m0006_pending_pairings_device_id;
pub mod m0007_pending_pairings_origin_addr;
pub mod schema;

pub use sea_orm_migration::MigratorTrait;
use sea_orm_migration::{MigrationTrait, prelude::*, sea_orm as orm};

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migration_table_name() -> sea_orm::DynIden {
        "client_migrations".into_iden()
    }

    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(crate::client_migration::m0001_change_log::Migration),
            Box::new(crate::client_migration::m0002_pairing_tables::Migration),
            Box::new(crate::client_migration::m0003_session_tokens::Migration),
            Box::new(crate::client_migration::m0004_client_settings::Migration),
            Box::new(crate::client_migration::m0005_sync_triggers::Migration),
            Box::new(crate::client_migration::m0006_pending_pairings_device_id::Migration),
            Box::new(crate::client_migration::m0007_pending_pairings_origin_addr::Migration),
        ]
    }
}

impl Migrator {
    pub async fn run(pool: &sqlx::SqlitePool) -> Result<(), sqlx::Error> {
        let db = orm::SqlxSqliteConnector::from_sqlx_sqlite_pool(pool.clone());
        Migrator::up(&db, None)
            .await
            .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    use super::Migrator;

    /// A single-connection in-memory pool. With more than one connection each
    /// pooled connection would get its own empty `:memory:` database, so the
    /// schema written by one statement would be invisible to the next.
    async fn memory_pool() -> sqlx::SqlitePool {
        let opts =
            SqliteConnectOptions::from_str("sqlite::memory:").expect("valid sqlite in-memory URL");
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("open in-memory sqlite pool")
    }

    #[tokio::test]
    async fn client_migrations_rerun_safe_and_install_triggers() {
        let pool = memory_pool().await;

        // The sync audit triggers target business tables, and SQLite refuses
        // to create a trigger for a table that does not exist, so the
        // business schema must be present before the triggers are installed.
        crate::migration::Migrator::run(&pool)
            .await
            .expect("business migrations");

        Migrator::run(&pool).await.expect("first client run");
        Migrator::run(&pool).await.expect("second client run");

        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master WHERE type = 'trigger' AND name LIKE 'sync_%'",
        )
        .fetch_one(&pool)
        .await
        .expect("count sync triggers");

        assert!(count > 0, "expected sync audit triggers, got {count}");
    }
}
