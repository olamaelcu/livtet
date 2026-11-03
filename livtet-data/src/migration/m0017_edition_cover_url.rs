use sea_orm_migration::prelude::*;

use super::schema::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0017-edition-cover-url"
    }
}

/// An edition imported from the catalog has no file on disk, so the covers
/// the library shows for it come from the catalog's image URL. Local covers
/// keep their home in `digital_inventory.cover_path` and
/// `digital_inventory.cover_path`; this column only carries a remote URL.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Editions::Table)
                    .add_column(
                        ColumnDef::new(Editions::CoverImageUrl)
                            .text()
                            .null()
                            .to_owned(),
                    )
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Editions::Table)
                    .drop_column(Editions::CoverImageUrl)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}
