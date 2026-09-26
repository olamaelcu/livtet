//! Per-edition format metadata storage (`editions.format_metadata`).
//!
//! `m0011` adds a nullable JSON column validated at the application layer
//! against the edition format's `FormatMetadataSchema`. The column is
//! nullable because only some formats (audiobooks today) produce metadata.

use livtet_data::entities::{editions, works};
use livtet_data::orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use livtet_data::{Kind, TestDb};
use livtet_types::{DbId, FormatMetadataSchema};
use serde_json::json;
use time::PrimitiveDateTime;

fn now() -> PrimitiveDateTime {
    PrimitiveDateTime::new(
        time::Date::from_calendar_date(2026, time::Month::January, 1).unwrap(),
        time::Time::MIDNIGHT,
    )
}

async fn seed_work(db: &DatabaseConnection) -> works::Model {
    works::ActiveModel {
        id: Set(DbId::new()),
        title: Set("Test Work".into()),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap()
}

fn audiobook_metadata() -> serde_json::Value {
    json!({
        "duration_seconds": 7200,
        "chapters": [{"name": "Intro", "audio_start": 0, "audio_end": 300}],
    })
}

#[tokio::test]
async fn edition_format_metadata_round_trips() {
    let test_db = TestDb::new(&[Kind::Business]).await.unwrap();
    let db = test_db.state().db_conn();
    let work = seed_work(&db).await;

    let edition = editions::ActiveModel {
        id: Set(DbId::new()),
        work_id: Set(work.id),
        group_id: Set(None),
        title: Set(Some("Test Audiobook".into())),
        published_date: Set(None),
        format_id: Set(None),
        language_id: Set(None),
        notes: Set(None),
        description: Set(None),
        format_metadata: Set(Some(audiobook_metadata())),
        created_at: Set(now()),
        updated_at: Set(None),
    }
    .insert(&db)
    .await
    .expect("edition with format metadata inserts");

    let fetched = editions::Entity::find_by_id(edition.id)
        .one(&db)
        .await
        .expect("edition fetches")
        .expect("edition is present");
    let stored = fetched.format_metadata.expect("format metadata is stored");
    assert_eq!(stored, audiobook_metadata());
    FormatMetadataSchema::Audiobook
        .validate(&stored)
        .expect("stored format metadata validates against the audiobook schema");
}

#[tokio::test]
async fn edition_format_metadata_defaults_to_null() {
    let test_db = TestDb::new(&[Kind::Business]).await.unwrap();
    let db = test_db.state().db_conn();
    let work = seed_work(&db).await;

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
    .insert(&db)
    .await
    .expect("edition without format metadata inserts");

    let fetched = editions::Entity::find_by_id(edition.id)
        .one(&db)
        .await
        .expect("edition fetches")
        .expect("edition is present");
    assert!(
        fetched.format_metadata.is_none(),
        "editions without format metadata read back as none"
    );
}
