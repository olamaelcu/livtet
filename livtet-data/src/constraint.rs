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
    SeriesEntriesSeries,
    SeriesEntriesEdition,

    // ── m0004_inventory_loans ──────────────────────────────────────
    OwnedEditionsEdition,
    OwnedEditionsCondition,
    LoanEntityIdentifiersEntity,
    EditionsLoansEdition,
    EditionsLoansLoan,
    EditionsLoansOwned,
    DigitalInventoryEdition,

    // ── m0009_edition_specific_covers ──────────────────────────────
    EditionSpecificCoversEdition,

    // ── m0005_reading_annotations ──────────────────────────────────
    AnnotationsEdition,
    ReadingListBookList,
    ReadingListBookEdition,
    ReadingProgressEdition,
    ReadingProgressFormat,
    WorkStatusWork,

    // ── client-0002-pairing_tables ─────────────────────────────────
    PairedDevicesType,
    PendingPairingsDeviceType,
    PendingPairingsStatus,

    // ── m0013_shelf_tables ─────────────────────────────────────────
    BookShelfShelf,
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
            Self::SeriesEntriesSeries => "Referenced series does not exist",
            Self::SeriesEntriesEdition => "Referenced edition does not exist",

            // ── m0004_inventory_loans ──────────────────────────────
            Self::OwnedEditionsEdition => "Referenced edition does not exist",
            Self::OwnedEditionsCondition => "Referenced book condition does not exist",
            Self::LoanEntityIdentifiersEntity => "Referenced loan entity does not exist",
            Self::EditionsLoansEdition => "Referenced edition does not exist",
            Self::EditionsLoansLoan => "Referenced loan entity does not exist",
            Self::EditionsLoansOwned => "Referenced owned edition does not exist",
            Self::DigitalInventoryEdition => "Referenced edition does not exist",

            // ── m0009_edition_specific_covers ──────────────────────
            Self::EditionSpecificCoversEdition => "Referenced edition does not exist",

            // ── m0005_reading_annotations ──────────────────────────
            Self::AnnotationsEdition => "Referenced edition does not exist",
            Self::ReadingListBookList => "Referenced reading list does not exist",
            Self::ReadingListBookEdition => "Referenced edition does not exist",
            Self::ReadingProgressEdition => "Referenced edition does not exist",
            Self::ReadingProgressFormat => "Referenced format does not exist",
            Self::WorkStatusWork => "Referenced work does not exist",

            // ── client-0002-pairing_tables ─────────────────────────
            Self::PairedDevicesType => "Referenced device type does not exist",
            Self::PendingPairingsDeviceType => "Referenced device type does not exist",
            Self::PendingPairingsStatus => "Referenced pairing status does not exist",
            Self::BookShelfShelf => "Referenced shelf does not exist",
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

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use super::Constraint;

    #[test]
    fn every_variant_has_a_message_about_the_thing_it_references() {
        for constraint in Constraint::all() {
            let message = constraint.human_readable();
            assert!(!message.trim().is_empty(), "{constraint:?} has no message");
            // These are all foreign keys, so every message is about a missing
            // referent — the message is shown to a user, not logged.
            assert!(
                message.starts_with("Referenced "),
                "{constraint:?}: {message:?}"
            );
            assert!(
                message.ends_with(" does not exist"),
                "{constraint:?}: {message:?}"
            );
        }
    }

    /// The name is what reaches the database, via `.name()` in a migration,
    /// and what [`crate::ConstraintViolation::enhance_db_err`] searches a
    /// driver error for. Two variants sharing one name would make that search
    /// ambiguous.
    #[test]
    fn names_are_prefixed_snake_case_and_unique() {
        let names: Vec<String> = Constraint::all().map(|c| c.to_string()).collect();
        for name in &names {
            assert!(name.starts_with("fk_"), "{name:?} is not fk_-prefixed");
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{name:?} is not snake_case"
            );
        }
        let mut deduped = names.clone();
        deduped.sort_unstable();
        deduped.dedup();
        assert_eq!(
            deduped.len(),
            names.len(),
            "two constraints share a name, which would make error matching ambiguous"
        );
    }

    /// Pins an asymmetry rather than endorsing it: strum applies
    /// `prefix = "fk_"` to `Display` but not to `EnumString`, so a
    /// `Constraint` does **not** parse back from the name it prints.
    /// `FromStr` wants the unprefixed form.
    ///
    /// Nothing outside this module parses a `Constraint` today — the name is
    /// only ever written out, into a migration's `.name()` and into
    /// `enhance_db_err`'s substring scan — so this is a latent trap rather
    /// than a live bug. Rewrite this test if the derive is ever made
    /// symmetric; the failure will tell you it happened.
    #[test]
    fn from_str_wants_the_unprefixed_name_that_display_does_not_print() {
        for constraint in Constraint::all() {
            let printed = constraint.to_string();
            let unprefixed = printed
                .strip_prefix("fk_")
                .expect("Display is fk_-prefixed");

            assert_eq!(
                Constraint::from_str(unprefixed).expect(unprefixed),
                constraint
            );
            assert!(
                Constraint::from_str(&printed).is_err(),
                "{printed} parsed, so Display and FromStr now agree"
            );
        }
        assert!(Constraint::from_str("not_a_constraint").is_err());
    }

    /// `all()` is what `enhance_db_err` scans, so a variant missing from it
    /// would silently stop producing its message.
    #[test]
    fn all_yields_every_variant_once() {
        let count = Constraint::all().count();
        let mut seen: Vec<Constraint> = Constraint::all().collect();
        seen.dedup();
        assert_eq!(seen.len(), count, "all() repeats a variant");
        assert!(count >= 50, "expected the full constraint set, got {count}");
    }
}
