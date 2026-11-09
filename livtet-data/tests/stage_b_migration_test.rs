//! Stage B schema: legacy note backfill and tombstone filtering.

use livtet_data::entities::{editions, formats, reading_annotations, reading_progress, works};
use livtet_data::migration::m0022_reading_progress_stage_b::backfill;
use livtet_data::orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use livtet_data::{Kind, TestDb};
use livtet_types::DbId;
use time::PrimitiveDateTime;

fn legacy_time() -> PrimitiveDateTime {
    PrimitiveDateTime::new(
        time::Date::from_calendar_date(2026, time::Month::January, 1).unwrap(),
        time::Time::MIDNIGHT,
    )
}

async fn seed_edition(db: &DatabaseConnection) -> (DbId, DbId) {
    let work = works::ActiveModel {
        id: Set(DbId::new()),
        title: Set("Stage B Work".into()),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("work inserts");
    let edition = editions::ActiveModel {
        id: Set(DbId::new()),
        work_id: Set(work.id),
        title: Set(Some("Stage B Edition".into())),
        created_at: Set(legacy_time()),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("edition inserts");
    let format = formats::Entity::find()
        .all(db)
        .await
        .expect("formats")
        .into_iter()
        .next()
        .expect("a seeded format")
        .id;
    (edition.id, format)
}

#[tokio::test]
async fn backfill_stamps_and_sorts_legacy_notes_and_hides_tombstones() {
    let test_db = TestDb::new(&[Kind::Business, Kind::Client])
        .await
        .expect("test db");
    let db = test_db.state().db_conn();
    let (edition_id, format_id) = seed_edition(&db).await;

    reading_progress::ActiveModel {
        id: Set(DbId::new()),
        edition_id: Set(edition_id),
        format_id: Set(format_id),
        progress: Set(42.0),
        progress_unit: Set(Some("percentage".into())),
        last_location: Set(None),
        total_reading_time_secs: Set(0),
        annotations: Set(Some(
            r#"[{"id":"b","text":"two"},{"id":"c","text":"gone","deletedAt":"2026-01-01T00:00:00Z"},{"id":"a","text":"one"}]"#
                .into(),
        )),
        created_at: Set(legacy_time()),
        updated_at: Set(None),
        updated_by: Set(None),
    }
    .insert(&db)
    .await
    .expect("legacy progress inserts");

    backfill(&db).await.expect("backfill");

    let row = reading_progress::Entity::find()
        .one(&db)
        .await
        .expect("query")
        .expect("row");
    assert_eq!(row.updated_at.as_deref(), Some("2026-01-01T00:00:00.000Z"));
    assert_eq!(row.updated_by.as_deref(), Some("legacy"));

    let notes: Vec<serde_json::Value> =
        serde_json::from_str(row.annotations.as_deref().expect("notes")).expect("json");
    assert_eq!(notes.len(), 3);
    assert_eq!(
        notes
            .iter()
            .map(|note| note["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    for note in &notes {
        assert_eq!(note["updatedAt"], "1970-01-01T00:00:00Z");
        assert_eq!(note["updatedBy"], "legacy");
    }
    assert_eq!(notes[2]["deletedAt"], "2026-01-01T00:00:00Z");

    // The reader view omits the tombstone.
    let visible = reading_annotations::Entity::find()
        .all(&db)
        .await
        .expect("view");
    assert_eq!(visible.len(), 2);
    assert_eq!(visible[0].id, "a");
    assert_eq!(visible[1].id, "b");
}
