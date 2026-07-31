use strum::{EnumIter, EnumString, IntoEnumIterator};

/// Single-column UNIQUE indexes that enforce a 1:1 cardinality the
/// rest of the codebase already assumes.
///
/// Extracted from [`crate::PrimaryKey`] because these are semantically
/// different constraints — a composite primary key on a junction table
/// (the domain of `PrimaryKey`) is not the same thing as a
/// single-column UNIQUE index (`UniqueIndex`).  The old `PrimaryKey`
/// enum mixed both, which made `strum(prefix = "pk_")`'s `Display`
/// output misleading for the UNIQUE variant.
///
/// Plain single-column primary keys (e.g. `pk_db_id(…)`) use SeaORM's
/// built-in PK machinery and need no named index here.
#[derive(Copy, Clone, Debug, Eq, PartialEq, EnumString, EnumIter)]
pub enum UniqueIndex {
    // ── m0011_digital_inventory_unique_edition ────────────────────
    DigitalInventoryEdition,
}

impl UniqueIndex {
    /// Human-readable description of what a violation of this unique
    /// index means.  Used by [`crate::ConstraintViolation::enhance_db_err`].
    pub fn human_readable(self) -> &'static str {
        match self {
            Self::DigitalInventoryEdition => {
                "Another digital inventory row already exists for this edition"
            }
        }
    }

    /// Substrings used to identify this unique-index violation in a
    /// database error message.
    ///
    /// Two patterns are provided per variant:
    /// - The DDL index name (e.g. `"uq_digital_inventory_edition_id"`)
    ///   — matched by databases such as PostgreSQL that embed index
    ///   names in violation messages.
    /// - The table.column prefix (e.g. `"digital_inventory."`) —
    ///   matched by SQLite, which reports `"UNIQUE constraint failed:
    ///   digital_inventory.edition_id"` without the index name.
    pub fn patterns(self) -> &'static [&'static str] {
        match self {
            Self::DigitalInventoryEdition => {
                &["uq_digital_inventory_edition_id", "digital_inventory."]
            }
        }
    }

    /// Returns an iterator over every `UniqueIndex` variant in
    /// declaration order.  Used by [`crate::ConstraintViolation::enhance_db_err`].
    pub fn all() -> impl Iterator<Item = Self> {
        Self::iter()
    }
}
