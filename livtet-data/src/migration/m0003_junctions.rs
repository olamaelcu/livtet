use sea_orm_migration::prelude::*;

use super::schema::*;
use crate::{Constraint, PrimaryKey};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0003-junctions"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // ── Edition Groups ──────────────────────────────────────────────────────
        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(EditionGroups::Table)
                    .if_not_exists()
                    .col(pk_db_id(EditionGroups::Id))
                    .col(string(EditionGroups::Label))
                    .col(text_null(EditionGroups::Description))
                    .to_owned(),
            ),
        )
        .await?;

        // ── Edition Group Identifiers ────────────────────────────────────────
        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(EditionGroupIdentifiers::Table)
                    .if_not_exists()
                    .col(db_id(EditionGroupIdentifiers::EditionGroupId))
                    .col(string(EditionGroupIdentifiers::IdentifierKind))
                    .col(string(EditionGroupIdentifiers::IdentifierValue))
                    .primary_key(
                        Index::create()
                            .name(PrimaryKey::EditionGroupIdentifiers.to_string())
                            .col(EditionGroupIdentifiers::EditionGroupId)
                            .col(EditionGroupIdentifiers::IdentifierKind)
                            .col(EditionGroupIdentifiers::IdentifierValue),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name(Constraint::EditionGroupIdentifiersGroup.to_string())
                            .from(
                                EditionGroupIdentifiers::Table,
                                EditionGroupIdentifiers::EditionGroupId,
                            )
                            .to(EditionGroups::Table, EditionGroups::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            ),
        )
        .await?;

        // ── Works ───────────────────────────────────────────────────────
        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(Works::Table)
                    .if_not_exists()
                    .col(pk_db_id(Works::Id))
                    .col(string(Works::Title))
                    .col(text_null(Works::Description))
                    .col(string_null(Works::SortTitle))
                    .col(string_null(Works::SeriesType))
                    .col(db_id_null(Works::LanguageId))
                    .col(db_id_null(Works::PreferredEditionId))
                    .foreign_key(
                        ForeignKey::create()
                            .name(Constraint::WorksLanguage.to_string())
                            .from(Works::Table, Works::LanguageId)
                            .to(Languages::Table, Languages::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name(Constraint::WorksPreferredEdition.to_string())
                            .from(Works::Table, Works::PreferredEditionId)
                            .to(Editions::Table, Editions::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            ),
        )
        .await?;

        // ── Editions ────────────────────────────────────────────────────
        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(Editions::Table)
                    .if_not_exists()
                    .col(pk_db_id(Editions::Id))
                    .col(db_id(Editions::WorkId))
                    .col(db_id_null(Editions::GroupId))
                    .col(string_null(Editions::Title))
                    .col(date_null(Editions::PublishedDate))
                    .col(db_id_null(Editions::FormatId))
                    .col(db_id_null(Editions::LanguageId))
                    .col(text_null(Editions::Notes))
                    .col(text_null(Editions::Description))
                    .foreign_key(
                        ForeignKey::create()
                            .name(Constraint::EditionsWork.to_string())
                            .from(Editions::Table, Editions::WorkId)
                            .to(Works::Table, Works::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name(Constraint::EditionsEditionGroup.to_string())
                            .from(Editions::Table, Editions::GroupId)
                            .to(EditionGroups::Table, EditionGroups::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name(Constraint::EditionsFormat.to_string())
                            .from(Editions::Table, Editions::FormatId)
                            .to(Formats::Table, Formats::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name(Constraint::EditionsLanguage.to_string())
                            .from(Editions::Table, Editions::LanguageId)
                            .to(Languages::Table, Languages::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            ),
        )
        .await?;

        // ── Work junction tables (no timestamps) ────────────────────────

        create_strict_table(
            manager,
            &Table::create()
                .table(WorkAuthors::Table)
                .if_not_exists()
                .col(db_id(WorkAuthors::WorkId))
                .col(db_id(WorkAuthors::AuthorId))
                .col(string(WorkAuthors::Role))
                .primary_key(
                    Index::create()
                        .name(PrimaryKey::WorkAuthors.to_string())
                        .col(WorkAuthors::WorkId)
                        .col(WorkAuthors::AuthorId)
                        .col(WorkAuthors::Role),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::WorkAuthorsWork.to_string())
                        .from(WorkAuthors::Table, WorkAuthors::WorkId)
                        .to(Works::Table, Works::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::WorkAuthorsAuthor.to_string())
                        .from(WorkAuthors::Table, WorkAuthors::AuthorId)
                        .to(Authors::Table, Authors::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

        create_strict_table(
            manager,
            &Table::create()
                .table(WorkTags::Table)
                .if_not_exists()
                .col(db_id(WorkTags::WorkId))
                .col(db_id(WorkTags::TagId))
                .primary_key(
                    Index::create()
                        .name(PrimaryKey::WorkTags.to_string())
                        .col(WorkTags::WorkId)
                        .col(WorkTags::TagId),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::WorkTagsWork.to_string())
                        .from(WorkTags::Table, WorkTags::WorkId)
                        .to(Works::Table, Works::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::WorkTagsTag.to_string())
                        .from(WorkTags::Table, WorkTags::TagId)
                        .to(Tags::Table, Tags::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

        create_strict_table(
            manager,
            &Table::create()
                .table(WorkGenres::Table)
                .if_not_exists()
                .col(db_id(WorkGenres::WorkId))
                .col(db_id(WorkGenres::GenreId))
                .primary_key(
                    Index::create()
                        .name(PrimaryKey::WorkGenres.to_string())
                        .col(WorkGenres::WorkId)
                        .col(WorkGenres::GenreId),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::WorkGenresWork.to_string())
                        .from(WorkGenres::Table, WorkGenres::WorkId)
                        .to(Works::Table, Works::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::WorkGenresGenre.to_string())
                        .from(WorkGenres::Table, WorkGenres::GenreId)
                        .to(Genres::Table, Genres::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

        create_strict_table(
            manager,
            &Table::create()
                .table(WorkSubjects::Table)
                .if_not_exists()
                .col(db_id(WorkSubjects::WorkId))
                .col(db_id(WorkSubjects::SubjectId))
                .primary_key(
                    Index::create()
                        .name(PrimaryKey::WorkSubjects.to_string())
                        .col(WorkSubjects::WorkId)
                        .col(WorkSubjects::SubjectId),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::WorkSubjectsWork.to_string())
                        .from(WorkSubjects::Table, WorkSubjects::WorkId)
                        .to(Works::Table, Works::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::WorkSubjectsSubject.to_string())
                        .from(WorkSubjects::Table, WorkSubjects::SubjectId)
                        .to(Subjects::Table, Subjects::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

        create_strict_table(
            manager,
            &Table::create()
                .table(WorkPublishers::Table)
                .if_not_exists()
                .col(db_id(WorkPublishers::WorkId))
                .col(db_id(WorkPublishers::PublisherId))
                .primary_key(
                    Index::create()
                        .name(PrimaryKey::WorkPublishers.to_string())
                        .col(WorkPublishers::WorkId)
                        .col(WorkPublishers::PublisherId),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::WorkPublishersWork.to_string())
                        .from(WorkPublishers::Table, WorkPublishers::WorkId)
                        .to(Works::Table, Works::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::WorkPublishersPublisher.to_string())
                        .from(WorkPublishers::Table, WorkPublishers::PublisherId)
                        .to(Publishers::Table, Publishers::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

        create_strict_table(
            manager,
            &Table::create()
                .table(WorkIdentifiers::Table)
                .if_not_exists()
                .col(db_id(WorkIdentifiers::WorkId))
                .col(db_id(WorkIdentifiers::IdentifierId))
                .primary_key(
                    Index::create()
                        .name(PrimaryKey::WorkIdentifiers.to_string())
                        .col(WorkIdentifiers::WorkId)
                        .col(WorkIdentifiers::IdentifierId),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::WorkIdentifiersWork.to_string())
                        .from(WorkIdentifiers::Table, WorkIdentifiers::WorkId)
                        .to(Works::Table, Works::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::WorkIdentifiersIdentifier.to_string())
                        .from(WorkIdentifiers::Table, WorkIdentifiers::IdentifierId)
                        .to(Identifiers::Table, Identifiers::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

        // ── Edition junction tables (no timestamps) ─────────────────────

        create_strict_table(
            manager,
            &Table::create()
                .table(EditionAuthors::Table)
                .if_not_exists()
                .col(db_id(EditionAuthors::EditionId))
                .col(db_id(EditionAuthors::AuthorId))
                .col(string(EditionAuthors::Role))
                .primary_key(
                    Index::create()
                        .name(PrimaryKey::EditionAuthors.to_string())
                        .col(EditionAuthors::EditionId)
                        .col(EditionAuthors::AuthorId)
                        .col(EditionAuthors::Role),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::EditionAuthorsEdition.to_string())
                        .from(EditionAuthors::Table, EditionAuthors::EditionId)
                        .to(Editions::Table, Editions::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::EditionAuthorsAuthor.to_string())
                        .from(EditionAuthors::Table, EditionAuthors::AuthorId)
                        .to(Authors::Table, Authors::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

        create_strict_table(
            manager,
            &Table::create()
                .table(EditionTags::Table)
                .if_not_exists()
                .col(db_id(EditionTags::EditionId))
                .col(db_id(EditionTags::TagId))
                .primary_key(
                    Index::create()
                        .name(PrimaryKey::EditionTags.to_string())
                        .col(EditionTags::EditionId)
                        .col(EditionTags::TagId),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::EditionTagsEdition.to_string())
                        .from(EditionTags::Table, EditionTags::EditionId)
                        .to(Editions::Table, Editions::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::EditionTagsTag.to_string())
                        .from(EditionTags::Table, EditionTags::TagId)
                        .to(Tags::Table, Tags::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

        create_strict_table(
            manager,
            &Table::create()
                .table(EditionGenres::Table)
                .if_not_exists()
                .col(db_id(EditionGenres::EditionId))
                .col(db_id(EditionGenres::GenreId))
                .primary_key(
                    Index::create()
                        .name(PrimaryKey::EditionGenres.to_string())
                        .col(EditionGenres::EditionId)
                        .col(EditionGenres::GenreId),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::EditionGenresEdition.to_string())
                        .from(EditionGenres::Table, EditionGenres::EditionId)
                        .to(Editions::Table, Editions::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::EditionGenresGenre.to_string())
                        .from(EditionGenres::Table, EditionGenres::GenreId)
                        .to(Genres::Table, Genres::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

        create_strict_table(
            manager,
            &Table::create()
                .table(EditionSubjects::Table)
                .if_not_exists()
                .col(db_id(EditionSubjects::EditionId))
                .col(db_id(EditionSubjects::SubjectId))
                .primary_key(
                    Index::create()
                        .name(PrimaryKey::EditionSubjects.to_string())
                        .col(EditionSubjects::EditionId)
                        .col(EditionSubjects::SubjectId),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::EditionSubjectsEdition.to_string())
                        .from(EditionSubjects::Table, EditionSubjects::EditionId)
                        .to(Editions::Table, Editions::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::EditionSubjectsSubject.to_string())
                        .from(EditionSubjects::Table, EditionSubjects::SubjectId)
                        .to(Subjects::Table, Subjects::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

        create_strict_table(
            manager,
            &Table::create()
                .table(EditionPublishers::Table)
                .if_not_exists()
                .col(db_id(EditionPublishers::EditionId))
                .col(db_id(EditionPublishers::PublisherId))
                .primary_key(
                    Index::create()
                        .name(PrimaryKey::EditionPublishers.to_string())
                        .col(EditionPublishers::EditionId)
                        .col(EditionPublishers::PublisherId),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::EditionPublishersEdition.to_string())
                        .from(EditionPublishers::Table, EditionPublishers::EditionId)
                        .to(Editions::Table, Editions::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::EditionPublishersPublisher.to_string())
                        .from(EditionPublishers::Table, EditionPublishers::PublisherId)
                        .to(Publishers::Table, Publishers::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

        create_strict_table(
            manager,
            &Table::create()
                .table(EditionIdentifiers::Table)
                .if_not_exists()
                .col(db_id(EditionIdentifiers::EditionId))
                .col(db_id(EditionIdentifiers::IdentifierId))
                .primary_key(
                    Index::create()
                        .name(PrimaryKey::EditionIdentifiers.to_string())
                        .col(EditionIdentifiers::EditionId)
                        .col(EditionIdentifiers::IdentifierId),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::EditionIdentifiersEdition.to_string())
                        .from(EditionIdentifiers::Table, EditionIdentifiers::EditionId)
                        .to(Editions::Table, Editions::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::EditionIdentifiersIdentifier.to_string())
                        .from(EditionIdentifiers::Table, EditionIdentifiers::IdentifierId)
                        .to(Identifiers::Table, Identifiers::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

        // ── Series entries (with timestamps) ────────────────────────────
        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(SeriesEntries::Table)
                    .if_not_exists()
                    .col(db_id(SeriesEntries::SeriesId))
                    .col(db_id(SeriesEntries::EditionId))
                    .col(integer(SeriesEntries::Position))
                    .primary_key(
                        Index::create()
                            .name(PrimaryKey::SeriesEntries.to_string())
                            .col(SeriesEntries::SeriesId)
                            .col(SeriesEntries::EditionId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name(Constraint::SeriesEntriesSeries.to_string())
                            .from(SeriesEntries::Table, SeriesEntries::SeriesId)
                            .to(Series::Table, Series::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name(Constraint::SeriesEntriesEdition.to_string())
                            .from(SeriesEntries::Table, SeriesEntries::EditionId)
                            .to(Editions::Table, Editions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
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
                    .table(SeriesEntries::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(EditionIdentifiers::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(EditionPublishers::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(EditionSubjects::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(EditionGenres::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(EditionTags::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(EditionAuthors::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(EditionGroupIdentifiers::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(EditionGroups::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(WorkIdentifiers::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(WorkPublishers::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(WorkSubjects::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(WorkGenres::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(WorkTags::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(WorkAuthors::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(Editions::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Works::Table).if_exists().to_owned())
            .await?;

        Ok(())
    }
}
