use strum::{Display, EnumIter, EnumString, IntoEnumIterator};

/// Named (non-PK, non-UNIQUE-enforcement) secondary indexes in the
/// livtet schema.
///
/// Each variant maps to an `Index::create().name("idx_…")` call in a
/// migration. `Display` produces the raw index-name string, suitable
/// for passing directly to `.name()` via `.to_string()`.
///
/// Coverage conventions:
/// - Every foreign-key child column gets an index (SQLite does not
///   auto-index them; joins and cascade deletes otherwise scan).
/// - Junction tables get a reverse index on the non-leading PK column
///   so lookups work in both directions.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Display, EnumIter, EnumString)]
#[strum(serialize_all = "snake_case", prefix = "idx_")]
pub enum NamedIndex {
    // ── m0001_core_entities: vocabulary name/code lookups ──────────
    #[strum(to_string = "idx_authors_name")]
    AuthorsName,
    #[strum(to_string = "idx_genres_name")]
    GenresName,
    #[strum(to_string = "idx_subjects_name")]
    SubjectsName,
    #[strum(to_string = "idx_publishers_name")]
    PublishersName,
    #[strum(to_string = "idx_languages_code")]
    LanguagesCode,
    #[strum(to_string = "idx_series_name")]
    SeriesName,

    // ── m0003_junctions: works / editions FK columns ───────────────
    #[strum(to_string = "idx_works_language_id")]
    WorksLanguageId,
    #[strum(to_string = "idx_works_preferred_edition_id")]
    WorksPreferredEditionId,
    #[strum(to_string = "idx_editions_work_id")]
    EditionsWorkId,
    #[strum(to_string = "idx_editions_group_id")]
    EditionsGroupId,
    #[strum(to_string = "idx_editions_format_id")]
    EditionsFormatId,
    #[strum(to_string = "idx_editions_language_id")]
    EditionsLanguageId,

    // ── m0003_junctions: reverse junction lookups ──────────────────
    #[strum(to_string = "idx_work_authors_author_id")]
    WorkAuthorsAuthorId,
    #[strum(to_string = "idx_work_tags_tag_id")]
    WorkTagsTagId,
    #[strum(to_string = "idx_work_genres_genre_id")]
    WorkGenresGenreId,
    #[strum(to_string = "idx_work_subjects_subject_id")]
    WorkSubjectsSubjectId,
    #[strum(to_string = "idx_work_publishers_publisher_id")]
    WorkPublishersPublisherId,
    #[strum(to_string = "idx_work_identifiers_identifier_id")]
    WorkIdentifiersIdentifierId,
    #[strum(to_string = "idx_edition_authors_author_id")]
    EditionAuthorsAuthorId,
    #[strum(to_string = "idx_edition_tags_tag_id")]
    EditionTagsTagId,
    #[strum(to_string = "idx_edition_genres_genre_id")]
    EditionGenresGenreId,
    #[strum(to_string = "idx_edition_subjects_subject_id")]
    EditionSubjectsSubjectId,
    #[strum(to_string = "idx_edition_publishers_publisher_id")]
    EditionPublishersPublisherId,
    #[strum(to_string = "idx_edition_identifiers_identifier_id")]
    EditionIdentifiersIdentifierId,
    #[strum(to_string = "idx_series_entries_edition_id")]
    SeriesEntriesEditionId,
    #[strum(to_string = "idx_edition_group_identifiers_value")]
    EditionGroupIdentifiersValue,

    // ── m0004_inventory_loans ──────────────────────────────────────
    #[strum(to_string = "idx_owned_editions_edition_id")]
    OwnedEditionsEditionId,
    #[strum(to_string = "idx_owned_editions_condition_id")]
    OwnedEditionsConditionId,
    #[strum(to_string = "idx_loan_entity_identifiers_loan_entity_id")]
    LoanEntityIdentifiersLoanEntityId,
    #[strum(to_string = "idx_editions_loans_edition_id")]
    EditionsLoansEditionId,
    #[strum(to_string = "idx_editions_loans_loan_entity_id")]
    EditionsLoansLoanEntityId,
    #[strum(to_string = "idx_editions_loans_owned_edition_id")]
    EditionsLoansOwnedEditionId,

    // ── m0005_reading_annotations ──────────────────────────────────
    #[strum(to_string = "idx_annotations_edition_id")]
    AnnotationsEditionId,
    #[strum(to_string = "idx_annotations_user_id")]
    AnnotationsUserId,
    #[strum(to_string = "idx_reading_list_book_edition_id")]
    ReadingListBookEditionId,
    #[strum(to_string = "idx_reading_progress_format_id")]
    ReadingProgressFormatId,
    #[strum(to_string = "idx_reading_sessions_edition_id")]
    ReadingSessionsEditionId,
    #[strum(to_string = "idx_reading_sessions_format_id")]
    ReadingSessionsFormatId,
    #[strum(to_string = "idx_reading_sessions_source_id")]
    ReadingSessionsSourceId,
    #[strum(to_string = "idx_reading_sessions_started_at")]
    ReadingSessionsStartedAt,

    // ── m0006_search_history ───────────────────────────────────────
    #[strum(to_string = "idx_search_history_searched_at")]
    SearchHistorySearchedAt,

    // ── m0009_edition_specific_covers (covering index for the view) ─
    #[strum(to_string = "idx_edition_specific_covers_edition_id")]
    EditionSpecificCoversEditionId,
}

impl NamedIndex {
    pub fn all() -> impl Iterator<Item = Self> {
        Self::iter()
    }
}
