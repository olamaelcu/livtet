//! Unified database-error enrichment for the livtet schema.
//!
//! [`ConstraintViolation`] is the single type callers should use when
//! converting a [`sea_orm::DbErr`] into something more actionable.
//! It covers both foreign-key violations ([`Constraint`]) and composite
//! primary-key / unique-index violations ([`PrimaryKey`]).
//!
//! # Matching strategy
//!
//! [`ConstraintViolation::enhance_db_err`] searches the raw error message
//! for substrings rather than doing an exact parse, because different
//! database engines surface constraint names differently:
//!
//! | Engine     | FK violation message                                        |
//! |------------|-------------------------------------------------------------|
//! | PostgreSQL | `…violates foreign key constraint "fk_works_language"`      |
//! | SQLite     | `FOREIGN KEY constraint failed` (no constraint name)        |
//!
//! | Engine     | UNIQUE / PK violation message                               |
//! |------------|-------------------------------------------------------------|
//! | PostgreSQL | `…violates unique constraint "pk_work_authors"`             |
//! | SQLite     | `UNIQUE constraint failed: work_authors.work_id, …`        |
//!
//! FK matching works with PostgreSQL (constraint name substring).
//! PK matching works with both PostgreSQL (index name) and SQLite
//! (table-name prefix via [`PrimaryKey::patterns`]).

use crate::{Constraint, PrimaryKey};

/// A named database constraint that was violated.
///
/// Returned as the second element of the tuple from
/// [`ConstraintViolation::enhance_db_err`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ConstraintViolation {
    /// A foreign-key constraint defined in [`Constraint`] was violated.
    ForeignKey(Constraint),
    /// A composite primary-key index defined in [`PrimaryKey`] was violated.
    CompositeKey(PrimaryKey),
}

impl ConstraintViolation {
    /// A short, plain-English description of what this violation means.
    pub fn human_readable(self) -> &'static str {
        match self {
            Self::ForeignKey(c) => c.human_readable(),
            Self::CompositeKey(pk) => pk.human_readable(),
        }
    }

    /// Inspect a [`sea_orm::DbErr`] and attempt to identify which named
    /// constraint was violated.
    ///
    /// Returns `(message, Some(violation))` when a known constraint name or
    /// table-prefix pattern is found in the error text — `message` is the
    /// original error string prefixed with a human-readable explanation.
    /// Returns `(original_message, None)` when no known constraint is
    /// recognised.
    ///
    /// # Matching order
    ///
    /// 1. Foreign-key constraints ([`Constraint::all`]) — substring match on
    ///    the constraint name (e.g. `"fk_works_language"`).
    /// 2. Composite primary-key indexes ([`PrimaryKey::all`]) — substring
    ///    match on [`PrimaryKey::patterns`], which covers both the DDL index
    ///    name and the SQLite `table.column` format.
    pub fn enhance_db_err(err: sea_orm::DbErr) -> (String, Option<Self>) {
        let msg = err.to_string();

        // ── 1. Foreign-key constraints ──────────────────────────────────
        // Search for any FK constraint name as a substring of the message.
        // PostgreSQL embeds the constraint name; SQLite does not, so this
        // path only fires for PostgreSQL-style messages.
        if let Some(c) = Constraint::all().find(|c| msg.contains(c.to_string().as_str())) {
            let violation = Self::ForeignKey(c);
            return (
                format!("{} ({})", violation.human_readable(), msg),
                Some(violation),
            );
        }

        // ── 2. Composite primary-key / unique constraints ───────────────
        // Each PrimaryKey variant supplies two patterns: the DDL index name
        // (PostgreSQL) and the SQLite table.column prefix.
        if let Some(pk) =
            PrimaryKey::all().find(|pk| pk.patterns().iter().any(|p| msg.contains(*p)))
        {
            let violation = Self::CompositeKey(pk);
            return (
                format!("{} ({})", violation.human_readable(), msg),
                Some(violation),
            );
        }

        (msg, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db_err(msg: &str) -> sea_orm::DbErr {
        sea_orm::DbErr::Custom(msg.to_owned())
    }

    #[test]
    fn enhance_returns_none_for_unrecognised_message() {
        let (msg, violation) = ConstraintViolation::enhance_db_err(db_err("some random error"));
        assert!(violation.is_none());
        assert!(
            msg.contains("some random error"),
            "passthrough message must contain the original text, got: {msg}"
        );
    }

    #[test]
    fn enhance_detects_fk_by_constraint_name_substring() {
        // PostgreSQL embeds the constraint name in violation messages.
        let raw = r#"ERROR: insert or update on table "editions" violates foreign key constraint "fk_editions_work""#;
        let (msg, violation) = ConstraintViolation::enhance_db_err(db_err(raw));
        assert!(
            matches!(
                violation,
                Some(ConstraintViolation::ForeignKey(Constraint::EditionsWork))
            ),
            "expected ForeignKey(EditionsWork), got {violation:?}"
        );
        assert!(
            msg.contains("Referenced work does not exist"),
            "message should include human_readable text, got: {msg}"
        );
    }

    #[test]
    fn enhance_detects_pk_by_index_name_substring() {
        // PostgreSQL embeds the index name for unique violations.
        let raw = r#"ERROR: duplicate key value violates unique constraint "pk_work_authors""#;
        let (msg, violation) = ConstraintViolation::enhance_db_err(db_err(raw));
        assert!(
            matches!(
                violation,
                Some(ConstraintViolation::CompositeKey(PrimaryKey::WorkAuthors))
            ),
            "expected CompositeKey(WorkAuthors), got {violation:?}"
        );
        assert!(
            msg.contains("Duplicate work-author role assignment"),
            "message should include human_readable text, got: {msg}"
        );
    }

    #[test]
    fn enhance_detects_pk_by_sqlite_table_prefix() {
        // SQLite reports "UNIQUE constraint failed: table.col, ..." without the index name.
        let raw = "error returned from database: UNIQUE constraint failed: work_authors.work_id, work_authors.author_id, work_authors.role";
        let (msg, violation) = ConstraintViolation::enhance_db_err(db_err(raw));
        assert!(
            matches!(
                violation,
                Some(ConstraintViolation::CompositeKey(PrimaryKey::WorkAuthors))
            ),
            "expected CompositeKey(WorkAuthors) from SQLite message, got {violation:?}"
        );
        assert!(msg.contains("Duplicate work-author role assignment"));
    }

    #[test]
    fn enhance_detects_reading_list_book_sqlite() {
        let raw = "error returned from database: UNIQUE constraint failed: reading_list_book.reading_list_id, reading_list_book.edition_id";
        let (_, violation) = ConstraintViolation::enhance_db_err(db_err(raw));
        assert!(
            matches!(
                violation,
                Some(ConstraintViolation::CompositeKey(
                    PrimaryKey::ReadingListBook
                ))
            ),
            "expected CompositeKey(ReadingListBook), got {violation:?}"
        );
    }

    #[test]
    fn human_readable_delegates_to_inner() {
        assert_eq!(
            ConstraintViolation::ForeignKey(Constraint::WorksLanguage).human_readable(),
            Constraint::WorksLanguage.human_readable(),
        );
        assert_eq!(
            ConstraintViolation::CompositeKey(PrimaryKey::SeriesEntries).human_readable(),
            PrimaryKey::SeriesEntries.human_readable(),
        );
    }
}
