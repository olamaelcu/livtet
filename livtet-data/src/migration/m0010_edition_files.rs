use sea_orm_migration::prelude::*;

use super::schema::*;
use crate::Constraint;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0010-edition_files"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table =
            timestamps(
                Table::create()
                    .table(EditionFiles::Table)
                    .if_not_exists()
                    .col(pk_db_id(EditionFiles::Id))
                    .col(db_id(EditionFiles::EditionId))
                    .col(string(EditionFiles::FilePath))
                    .col(string(EditionFiles::FileFormat))
                    .col(big_integer_null(EditionFiles::FileSizeBytes))
                    .col(text_null(EditionFiles::FileLastModified))
                    .col(string(EditionFiles::FileMode).default("link").check(
                        Expr::col(EditionFiles::FileMode).is_in(["link", "symlink", "copy"]),
                    ))
                    .col(string(EditionFiles::SourcePlugin))
                    .col(text_null(EditionFiles::SourceId))
                    .foreign_key(
                        ForeignKey::create()
                            .name(Constraint::EditionFilesEdition.to_string())
                            .from(EditionFiles::Table, EditionFiles::EditionId)
                            .to(Editions::Table, Editions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .take(),
            );

        create_strict_table(manager, &table).await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_edition_files_edition_id")
                    .table(EditionFiles::Table)
                    .col(EditionFiles::EditionId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_edition_files_path")
                    .table(EditionFiles::Table)
                    .col(EditionFiles::FilePath)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(EditionFiles::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
