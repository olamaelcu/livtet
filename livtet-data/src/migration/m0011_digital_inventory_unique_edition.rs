//! Add a single-column UNIQUE index on `digital_inventory.edition_id`
//! to enforce the 1:1 cardinality between `editions` and
//! `digital_inventory` that the seed data and downstream consumers
//! (the OPDS server's `HashMap<DbId, Model>` view, in particular) already
//! assume.
//!
//! Pre-condition: this migration will fail if a pre-existing database
//! already contains two or more `digital_inventory` rows that share an
//! `edition_id`. Acceptable assumption: no production install exists
//! yet, so we don't ship a deduplication pass.

use sea_orm_migration::prelude::*;

use super::schema::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0011-digital_inventory_unique_edition"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !manager
            .has_index(
                DigitalInventory::Table.to_string(),
                "uq_digital_inventory_edition_id".to_string(),
            )
            .await?
        {
            manager
                .create_index(
                    Index::create()
                        .name("uq_digital_inventory_edition_id")
                        .table(DigitalInventory::Table)
                        .col(DigitalInventory::EditionId)
                        .unique()
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("uq_digital_inventory_edition_id")
                    .table(DigitalInventory::Table)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}
