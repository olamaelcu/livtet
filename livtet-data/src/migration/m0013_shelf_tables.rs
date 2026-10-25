//! Shelf tables: named shelves per account and the books placed on them.
//!
//! Mirrors the AT Protocol records the desktop publishes
//! (`net.olamaelcu.livtet.biblio.shelf` / `…bookShelving`): a shelf is keyed by
//! its record URI, and a shelving row points at both the shelf and the edition
//! record it places.

use sea_orm_migration::prelude::*;

use super::schema::*;
use crate::{Constraint, NamedIndex};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0013-shelf-tables"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // 1. shelf
        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(Shelf::Table)
                    .if_not_exists()
                    .col(text(Shelf::Uri).primary_key())
                    .col(text(Shelf::Name))
                    .col(text_null(Shelf::Description))
                    .to_owned(),
            ),
        )
        .await?;

        // 2. book_shelf
        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(BookShelf::Table)
                    .if_not_exists()
                    .col(text(BookShelf::Uri).primary_key())
                    .col(text(BookShelf::ShelfUri))
                    .col(text(BookShelf::BookUri))
                    .col(text(BookShelf::Status))
                    .col(integer(BookShelf::Position).null())
                    .col(text_null(BookShelf::Notes))
                    .col(text_null(BookShelf::Emoji))
                    .col(text(BookShelf::Did).default(Expr::val("")))
                    .foreign_key(
                        ForeignKey::create()
                            .name(Constraint::BookShelfShelf.to_string())
                            .from(BookShelf::Table, BookShelf::ShelfUri)
                            .to(Shelf::Table, Shelf::Uri)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            ),
        )
        .await?;

        // A book sits on exactly one shelf per account, so moving it must
        // retarget its row rather than add a second one. Earlier write paths
        // keyed rows by (shelf, book) and could leave several rows for one
        // book; collapse them (newest row wins) before the constraint lands.
        manager
            .get_connection()
            .execute_unprepared(
                "DELETE FROM book_shelf \
                 WHERE rowid NOT IN ( \
                   SELECT MAX(rowid) FROM book_shelf GROUP BY did, book_uri \
                 )",
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("uq_book_shelf_did_book_uri")
                    .table(BookShelf::Table)
                    .col(BookShelf::Did)
                    .col(BookShelf::BookUri)
                    .unique()
                    .to_owned(),
            )
            .await?;

        create_named_index(
            manager,
            NamedIndex::BookShelfShelfUri,
            BookShelf::Table,
            BookShelf::ShelfUri,
        )
        .await?;
        create_named_index(
            manager,
            NamedIndex::BookShelfStatus,
            BookShelf::Table,
            BookShelf::Status,
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(BookShelf::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(Shelf::Table).if_exists().to_owned())
            .await?;
        Ok(())
    }
}
