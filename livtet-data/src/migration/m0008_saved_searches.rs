use sea_orm_migration::prelude::*;

use super::schema::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0008-saved_searches"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                timestamps(
                    Table::create()
                        .table(SavedSearches::Table)
                        .if_not_exists()
                        .col(pk_db_id(SavedSearches::Id))
                        .col(string(SavedSearches::Name))
                        .col(text(SavedSearches::DefinitionJson))
                        .col(text_null(SavedSearches::BindingsJson))
                        .col(text_null(SavedSearches::OptionsJson))
                        .take(),
                )
                .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(SavedSearches::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
