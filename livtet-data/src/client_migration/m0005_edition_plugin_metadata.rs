//! Migration `m0016` — `edition_plugin_metadata`.
//!
//! Stores arbitrary key-value metadata on editions, attributed to a
//! specific plugin id. Plugins that need to extend the edition shape
//! (KOReader device path, koreader last-sync timestamp, OpenLibrary
//! cover URL, etc.) write rows here instead of altering the `editions`
//! schema. The unique index on `(edition_id, plugin_id, key)` keeps
//! upserts idempotent.

use sea_orm_migration::prelude::*;

use super::schema::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0016_edition_plugin_metadata"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(EditionPluginMetadata::Table)
                    .if_not_exists()
                    .col(pk_db_id(EditionPluginMetadata::Id))
                    .col(db_id(EditionPluginMetadata::EditionId))
                    .col(string(EditionPluginMetadata::PluginId))
                    .col(string(EditionPluginMetadata::Key))
                    .col(string(EditionPluginMetadata::Value))
                    .col(
                        timestamp(EditionPluginMetadata::CreatedAt)
                            .default(Expr::current_timestamp()),
                    )
                    .col(timestamp_null(EditionPluginMetadata::UpdatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_edition_plugin_metadata_edition")
                            .from(
                                EditionPluginMetadata::Table,
                                EditionPluginMetadata::EditionId,
                            )
                            .to(Editions::Table, Editions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("uq_edition_plugin_metadata_lookup")
                    .table(EditionPluginMetadata::Table)
                    .col(EditionPluginMetadata::EditionId)
                    .col(EditionPluginMetadata::PluginId)
                    .col(EditionPluginMetadata::Key)
                    .unique()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("uq_edition_plugin_metadata_lookup")
                    .table(EditionPluginMetadata::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_table(
                Table::drop()
                    .table(EditionPluginMetadata::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
