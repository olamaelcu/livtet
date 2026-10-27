//! Page-level bookmarks (`reading_bookmarks`).
//!
//! `m0015` adds a table apart from both `annotations` (a note carries text) and
//! `reading_progress` (a position is not a marker): a bookmark is just a
//! location the reader flagged, so it holds no content and no colour, and it
//! disappears with its edition.
//!
//! The position is encoded exactly like `reading_progress`: `progress` carries
//! the number, `progress_unit` names its unit, and `last_location` carries the
//! opaque locator. That one shape covers every reader family — EPUB locators,
//! PDF and physical page numbers, audiobook seconds.

use livtet_data::entities::{editions, reading_bookmarks, works};
use livtet_data::orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, Set,
};
use livtet_data::{Kind, TestDb};
use livtet_types::DbId;
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
        title: Set("Test Work".into()),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("work inserts");

    let edition = editions::ActiveModel {
        id: Set(DbId::new()),
        work_id: Set(work.id),
        group_id: Set(None),
        title: Set(Some("Test Ebook".into())),
        published_date: Set(None),
        format_id: Set(None),
        language_id: Set(None),
        notes: Set(None),
        description: Set(None),
        format_metadata: Set(None),
        created_at: Set(now()),
        updated_at: Set(None),
    }
    .insert(db)
    .await
    .expect("edition inserts");
    edition.id
}

async fn insert_bookmark(
    db: &DatabaseConnection,
    edition_id: DbId,
    progress: f64,
    unit: &str,
    last_location: Option<&str>,
) -> reading_bookmarks::Model {
    reading_bookmarks::ActiveModel {
        id: Set(DbId::new()),
        edition_id: Set(edition_id),
        progress: Set(progress),
        progress_unit: Set(Some(unit.to_string())),
        last_location: Set(last_location.map(str::to_string)),
        created_at: Set(now()),
    }
    .insert(db)
    .await
    .expect("bookmark inserts")
}

#[tokio::test]
async fn bookmarks_round_trip_per_edition() {
    let test_db = TestDb::new(&[Kind::Business]).await.unwrap();
    let db = test_db.state().db_conn();
    let edition_id = seed_edition(&db).await;
    let other_edition_id = seed_edition(&db).await;

    insert_bookmark(&db, edition_id, 3.0, "pdf", None).await;
    // EPUB bookmarks keep the serialized locator and store no numeric progress,
    // matching how `reading_progress` writes an EPUB position.
    let locator = r#"{"href":"ch1","locations":{"progression":0.25}}"#;
    let epub = insert_bookmark(&db, edition_id, 0.0, "epub", Some(locator)).await;
    insert_bookmark(&db, other_edition_id, 9.0, "pdf", None).await;

    let stored = reading_bookmarks::Entity::find()
        .filter(reading_bookmarks::Column::EditionId.eq(edition_id))
        .all(&db)
        .await
        .expect("bookmarks list");

    assert_eq!(stored.len(), 2, "only this edition's bookmarks are listed");
    let fetched = stored
        .iter()
        .find(|row| row.id == epub.id)
        .expect("the EPUB bookmark is listed");
    assert_eq!(fetched.progress_unit.as_deref(), Some("epub"));
    assert_eq!(fetched.last_location.as_deref(), Some(locator));
}

#[tokio::test]
async fn a_bookmark_carries_every_reader_family() {
    let test_db = TestDb::new(&[Kind::Business]).await.unwrap();
    let db = test_db.state().db_conn();
    let edition_id = seed_edition(&db).await;

    // One row per unit: page (PDF and physical books), seconds (audiobooks),
    // and the opaque locator (EPUB). Nothing about the table is book-format
    // specific.
    insert_bookmark(&db, edition_id, 42.0, "page", None).await;
    insert_bookmark(&db, edition_id, 900.5, "timestamp", Some("900.5")).await;
    insert_bookmark(&db, edition_id, 0.0, "epub", Some(r#"{"href":"ch2"}"#)).await;

    let units: Vec<String> = reading_bookmarks::Entity::find()
        .filter(reading_bookmarks::Column::EditionId.eq(edition_id))
        .all(&db)
        .await
        .expect("bookmarks list")
        .into_iter()
        .filter_map(|row| row.progress_unit)
        .collect();

    assert_eq!(units, ["page", "timestamp", "epub"]);
}

#[tokio::test]
async fn deleting_the_edition_cascades_to_its_bookmarks() {
    let test_db = TestDb::new(&[Kind::Business]).await.unwrap();
    let db = test_db.state().db_conn();
    let edition_id = seed_edition(&db).await;
    insert_bookmark(&db, edition_id, 1.0, "pdf", None).await;

    editions::Entity::delete_by_id(edition_id)
        .exec(&db)
        .await
        .expect("edition deletes");

    let remaining = reading_bookmarks::Entity::find()
        .filter(reading_bookmarks::Column::EditionId.eq(edition_id))
        .count(&db)
        .await
        .expect("bookmarks count");
    assert_eq!(remaining, 0, "bookmarks follow their edition");
}
