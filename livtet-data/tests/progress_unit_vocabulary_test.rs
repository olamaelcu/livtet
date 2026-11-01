//! One progress-unit vocabulary (`m0016`).
//!
//! Three dialects met in these two tables: the SQL default `ratio` (a 0.0–1.0
//! fraction), the reader-family tokens the desktop sync writes (`epub`, `pdf`,
//! `timestamp`), and `formats.progress_unit`, which was added *after* the
//! format rows were seeded and is therefore `NULL` everywhere. `m0016` fills the
//! format column from [`KnownFormats::default_progress_unit`] and folds the
//! retired spellings onto `livtet_types::ProgressUnit`, rescaling the
//! fraction-valued ones so `0.42` becomes `42`.

use livtet_data::entities::{editions, formats, reading_bookmarks, reading_progress, works};
use livtet_data::migration::m0016_progress_unit_vocabulary::apply;
use livtet_data::orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use livtet_data::{Kind, TestDb};
use livtet_types::{DbId, KnownFormats};
use time::PrimitiveDateTime;

fn now() -> PrimitiveDateTime {
    PrimitiveDateTime::new(
        time::Date::from_calendar_date(2026, time::Month::January, 1).unwrap(),
        time::Time::MIDNIGHT,
    )
}

async fn seed_edition(db: &DatabaseConnection) -> DbId {
    let work = works::ActiveModel {
        id: Set(DbId::new()),
        title: Set("Vocabulary Work".into()),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("work inserts");

    let edition = editions::ActiveModel {
        id: Set(DbId::new()),
        work_id: Set(work.id),
        title: Set(Some("Vocabulary Edition".into())),
        created_at: Set(now()),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("edition inserts");
    edition.id
}

async fn format_id(db: &DatabaseConnection, format: KnownFormats) -> DbId {
    formats::Entity::find()
        .all(db)
        .await
        .expect("formats list")
        .into_iter()
        .find(|row| row.name == format.name())
        .expect("seeded format exists")
        .id
}

/// Write a row in one of the retired dialects, the way an older client did.
async fn insert_progress(
    db: &DatabaseConnection,
    edition_id: DbId,
    format_id: DbId,
    progress: f64,
    unit: &str,
) {
    reading_progress::ActiveModel {
        id: Set(DbId::new()),
        edition_id: Set(edition_id),
        format_id: Set(format_id),
        progress: Set(progress),
        progress_unit: Set(Some(unit.to_string())),
        last_location: Set(None),
        total_reading_time_secs: Set(0),
        created_at: Set(now()),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("progress inserts");
}

async fn insert_bookmark(db: &DatabaseConnection, edition_id: DbId, progress: f64, unit: &str) {
    reading_bookmarks::ActiveModel {
        id: Set(DbId::new()),
        edition_id: Set(edition_id),
        progress: Set(progress),
        progress_unit: Set(Some(unit.to_string())),
        last_location: Set(None),
        created_at: Set(now()),
    }
    .insert(db)
    .await
    .expect("bookmark inserts");
}

async fn progress_rows(db: &DatabaseConnection) -> Vec<(i64, Option<String>)> {
    let mut rows: Vec<(i64, Option<String>)> = reading_progress::Entity::find()
        .all(db)
        .await
        .expect("progress rows")
        .into_iter()
        .map(|row| (row.progress as i64, row.progress_unit))
        .collect();
    rows.sort();
    rows
}

async fn bookmark_rows(db: &DatabaseConnection) -> Vec<(i64, Option<String>)> {
    let mut rows: Vec<(i64, Option<String>)> = reading_bookmarks::Entity::find()
        .all(db)
        .await
        .expect("bookmark rows")
        .into_iter()
        .map(|row| (row.progress as i64, row.progress_unit))
        .collect();
    rows.sort();
    rows
}

#[tokio::test]
async fn every_format_carries_its_default_unit() {
    let test_db = TestDb::new(&[Kind::Business]).await.unwrap();
    let db = test_db.state().db_conn();

    let stored = formats::Entity::find()
        .all(&db)
        .await
        .expect("formats list");
    assert!(!stored.is_empty(), "the format seeds ran");

    for format in KnownFormats::all() {
        let row = stored
            .iter()
            .find(|row| row.name == format.name())
            .unwrap_or_else(|| panic!("{format:?} is seeded"));
        assert_eq!(
            row.progress_unit.as_deref(),
            Some(format.default_progress_unit()),
            "{format:?} should carry its default unit, not NULL"
        );
    }
}

#[tokio::test]
async fn retired_units_are_folded_onto_progress_unit() {
    let test_db = TestDb::new(&[Kind::Business]).await.unwrap();
    let db = test_db.state().db_conn();
    let edition_id = seed_edition(&db).await;
    // Progress is unique per (edition, format), so each dialect gets its own
    // format the way three different readers would.
    let audiobook = format_id(&db, KnownFormats::Audiobook).await;
    let epub = format_id(&db, KnownFormats::Epub).await;
    let pdf = format_id(&db, KnownFormats::Pdf).await;

    // A fraction-valued row, an EPUB locator row (its number was the same
    // fraction) and a PDF page row, as the desktop sync wrote them.
    insert_progress(&db, edition_id, audiobook, 0.42, "ratio").await;
    insert_progress(&db, edition_id, epub, 0.25, "epub").await;
    insert_progress(&db, edition_id, pdf, 7.0, "pdf").await;
    insert_bookmark(&db, edition_id, 0.5, "ratio").await;
    insert_bookmark(&db, edition_id, 12.0, "pdf").await;

    apply(&db).await.expect("migration applies");

    assert_eq!(
        progress_rows(&db).await,
        vec![
            (7, Some("page".to_string())),
            (25, Some("percentage".to_string())),
            (42, Some("percentage".to_string())),
        ],
        "fractions are scaled to percent, pages are only re-tagged"
    );
    assert_eq!(
        bookmark_rows(&db).await,
        vec![
            (12, Some("page".to_string())),
            (50, Some("percentage".to_string())),
        ]
    );
}

#[tokio::test]
async fn an_already_percentage_labelled_ratio_is_not_rescaled() {
    let test_db = TestDb::new(&[Kind::Business]).await.unwrap();
    let db = test_db.state().db_conn();
    let edition_id = seed_edition(&db).await;
    let audiobook = format_id(&db, KnownFormats::Audiobook).await;

    // Older rows sometimes stored a percentage under the legacy token.
    insert_progress(&db, edition_id, audiobook, 61.0, "ratio").await;

    apply(&db).await.expect("migration applies");

    assert_eq!(
        progress_rows(&db).await,
        vec![(61, Some("percentage".to_string()))],
        "a value outside 0.0-1.0 was already a percentage and keeps its number"
    );
}

#[tokio::test]
async fn applying_twice_changes_nothing() {
    let test_db = TestDb::new(&[Kind::Business]).await.unwrap();
    let db = test_db.state().db_conn();
    let edition_id = seed_edition(&db).await;
    let audiobook = format_id(&db, KnownFormats::Audiobook).await;

    insert_progress(&db, edition_id, audiobook, 0.3, "ratio").await;
    apply(&db).await.expect("first apply");
    let first = progress_rows(&db).await;
    apply(&db).await.expect("second apply");

    assert_eq!(
        progress_rows(&db).await,
        first,
        "the migration is idempotent"
    );
}

#[tokio::test]
async fn an_explicit_format_unit_survives_the_backfill() {
    let test_db = TestDb::new(&[Kind::Business]).await.unwrap();
    let db = test_db.state().db_conn();

    // A reader or import that set the unit themselves must not be overwritten.
    let ebook = format_id(&db, KnownFormats::Ebook).await;
    let mut row: formats::ActiveModel = formats::Entity::find_by_id(ebook)
        .one(&db)
        .await
        .expect("format lookup")
        .expect("ebook format")
        .into();
    row.progress_unit = Set(Some("chapter".to_string()));
    row.update(&db).await.expect("format unit set");

    apply(&db).await.expect("migration applies");

    let stored = formats::Entity::find_by_id(ebook)
        .one(&db)
        .await
        .expect("format lookup")
        .expect("ebook format");
    assert_eq!(stored.progress_unit.as_deref(), Some("chapter"));
}
