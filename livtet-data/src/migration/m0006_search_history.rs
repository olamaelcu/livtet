use sea_orm_migration::prelude::*;

use super::schema::*;
use crate::NamedIndex;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0006-search_history"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        create_strict_table(
            manager,
            &Table::create()
                .table(SearchHistory::Table)
                .if_not_exists()
                .col(pk_db_id(SearchHistory::Id))
                .col(string(SearchHistory::Query))
                .col(timestamp(SearchHistory::SearchedAt).default(Expr::current_timestamp()))
                .to_owned(),
        )
        .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name(NamedIndex::SearchHistorySearchedAt.to_string())
                    .table(SearchHistory::Table)
                    .col(SearchHistory::SearchedAt)
                    .to_owned(),
            )
            .await
    }

    /// Dropping the table takes its index with it, as in every sibling
    /// migration. Without this, `MigrationTrait`'s default `down` errors
    /// ("We Don't Do That Here") and the whole business schema becomes
    /// unrollable past this point.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(SearchHistory::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum SearchHistory {
    Table,
    Id,
    Query,
    SearchedAt,
}
