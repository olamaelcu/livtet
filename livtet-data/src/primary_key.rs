use strum::{Display, EnumIter, EnumString, IntoEnumIterator};

/// Every named uniqueness constraint in the livtet schema whose violation
/// we want to classify as a primary-key-style error.
///
/// This enum mixes two kinds of indexes:
///
/// - Composite (multi-column) primary-key indexes on junction tables
///   and similar many-to-many link tables (e.g. `pk_work_authors`,
///   `pk_edition_tags`). These are the original use case; each maps to
///   an `Index::create().name("pk_…").unique()` call in a migration.
/// - Single-column UNIQUE indexes that enforce a 1:1 cardinality that
///   the rest of the codebase already assumes (e.g.
///   `uq_digital_inventory_edition_id` from m0011). These are
///   semantically equivalent for downstream consumers: a duplicate
///   insert is a duplicate row, full stop.
///
/// `Display` produces the raw index-name string with the `pk_` prefix
/// from `strum(prefix = "pk_")`, suitable for passing directly to
/// `.name()` via `.to_string()` for the composite-PK variants. The
/// single-column UNIQUE variant intentionally diverges — see the
/// in-line comment on `DigitalInventoryEdition` below.
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

    // ── m0011_digital_inventory_unique_edition ────────────────────
    // Display intentionally diverges from the DDL name; this variant is
    // a single-column UNIQUE, not a composite PK. `strum(prefix = "pk_")`
    // yields `pk_digital_inventory_edition` for Display, but the actual
    // index is `uq_digital_inventory_edition_id`. No caller invokes
    // `DigitalInventoryEdition.to_string()` for DDL — the migration uses
    // the literal string directly — so the divergence is safe. If you
    // ever need the DDL name, look it up explicitly rather than relying
    // on Display.
    DigitalInventoryEdition,
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

            // ── m0011_digital_inventory_unique_edition ────────────
            Self::DigitalInventoryEdition => {
                "Another digital inventory row already exists for this edition"
            }
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

            // ── m0011_digital_inventory_unique_edition ────────────
            Self::DigitalInventoryEdition => {
                &["uq_digital_inventory_edition_id", "digital_inventory."]
            }
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
