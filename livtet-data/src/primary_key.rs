use strum::{Display, EnumIter, EnumString, IntoEnumIterator};

/// Composite (multi-column) primary-key indexes on junction tables
/// and similar many-to-many link tables.
///
/// Each variant maps to an `Index::create().name("pk_…").unique()`
/// call in a migration.  `Display` produces the raw index-name string
/// with the `pk_` prefix from `strum(prefix = "pk_")`, suitable for
/// passing directly to `.name()` via `.to_string()`.
///
/// Single-column UNIQUE indexes live in [`crate::UniqueIndex`] instead
/// — those are semantically different constraints.
///
/// Plain single-column primary keys (e.g. `pk_db_id(…)`) use SeaORM's
/// built-in PK machinery and need no named index here.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Display, EnumString, EnumIter)]
#[strum(serialize_all = "snake_case", prefix = "pk_")]
pub enum PrimaryKey {
    // ── m0003_junctions ────────────────────────────────────────────
    WorkAuthors,
    WorkTags,
    WorkGenres,
    WorkSubjects,
    WorkPublishers,
    WorkIdentifiers,
    EditionAuthors,
    EditionTags,
    EditionGenres,
    EditionSubjects,
    EditionPublishers,
    EditionIdentifiers,
    SeriesEntries,

    // ── m0008_edition_groups ───────────────────────────────────────────
    EditionGroupIdentifiers,

    // ── m0005_reading_annotations ──────────────────────────────────
    ReadingListBook,
}

impl PrimaryKey {
    /// Human-readable description of what a violation of this primary key
    /// means.  Used by [`crate::ConstraintViolation::enhance_db_err`] to
    /// prefix the raw database error with a plain-English explanation.
    pub fn human_readable(self) -> &'static str {
        match self {
            // ── m0003_junctions ────────────────────────────────────
            Self::WorkAuthors => "Duplicate work-author role assignment",
            Self::WorkTags => "Duplicate work-tag assignment",
            Self::WorkGenres => "Duplicate work-genre assignment",
            Self::WorkSubjects => "Duplicate work-subject assignment",
            Self::WorkPublishers => "Duplicate work-publisher assignment",
            Self::WorkIdentifiers => "Duplicate work-identifier assignment",
            Self::EditionAuthors => "Duplicate edition-author role assignment",
            Self::EditionTags => "Duplicate edition-tag assignment",
            Self::EditionGenres => "Duplicate edition-genre assignment",
            Self::EditionSubjects => "Duplicate edition-subject assignment",
            Self::EditionPublishers => "Duplicate edition-publisher assignment",
            Self::EditionIdentifiers => "Duplicate edition-identifier assignment",
            Self::SeriesEntries => "Duplicate series entry",

            // ── m0008_edition_groups ──────────────────────────────
            Self::EditionGroupIdentifiers => "Duplicate edition-group-identifier assignment",

            // ── m0005_reading_annotations ──────────────────────────
            Self::ReadingListBook => "Duplicate reading-list entry",
        }
    }

    /// Substrings used to identify this primary-key-style violation
    /// in a database error message.
    ///
    /// Two patterns are provided per variant:
    /// - The DDL index name (e.g. `"pk_work_authors"` or, for the
    ///   single-column UNIQUE case, `"uq_digital_inventory_edition_id"`)
    ///   — matched by databases such as PostgreSQL that embed index
    ///   names in violation messages.
    /// - The table-column prefix (e.g. `"work_authors."`) — matched
    ///   by SQLite, which reports `"UNIQUE constraint failed:
    ///   work_authors.work_id, ..."` without the index name.
    ///
    /// SQLite substring-match caveat: SQLite foreign-key violations do
    /// not embed constraint names, so a UNIQUE error on a matched
    /// table may be misclassified as a primary-key violation even when
    /// it actually came from an FK check against a column on that
    /// table. This is a pre-existing design trade-off — the substring
    /// match is good enough for the surfaced-error path because both
    /// the genuine UNIQUE violation and the misclassified FK violation
    /// share the same correct user-facing recovery (deduplicate the
    /// row). Callers that need a strictly-correct classification should
    /// re-query the constraint that actually fired.
    pub fn patterns(self) -> &'static [&'static str] {
        match self {
            // ── m0003_junctions ────────────────────────────────────
            Self::WorkAuthors => &["pk_work_authors", "work_authors."],
            Self::WorkTags => &["pk_work_tags", "work_tags."],
            Self::WorkGenres => &["pk_work_genres", "work_genres."],
            Self::WorkSubjects => &["pk_work_subjects", "work_subjects."],
            Self::WorkPublishers => &["pk_work_publishers", "work_publishers."],
            Self::WorkIdentifiers => &["pk_work_identifiers", "work_identifiers."],
            Self::EditionAuthors => &["pk_edition_authors", "edition_authors."],
            Self::EditionTags => &["pk_edition_tags", "edition_tags."],
            Self::EditionGenres => &["pk_edition_genres", "edition_genres."],
            Self::EditionSubjects => &["pk_edition_subjects", "edition_subjects."],
            Self::EditionPublishers => &["pk_edition_publishers", "edition_publishers."],
            Self::EditionIdentifiers => &["pk_edition_identifiers", "edition_identifiers."],
            Self::SeriesEntries => &["pk_series_entries", "series_entries."],

            // ── m0008_edition_groups ──────────────────────────────
            Self::EditionGroupIdentifiers => {
                &["pk_edition_group_identifiers", "edition_group_identifiers."]
            }

            // ── m0005_reading_annotations ──────────────────────────
            Self::ReadingListBook => &["pk_reading_list_book", "reading_list_book."],
        }
    }

    /// Returns an iterator over every `PrimaryKey` variant in declaration order.
    ///
    /// Primarily used by [`crate::ConstraintViolation::enhance_db_err`] to scan
    /// a database error message for a known composite primary-key violation.
    pub fn all() -> impl Iterator<Item = Self> {
        Self::iter()
    }
}
