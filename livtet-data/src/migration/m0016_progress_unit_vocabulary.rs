//! One progress-unit vocabulary.
//!
//! `progress_unit` grew three dialects: the SQL default `ratio` (a 0.0–1.0
//! fraction), the reader-family tokens the desktop sync writes (`epub`, `pdf`,
//! `timestamp`), and the seeded `formats.progress_unit` column that was added
//! after the format rows existed — so it is `NULL` on every provisioned
//! database. `livtet_types::ProgressUnit` is the vocabulary every client and the
//! FFI already agree on, so this migration makes storage speak it too:
//!
//!   * `formats.progress_unit` is filled from
//!     [`KnownFormats::default_progress_unit`] where it is still `NULL`;
//!   * `reading_progress` / `reading_bookmarks` rows written in `ratio` (or
//!     `epub`, whose number was the same fraction) are scaled to percent and
//!     re-tagged `percentage`;
//!   * `pdf` page numbers are re-tagged `page`.
//!
//! The scale change is why this is not a rename: a `ratio` row of `0.42` means
//! 42%, so the number is multiplied before the unit changes. Rows are only
//! rescaled when they are still within 0.0–1.0, so already-percentage data that
//! was mislabelled `ratio` does not lose a factor of 100.

use livtet_types::{DbId, KnownFormats};
use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0016-progress-unit-vocabulary"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        apply(manager.get_connection()).await
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Deliberately empty, like the seed migration: a percentage of 42 is
        // indistinguishable from a ratio of 42 after the fact, so "unscaling"
        // would corrupt any row that was already percentage-valued.
        Ok(())
    }
}

/// Apply the vocabulary migration. Public so tests can run it against a
/// database they have seeded with legacy rows.
pub async fn apply<C>(db: &C) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    // 1. Formats: the column was added after these rows were seeded, so fill the
    //    gaps a format's own default. An explicit value always wins.
    for format in KnownFormats::all() {
        // Ids are 16-byte blobs, and both the unit (a `ProgressUnit` token) and
        // the hex id are generated here rather than user input, so a literal
        // statement needs no binding.
        let id_hex = DbId(format.ulid()).to_hex();
        db.execute_unprepared(&format!(
            "UPDATE formats SET progress_unit = '{}' WHERE id = x'{}' AND progress_unit IS NULL",
            format.default_progress_unit(),
            id_hex
        ))
        .await?;
    }

    // 2. Reading progress: rescale the fraction-valued units, then re-tag.
    for table in ["reading_progress", "reading_bookmarks"] {
        for (from, to, rescale) in [
            ("ratio", "percentage", true),
            ("epub", "percentage", true),
            ("pdf", "page", false),
        ] {
            if rescale {
                db.execute_unprepared(&format!(
                    "UPDATE {table} SET progress = progress * 100 \
                     WHERE progress_unit = '{from}' AND progress >= 0 AND progress <= 1"
                ))
                .await?;
            }
            db.execute_unprepared(&format!(
                "UPDATE {table} SET progress_unit = '{to}' WHERE progress_unit = '{from}'"
            ))
            .await?;
        }
    }

    Ok(())
}
