//! Structured author name parts.
//!
//! `authors` stores one joined `name` string, but citation styles need the
//! name split into a family (last) part and a given (fore) part — CSL-JSON's
//! `family` / `given`, from which BibTeX and RIS are rendered — and every row
//! written before this migration only has the joined form. So `family` and
//! `given` are added as nullable TEXT and derived from `name`:
//!
//!   * a comma form `"Family, Given"` splits on the **first** comma: the text
//!     before it is `family`, everything after it is `given` (so `"Doe, Jr.,
//!     John"` yields family `Doe` and given `Jr., John`);
//!   * otherwise the **final** whitespace-separated token is `family` and the
//!     tokens before it, joined with single spaces, are `given` ("Ursula K.
//!     Le Guin" yields family `Le Guin`, given `Ursula K.`);
//!     a surname particle immediately before the final token (`Le`, `van der`,
//!     `de la`) belongs to `family`, up to three words in total, and only when
//!     at least one token would remain for `given` — so `"Van Morrison"` keeps
//!     `Morrison` as the family name;
//!   * a single-token name sets `family` and leaves `given` `NULL`;
//!   * parts are trimmed, and an empty part is stored as `NULL` rather than
//!     `''` (so `"Homer,"` carries no given part).
//!
//! Both columns stay nullable because the split is lossy: an import that knows
//! the real structure (or a mononym) has nothing to put in `given`. The
//! backfill never overwrites a non-`NULL` part — an import-supplied value
//! always wins, and re-running [`apply`] is idempotent.

use livtet_types::DbId;
use sea_orm::{Statement, Value};
use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0019-author-name-parts"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        // `TEXT` is STRICT-legal; existing rows read back as NULL until the
        // backfill below fills them.
        conn.execute_unprepared("ALTER TABLE authors ADD COLUMN family TEXT")
            .await?;
        conn.execute_unprepared("ALTER TABLE authors ADD COLUMN given TEXT")
            .await?;
        apply(conn).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // `name` is untouched, so the derived split can always be rebuilt by
        // the next `up`. Parts supplied by an import *after* this migration ran
        // are dropped with the columns and have to be re-imported.
        let conn = manager.get_connection();
        conn.execute_unprepared("ALTER TABLE authors DROP COLUMN given")
            .await?;
        conn.execute_unprepared("ALTER TABLE authors DROP COLUMN family")
            .await?;
        Ok(())
    }
}

/// Apply the name-part backfill. Public so tests — and any importer that wrote
/// joined names after the migration ran — can run it again; it only ever fills
/// `NULL` parts.
pub async fn apply<C>(db: &C) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let rows = db
        .query_all_raw(Statement::from_string(
            db.get_database_backend(),
            "SELECT id, name, family, given FROM authors",
        ))
        .await?;

    for row in rows {
        let id: DbId = row.try_get("", "id")?;
        let name: String = row.try_get("", "name")?;
        let family: Option<String> = row.try_get("", "family")?;
        let given: Option<String> = row.try_get("", "given")?;

        // Both parts already known: nothing to derive, and skipping keeps the
        // write count down on a database that has been imported into.
        if family.is_some() && given.is_some() {
            continue;
        }

        let (derived_family, derived_given) = split_name(&name);
        if derived_family.is_none() && derived_given.is_none() {
            // A blank name derives nothing; leave the row alone.
            continue;
        }

        // `COALESCE` is the "never overwrite" rule: a part an import already
        // supplied wins over the derived one.
        db.execute_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            "UPDATE authors SET family = COALESCE(family, $1), \
             given = COALESCE(given, $2) WHERE id = $3",
            [
                Value::String(derived_family),
                Value::String(derived_given),
                id.into(),
            ],
        ))
        .await?;
    }

    Ok(())
}

/// Split a joined author name into `(family, given)`.
///
/// The rule is the one documented on this module: first comma when there is
/// one, otherwise the final whitespace token is the family name. Returns
/// `(None, None)` for a blank name and `(Some(family), None)` for a mononym.
pub fn split_name(name: &str) -> (Option<String>, Option<String>) {
    let name = name.trim();
    if name.is_empty() {
        return (None, None);
    }

    if let Some((family, given)) = name.split_once(',') {
        return (non_empty(family), non_empty(given));
    }

    let tokens: Vec<&str> = name.split_whitespace().collect();
    // The family name begins at the last token and grows leftwards over a
    // particle ("Le", "van der", "de la"), which belongs to the surname. Three
    // words is the ceiling, and at least one token must remain for `given`:
    // otherwise "Van Morrison" would lose its given name to a particle rule.
    let mut start = tokens.len() - 1;
    while start > 1 && tokens.len() - start < 3 && is_particle(tokens[start - 1]) {
        start -= 1;
    }

    let given = (start > 0).then(|| tokens[..start].join(" "));
    (Some(tokens[start..].join(" ")), given)
}

/// Whether `token` is a surname particle rather than a given name.
fn is_particle(token: &str) -> bool {
    const PARTICLES: &[&str] = &[
        "al", "ap", "ben", "bin", "da", "dal", "de", "del", "della", "den", "der", "di", "do",
        "dos", "du", "el", "ibn", "la", "le", "st", "ten", "ter", "van", "von", "y", "zu", "zum",
        "zur",
    ];
    let lower = token.trim_end_matches('.').to_lowercase();
    PARTICLES.contains(&lower.as_str())
}

/// Trim a part and treat an empty one as absent.
fn non_empty(part: &str) -> Option<String> {
    let part = part.trim();
    (!part.is_empty()).then(|| part.to_string())
}
