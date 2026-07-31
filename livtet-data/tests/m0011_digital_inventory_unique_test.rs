//! Schema-invariant guards for `m0011_digital_inventory_unique_edition`.
//!
//! `digital_inventory` is treated as 1:1 with `editions` by the seed
//! and by the OPDS server's `HashMap<DbId, Model>` consumer, but the
//! pre-m0011 schema allowed N:1. Migration `m0011` adds a UNIQUE index
//! on `digital_inventory.edition_id` to enforce the 1:1 intent at the
//! storage layer. These tests pin that invariant.
//!
//! The schema-invariant guard pattern mirrors
//! `tauri/src/commands/edition.rs::{duplicate_junction_pair_is_rejected,
//! identifiers_value_is_unique}` in the desktop repo.

use livtet_data::entities::{digital_inventory, editions, works};
use livtet_data::orm::{ActiveModelTrait, DatabaseConnection, Set};
use livtet_data::TestDb;
use livtet_types::DbId;
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

async fn seed_edition(db: &DatabaseConnection, work_id: DbId) -> editions::Model {
    editions::ActiveModel {
        id: Set(DbId::new()),
        work_id: Set(work_id),
        group_id: Set(None),
        title: Set(Some("Test Edition".into())),
        published_date: Set(None),
        format_id: Set(None),
        language_id: Set(None),
        notes: Set(None),
        description: Set(None),
        created_at: Set(now()),
        updated_at: Set(None),
    }
    .insert(db)
    .await
    .unwrap()
}

fn digital_inventory_row(
    id: DbId,
    edition_id: DbId,
) -> digital_inventory::ActiveModel {
    digital_inventory::ActiveModel {
        id: Set(id),
        edition_id: Set(edition_id),
        file_path: Set(Some(format!("/tmp/{id}.epub"))),
        cover_path: Set(None),
        blurhash: Set(None),
        dominant_color: Set(None),
        file_hash: Set(None),
        file_size_bytes: Set(None),
        notes: Set(None),
        added_at: Set(now()),
        updated_at: Set(None),
    }
}

/// Schema-invariant guard: `digital_inventory.edition_id` is UNIQUE.
/// A second row referencing the same edition must be rejected.
#[tokio::test]
async fn digital_inventory_edition_id_is_unique() {
    let test_db = TestDb::new(None).await.unwrap();
    let db = test_db.state().db_conn();
    let work = seed_work(&db).await;
    let edition = seed_edition(&db, work.id).await;

    digital_inventory_row(DbId::new(), edition.id)
        .insert(&db)
        .await
        .expect("first insert for an edition must succeed");

    let dup = digital_inventory_row(DbId::new(), edition.id)
        .insert(&db)
        .await;
    assert!(
        dup.is_err(),
        "digital_inventory must reject a second row for the same edition_id"
    );
}

/// Sanity companion to the negative test above: two distinct edition_ids
/// must both succeed, since the constraint is single-column and only
/// fires on duplicates.
#[tokio::test]
async fn digital_inventory_distinct_edition_ids_succeed() {
    let test_db = TestDb::new(None).await.unwrap();
    let db = test_db.state().db_conn();
    let work = seed_work(&db).await;
    let edition_a = seed_edition(&db, work.id).await;
    let edition_b = seed_edition(&db, work.id).await;

    digital_inventory_row(DbId::new(), edition_a.id)
        .insert(&db)
        .await
        .expect("first edition must accept a digital_inventory row");

    digital_inventory_row(DbId::new(), edition_b.id)
        .insert(&db)
        .await
        .expect("second edition must accept its own digital_inventory row");
}
