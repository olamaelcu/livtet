use sea_orm_migration::prelude::*;

use super::schema::*;
use crate::{Constraint, PrimaryKey};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0005-reading_annotations"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Annotations table
        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(Annotations::Table)
                    .if_not_exists()
                    .col(pk_db_id(Annotations::Id))
                    .col(db_id(Annotations::EditionId))
                    .col(db_id(Annotations::UserId))
                    .col(text_null(Annotations::Content))
                    .col(text_null(Annotations::Location))
                    .foreign_key(
                        ForeignKey::create()
                            .name(Constraint::AnnotationsEdition.to_string())
                            .from(Annotations::Table, Annotations::EditionId)
                            .to(Editions::Table, Editions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            ),
        )
        .await?;

        // Reading lists table
        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(ReadingLists::Table)
                    .if_not_exists()
                    .col(pk_db_id(ReadingLists::Id))
                    .col(string(ReadingLists::Name))
                    .col(text_null(ReadingLists::Description))
                    .to_owned(),
            ),
        )
        .await?;

        // Reading list book junction (composite PK, no timestamps)
        create_strict_table(
            manager,
            &Table::create()
                .table(ReadingListBook::Table)
                .if_not_exists()
                .col(db_id(ReadingListBook::ReadingListId))
                .col(db_id(ReadingListBook::EditionId))
                .col(integer(ReadingListBook::Position))
                .col(timestamp(ReadingListBook::AddedAt).default(Expr::current_timestamp()))
                .primary_key(
                    Index::create()
                        .name(PrimaryKey::ReadingListBook.to_string())
                        .col(ReadingListBook::ReadingListId)
                        .col(ReadingListBook::EditionId),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::ReadingListBookList.to_string())
                        .from(ReadingListBook::Table, ReadingListBook::ReadingListId)
                        .to(ReadingLists::Table, ReadingLists::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::ReadingListBookEdition.to_string())
                        .from(ReadingListBook::Table, ReadingListBook::EditionId)
                        .to(Editions::Table, Editions::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

        // Reading progress table — append-only, so only created_at (no updated_at).
        // `progress_unit` records the semantic unit stored in `progress`:
        //   - "ratio"  : a 0.0-1.0 completion fraction (ebooks / percent-based formats)
        //   - "page"   : a 1-based physical page number
        //   - "seconds": a seek position in seconds (audiobooks)
        create_strict_table(
            manager,
            &Table::create()
                .table(ReadingProgress::Table)
                .if_not_exists()
                .col(pk_db_id(ReadingProgress::Id))
                .col(db_id(ReadingProgress::EditionId))
                .col(db_id(ReadingProgress::FormatId))
                .col(double(ReadingProgress::Progress))
                .col(text_null(ReadingProgress::ProgressUnit).default("ratio".to_string()))
                .col(text_null(ReadingProgress::LastLocation))
                .col(big_integer(ReadingProgress::TotalReadingTimeSecs).default(0i64))
                .col(timestamp(ReadingProgress::CreatedAt).default(Expr::current_timestamp()))
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::ReadingProgressEdition.to_string())
                        .from(ReadingProgress::Table, ReadingProgress::EditionId)
                        .to(Editions::Table, Editions::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::ReadingProgressFormat.to_string())
                        .from(ReadingProgress::Table, ReadingProgress::FormatId)
                        .to(Formats::Table, Formats::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

        // Add unique index on (edition_id, format_id) for reading_progress
        manager
            .create_index(
                Index::create()
                    .name("idx_reading_progress_edition_format")
                    .table(ReadingProgress::Table)
                    .col(ReadingProgress::EditionId)
                    .col(ReadingProgress::FormatId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Work status table (WorkId IS the primary key, with timestamps)
        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(WorkStatus::Table)
                    .if_not_exists()
                    .col(pk_db_id(WorkStatus::WorkId))
                    .col(string(WorkStatus::Status))
                    .foreign_key(
                        ForeignKey::create()
                            .name(Constraint::WorkStatusWork.to_string())
                            .from(WorkStatus::Table, WorkStatus::WorkId)
                            .to(Works::Table, Works::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            ),
        )
        .await?;

        // Add progress_unit column to formats table
        manager
            .alter_table(
                Table::alter()
                    .table(Formats::Table)
                    .add_column(
                        ColumnDef::new(Formats::ProgressUnit)
                            .text()
                            .null()
                            .to_owned(),
                    )
                    .to_owned(),
            )
            .await?;

        // Reading sources table - deterministic ULID-based IDs
        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(ReadingSources::Table)
                    .if_not_exists()
                    .col(pk_db_id(ReadingSources::Id))
                    .col(string(ReadingSources::Urn).unique_key())
                    .col(string(ReadingSources::Name))
                    .col(string_null(ReadingSources::Emoji))
                    .col(string_null(ReadingSources::Color))
                    .col(json_null(ReadingSources::Attributes))
                    .col(string_null(ReadingSources::PluginId))
                    .col(timestamp_null(ReadingSources::DeletedAt))
                    .to_owned(),
            ),
        )
        .await?;

        // Reading sessions table
        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(ReadingSessions::Table)
                    .if_not_exists()
                    .col(pk_db_id(ReadingSessions::Id))
                    .col(db_id(ReadingSessions::EditionId))
                    .col(db_id(ReadingSessions::FormatId))
                    .col(db_id_null(ReadingSessions::SourceId))
                    .col(timestamp(ReadingSessions::StartedAt))
                    .col(timestamp_null(ReadingSessions::EndedAt))
                    .col(big_integer(ReadingSessions::DurationSeconds))
                    .col(json_null(ReadingSessions::RawProgression))
                    .col(double(ReadingSessions::ProgressDelta))
                    .col(text_null(ReadingSessions::LastLocation))
                    .col(text_null(ReadingSessions::Notes))
                    .foreign_key(
                        ForeignKey::create()
                            .name("reading_sessions_edition")
                            .from(ReadingSessions::Table, ReadingSessions::EditionId)
                            .to(Editions::Table, Editions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("reading_sessions_format")
                            .from(ReadingSessions::Table, ReadingSessions::FormatId)
                            .to(Formats::Table, Formats::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("reading_sessions_source")
                            .from(ReadingSessions::Table, ReadingSessions::SourceId)
                            .to(ReadingSources::Table, ReadingSources::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            ),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop in reverse creation order
        manager
            .drop_table(
                Table::drop()
                    .table(ReadingSessions::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(ReadingSources::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;

        // Remove progress_unit column from formats
        manager
            .alter_table(
                Table::alter()
                    .table(Formats::Table)
                    .drop_column(Formats::ProgressUnit)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_table(
                Table::drop()
                    .table(WorkStatus::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(ReadingProgress::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(ReadingListBook::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(ReadingLists::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(Annotations::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
