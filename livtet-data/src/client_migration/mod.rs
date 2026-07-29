//! Client-owned database migrations.
//!
//! Migrations create client-side tables: change-log, device pairing,
//! session tokens, plugin settings, and edition plugin metadata.

pub mod m0001_change_log;
pub mod m0002_pairing_tables;
pub mod m0003_session_tokens;
pub mod m0004_plugin_settings;
pub mod m0005_edition_plugin_metadata;
pub mod m0006_vector_embeddings;
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
            Box::new(crate::client_migration::m0004_plugin_settings::Migration),
            Box::new(crate::client_migration::m0005_edition_plugin_metadata::Migration),
            Box::new(crate::client_migration::m0006_vector_embeddings::Migration),
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
