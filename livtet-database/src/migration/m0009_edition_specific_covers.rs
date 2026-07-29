use sea_orm_migration::prelude::*;

use super::schema::*;
use crate::Constraint;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0009-edition_specific_covers"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // 1. edition_specific_covers table
        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(EditionSpecificCovers::Table)
                    .if_not_exists()
                    .col(pk_db_id(EditionSpecificCovers::Id))
                    .col(db_id(EditionSpecificCovers::EditionId))
                    .col(string(EditionSpecificCovers::CoverPath))
                    .foreign_key(
                        ForeignKey::create()
                            .name(Constraint::EditionSpecificCoversEdition.to_string())
                            .from(
                                EditionSpecificCovers::Table,
                                EditionSpecificCovers::EditionId,
                            )
                            .to(Editions::Table, Editions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            ),
        )
        .await?;

        // 2. edition_covers view — UNION of digital_inventory and
        //    edition_specific_covers cover_path columns, with a
        //    cover_source discriminator ('inventory' or 'manual').
        //    Uses coalesce to prefer digital_inventory over manual
        //    when both exist for the same edition.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE VIEW IF NOT EXISTS edition_covers AS
SELECT
    e.id AS edition_id,
    COALESCE(di.cover_path, esc.cover_path) AS cover_path,
    CASE
        WHEN di.cover_path IS NOT NULL THEN 'inventory'
        WHEN esc.cover_path IS NOT NULL THEN 'manual'
        ELSE NULL
    END AS cover_source
FROM editions e
LEFT JOIN digital_inventory di ON di.edition_id = e.id
LEFT JOIN edition_specific_covers esc ON esc.edition_id = e.id",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP VIEW IF EXISTS edition_covers")
            .await?;
        manager
            .drop_table(Table::drop().table(EditionSpecificCovers::Table).to_owned())
            .await
    }
}
