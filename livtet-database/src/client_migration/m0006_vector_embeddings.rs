use sea_orm_migration::prelude::*;

use super::schema::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0006-vector_embeddings"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(EditionEmbeddings::Table)
                    .if_not_exists()
                    .col(pk_db_id(EditionEmbeddings::Id))
                    .col(db_id(EditionEmbeddings::EditionId))
                    .col(string(EditionEmbeddings::Model))
                    .col(integer(EditionEmbeddings::Dimensions))
                    .col(binary(EditionEmbeddings::Vector))
                    .take(),
            ),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(EditionEmbeddings::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
