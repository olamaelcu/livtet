//! Publication place on edition/publisher links (`m0018`).
//!
//! Citation styles print the place of publication beside the publisher, so
//! `m0018` adds `edition_publishers.place`. There is deliberately no backfill —
//! nothing on disk holds a place — so every link written before the migration
//! reads back `NULL` until its import is rerun.

use livtet_data::entities::{edition_publishers, editions, publishers, works};
use livtet_data::orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
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
        title: Set("Publication Place Work".into()),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("work inserts");

    editions::ActiveModel {
        id: Set(DbId::new()),
        work_id: Set(work.id),
        title: Set(Some("Publication Place Edition".into())),
        created_at: Set(now()),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("edition inserts")
    .id
}

async fn seed_publisher(db: &DatabaseConnection, name: &str) -> DbId {
    publishers::ActiveModel {
        id: Set(DbId::new()),
        name: Set(name.into()),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("publisher inserts")
    .id
}

#[tokio::test]
async fn place_starts_null() {
    let test_db = TestDb::new(&[Kind::Business]).await.unwrap();
    let db = test_db.state().db_conn();

    let edition_id = seed_edition(&db).await;
    let publisher_id = seed_publisher(&db, "Tor").await;

    edition_publishers::ActiveModel {
        edition_id: Set(edition_id),
        publisher_id: Set(publisher_id),
        ..Default::default()
    }
    .insert(&db)
    .await
    .expect("link inserts");

    let link = edition_publishers::Entity::find_by_id((edition_id, publisher_id))
        .one(&db)
        .await
        .expect("link fetches")
        .expect("link is present");
    assert_eq!(
        link.place, None,
        "a link written before m0018 has no place to report"
    );
}

#[tokio::test]
async fn place_is_writable_once_an_import_supplies_it() {
    let test_db = TestDb::new(&[Kind::Business]).await.unwrap();
    let db = test_db.state().db_conn();

    let edition_id = seed_edition(&db).await;
    let publisher_id = seed_publisher(&db, "Tor").await;

    edition_publishers::ActiveModel {
        edition_id: Set(edition_id),
        publisher_id: Set(publisher_id),
        place: Set(Some("London".into())),
        ..Default::default()
    }
    .insert(&db)
    .await
    .expect("link inserts");

    let link = edition_publishers::Entity::find_by_id((edition_id, publisher_id))
        .one(&db)
        .await
        .expect("link fetches")
        .expect("link is present");
    assert_eq!(
        link.place.as_deref(),
        Some("London"),
        "the place is stored on the edition/publisher link"
    );
}
