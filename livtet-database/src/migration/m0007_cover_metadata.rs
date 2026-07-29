use sea_orm_migration::prelude::*;

use super::schema::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0007-cover_metadata"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !manager
            .has_column(
                DigitalInventory::Table.to_string(),
                DigitalInventory::Blurhash.to_string(),
            )
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(DigitalInventory::Table)
                        .add_column(ColumnDef::new(DigitalInventory::Blurhash).text())
                        .to_owned(),
                )
                .await?;
        }

        if !manager
            .has_column(
                DigitalInventory::Table.to_string(),
                DigitalInventory::DominantColor.to_string(),
            )
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(DigitalInventory::Table)
                        .add_column(
                            ColumnDef::new(DigitalInventory::DominantColor)
                                .text()
                                .null(),
                        )
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(DigitalInventory::Table)
                    .drop_column(DigitalInventory::DominantColor)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(DigitalInventory::Table)
                    .drop_column(DigitalInventory::Blurhash)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}
