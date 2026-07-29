//! Server catalog database migrations.
//!
//! Migrations create the core Livtet schema: authors, works, editions,
//! inventory, loans, reading annotations, and search history.

pub mod m0001_core_entities;
pub mod m0002_seed_data;
pub mod m0003_junctions;
pub mod m0004_inventory_loans;
pub mod m0005_reading_annotations;
pub mod m0006_search_history;
pub mod m0007_cover_metadata;
pub mod m0008_saved_searches;
pub mod m0009_edition_specific_covers;
pub mod m0010_edition_files;
pub mod schema;

pub use sea_orm_migration::MigratorTrait;
use sea_orm_migration::{MigrationTrait, prelude::*, sea_orm as orm};

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migration_table_name() -> sea_orm::DynIden {
        "core_migrations".into_iden()
    }

    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(crate::migration::m0001_core_entities::Migration),
            Box::new(crate::migration::m0002_seed_data::Migration),
            Box::new(crate::migration::m0003_junctions::Migration),
            Box::new(crate::migration::m0004_inventory_loans::Migration),
            Box::new(crate::migration::m0005_reading_annotations::Migration),
            Box::new(crate::migration::m0006_search_history::Migration),
            Box::new(crate::migration::m0007_cover_metadata::Migration),
            Box::new(crate::migration::m0008_saved_searches::Migration),
            Box::new(crate::migration::m0009_edition_specific_covers::Migration),
            Box::new(crate::migration::m0010_edition_files::Migration),
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
