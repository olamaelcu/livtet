use strum::{Display, EnumIter, EnumString, IntoEnumIterator};

/// Every named foreign-key constraint in the livtet schema.
///
/// Each variant maps to a `.name("fk_…")` call in a migration table
/// definition.  Display produces the raw constraint-name string, suitable
/// for passing directly to `.name()` (via `.to_string()`).
#[derive(Copy, Clone, Debug, Eq, PartialEq, Display, EnumString, EnumIter)]
#[strum(serialize_all = "snake_case", prefix = "fk_")]
pub enum Constraint {
    // ── m0003_junctions ────────────────────────────────────────────────────
    WorksLanguage,
    WorksPreferredEdition,
    EditionsWork,
    EditionsEditionGroup,
    EditionsFormat,
    EditionsLanguage,
    WorkAuthorsWork,
    WorkAuthorsAuthor,
    WorkTagsWork,
    WorkTagsTag,
    WorkGenresWork,
    WorkGenresGenre,
    WorkSubjectsWork,
    WorkSubjectsSubject,
    WorkPublishersWork,
    WorkPublishersPublisher,
    WorkIdentifiersWork,
    WorkIdentifiersIdentifier,
    EditionAuthorsEdition,
    EditionAuthorsAuthor,
    EditionTagsEdition,
    EditionTagsTag,
    EditionGenresEdition,
    EditionGenresGenre,
    EditionSubjectsEdition,
    EditionSubjectsSubject,
    EditionPublishersEdition,
    EditionPublishersPublisher,
    EditionIdentifiersEdition,
    EditionIdentifiersIdentifier,
    EditionGroupIdentifiersGroup,
    EditionGroupIdentifiersIdentifier,
    SeriesEntriesSeries,
    SeriesEntriesEdition,

    // ── m0003_inventory_loans ──────────────────────────────────────
    OwnedEditionsEdition,
    OwnedEditionsCondition,
    LoanEntityIdentifiersEntity,
    EditionsLoansEdition,
    EditionsLoansLoan,
    EditionsLoansOwned,
    DigitalInventoryEdition,
    EditionSpecificCoversEdition,

    // ── m0004_reading_annotations ──────────────────────────────────
    AnnotationsEdition,
    ReadingListBookList,
    ReadingListBookEdition,
    ReadingProgressEdition,
    ReadingProgressFormat,
    WorkStatusWork,

    // ── m0005_plugins_devices ──────────────────────────────────────
    PairedDevicesType,
    PendingPairingsDeviceType,
    PendingPairingsStatus,

    // ── m0009_edition_files ───────────────────────────────────────
    EditionFilesEdition,
}

impl Constraint {
    pub fn human_readable(self) -> &'static str {
        match self {
            // ── m0003_junctions ────────────────────────────────────────────────────
            Self::WorksLanguage => "Referenced language does not exist",
            Self::WorksPreferredEdition => "Referenced edition does not exist",
            Self::EditionsWork => "Referenced work does not exist",
            Self::EditionsEditionGroup => "Referenced edition group does not exist",
            Self::EditionsFormat => "Referenced format does not exist",
            Self::EditionsLanguage => "Referenced language does not exist",
            Self::WorkAuthorsWork => "Referenced work does not exist",
            Self::WorkAuthorsAuthor => "Referenced author does not exist",
            Self::WorkTagsWork => "Referenced work does not exist",
            Self::WorkTagsTag => "Referenced tag does not exist",
            Self::WorkGenresWork => "Referenced work does not exist",
            Self::WorkGenresGenre => "Referenced genre does not exist",
            Self::WorkSubjectsWork => "Referenced work does not exist",
            Self::WorkSubjectsSubject => "Referenced subject does not exist",
            Self::WorkPublishersWork => "Referenced work does not exist",
            Self::WorkPublishersPublisher => "Referenced publisher does not exist",
            Self::WorkIdentifiersWork => "Referenced work does not exist",
            Self::WorkIdentifiersIdentifier => "Referenced identifier does not exist",
            Self::EditionAuthorsEdition => "Referenced edition does not exist",
            Self::EditionAuthorsAuthor => "Referenced author does not exist",
            Self::EditionTagsEdition => "Referenced edition does not exist",
            Self::EditionTagsTag => "Referenced tag does not exist",
            Self::EditionGenresEdition => "Referenced edition does not exist",
            Self::EditionGenresGenre => "Referenced genre does not exist",
            Self::EditionSubjectsEdition => "Referenced edition does not exist",
            Self::EditionSubjectsSubject => "Referenced subject does not exist",
            Self::EditionPublishersEdition => "Referenced edition does not exist",
            Self::EditionPublishersPublisher => "Referenced publisher does not exist",
            Self::EditionIdentifiersEdition => "Referenced edition does not exist",
            Self::EditionIdentifiersIdentifier => "Referenced identifier does not exist",
            Self::EditionGroupIdentifiersGroup => "Referenced edition group does not exist",
            Self::EditionGroupIdentifiersIdentifier => "Referenced identifier does not exist",
            Self::SeriesEntriesSeries => "Referenced series does not exist",
            Self::SeriesEntriesEdition => "Referenced edition does not exist",

            // ── m0003_inventory_loans ──────────────────────────────
            Self::OwnedEditionsEdition => "Referenced edition does not exist",
            Self::OwnedEditionsCondition => "Referenced book condition does not exist",
            Self::LoanEntityIdentifiersEntity => "Referenced loan entity does not exist",
            Self::EditionsLoansEdition => "Referenced edition does not exist",
            Self::EditionsLoansLoan => "Referenced loan entity does not exist",
            Self::EditionsLoansOwned => "Referenced owned edition does not exist",
            Self::DigitalInventoryEdition => "Referenced edition does not exist",
            Self::EditionSpecificCoversEdition => "Referenced edition does not exist",

            // ── m0004_reading_annotations ──────────────────────────
            Self::AnnotationsEdition => "Referenced edition does not exist",
            Self::ReadingListBookList => "Referenced reading list does not exist",
            Self::ReadingListBookEdition => "Referenced edition does not exist",
            Self::ReadingProgressEdition => "Referenced edition does not exist",
            Self::ReadingProgressFormat => "Referenced format does not exist",
            Self::WorkStatusWork => "Referenced work does not exist",

            // ── m0005_plugins_devices ──────────────────────────────
            Self::PairedDevicesType => "Referenced device type does not exist",
            Self::PendingPairingsDeviceType => "Referenced device type does not exist",
            Self::PendingPairingsStatus => "Referenced pairing status does not exist",

            // ── m0009_edition_files ────────────────────────────────
            Self::EditionFilesEdition => "Referenced edition does not exist",
        }
    }

    /// Returns an iterator over every `Constraint` variant in declaration order.
    ///
    /// Primarily used by [`crate::ConstraintViolation::enhance_db_err`] to scan
    /// a database error message for a known FK constraint name substring.
    pub fn all() -> impl Iterator<Item = Self> {
        Self::iter()
    }
}
