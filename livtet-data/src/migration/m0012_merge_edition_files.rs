//! Merge `edition_files` into `digital_inventory` by adding `file_format`
//! column and dropping `edition_files` table.
//!
//! This migration:
//! 1. Adds `file_format` column to `digital_inventory` table
//! 2. Drops `edition_files` table (along with its indexes)
//!
//! Note: No data migration is performed since `edition_files` is being
//! dropped entirely and no production data exists yet.

use sea_orm_migration::prelude::*;

use super::schema::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0012-merge-edition-files"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add file_format column to digital_inventory
        if !manager
            .has_column(
                DigitalInventory::Table.to_string(),
                DigitalInventory::FileFormat.to_string(),
            )
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(DigitalInventory::Table)
                        .add_column(
                            ColumnDef::new(DigitalInventory::FileFormat)
                                .text()
                                .not_null()
                                .default(Expr::val("EPUB"))
                                .to_owned(),
                        )
                        .to_owned(),
                )
                .await?;
        }

        // Drop edition_files table if it exists (idempotent for fresh databases)
        if manager.has_table("edition_files").await? {
            manager
                .get_connection()
                .execute_unprepared("DROP TABLE edition_files")
                .await?;
        }

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Migration is intentionally not reversible
        Err(DbErr::Custom("migration not reversible".to_string()))
    }
}