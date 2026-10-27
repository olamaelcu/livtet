use sea_orm_migration::prelude::*;

use super::schema::*;
use crate::{Constraint, NamedIndex};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0015-reading_bookmarks"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Page-level bookmarks: a location a reader flagged without any note
        // text or highlight colour. Kept apart from `annotations` (a note has
        // content) and from `reading_progress` (a position is not a marker).
        // Append-only, so only `created_at`.
        //
        // The position is encoded exactly like `reading_progress`:
        // `progress_unit` names the unit stored in `progress` —
        //   - "ratio"     : a 0.0-1.0 completion fraction
        //   - "page"      : a 1-based page number (PDF and physical books)
        //   - "timestamp" : a seek position in seconds (audiobooks)
        //   - "epub"      : a serialized Readium locator in `last_location`
        // — so a bookmark works for every reader family.
        create_strict_table(
            manager,
            &Table::create()
                .table(ReadingBookmarks::Table)
                .if_not_exists()
                .col(pk_db_id(ReadingBookmarks::Id))
                .col(db_id(ReadingBookmarks::EditionId))
                .col(double(ReadingBookmarks::Progress))
                .col(text_null(ReadingBookmarks::ProgressUnit).default("ratio".to_string()))
                .col(text_null(ReadingBookmarks::LastLocation))
                .col(timestamp(ReadingBookmarks::CreatedAt).default(Expr::current_timestamp()))
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::ReadingBookmarksEdition.to_string())
                        .from(ReadingBookmarks::Table, ReadingBookmarks::EditionId)
                        .to(Editions::Table, Editions::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

        // Bookmarks are listed per edition, and SQLite does not index
        // foreign-key child columns on its own.
        create_named_index(
            manager,
            NamedIndex::ReadingBookmarksEditionId,
            ReadingBookmarks::Table,
            ReadingBookmarks::EditionId,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name(NamedIndex::ReadingBookmarksEditionId.to_string())
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(ReadingBookmarks::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}
