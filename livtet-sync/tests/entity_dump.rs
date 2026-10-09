//! `dump_all`, the `SyncedEntity` wrapper, and the `SyncableEntity` trait.
//!
//! These are the parts of `syncable_entity.rs` the registry round-trip tests
//! do not reach: the full-dump fetch, the change-log metadata wrapper, and a
//! trait whose default methods have no implementor in the tree.

use livtet_data::entities::works;
use livtet_data::orm::SqlxSqliteConnector;
use livtet_sync::SyncChange;
use livtet_sync::types::syncable_entity::{
    ENTITY_DUMP_TYPES, SyncableEntityKind, SyncedEntity, entity_type_to_table,
};
use livtet_test_utils::sync::Device;

// ─── dump_all ───────────────────────────────────────────────────────────────

/// Every axis of `EntityDump` is fetched through a typed sea-ORM query, so a
/// populated database must come back with rows on the axes it seeded -- and
/// the ids must survive as ULID strings rather than being dropped, which is
/// the reason `fetch_all_json` exists instead of raw SQL.
#[tokio::test]
async fn dump_all_fetches_every_axis_with_usable_ids() {
    let device = Device::seeded("dump").await;
    let conn = SqlxSqliteConnector::from_sqlx_sqlite_pool(device.pool().clone());

    let dump = SyncableEntityKind::dump_all(&conn)
        .await
        .expect("dump_all over a seeded database");

    // The seed always creates works and editions; the rest are optional, so
    // only these two are asserted non-empty.
    assert!(!dump.works.is_empty(), "the seed creates works");
    assert!(!dump.editions.is_empty(), "the seed creates editions");

    let work = &dump.works[0];
    let id = work
        .get("id")
        .and_then(|v| v.as_str())
        .expect("a work's id must serialise as a string");
    assert_eq!(
        id.len(),
        26,
        "a DbId must survive as a 26-char ULID string, not a dropped byte array: {id:?}"
    );
    assert!(
        work.get("title").is_some(),
        "the model's own fields must be present: {work}"
    );
}

#[tokio::test]
async fn dump_all_over_an_empty_database_yields_empty_axes() {
    let device = Device::new("dump-empty").await;
    let conn = SqlxSqliteConnector::from_sqlx_sqlite_pool(device.pool().clone());

    let dump = SyncableEntityKind::dump_all(&conn)
        .await
        .expect("dump_all over an empty database must succeed, not error");
    assert!(dump.works.is_empty());
    assert!(dump.editions.is_empty());
    assert!(dump.reading_progress.is_empty());
}

/// `ENTITY_DUMP_TYPES` names the axes `dump_all` builds, so each has to be a
/// registered entity type with a known table.
#[test]
fn entity_dump_types_are_all_registered() {
    for entity_type in ENTITY_DUMP_TYPES {
        assert!(
            SyncableEntityKind::ALL_VARIANTS.contains(entity_type),
            "{entity_type} is dumped but not registered"
        );
        assert!(
            entity_type_to_table(entity_type).is_some(),
            "{entity_type} is dumped but has no table mapping"
        );
    }
}

#[test]
fn entity_type_to_table_refuses_an_unknown_type() {
    assert_eq!(entity_type_to_table("widget"), None);
    assert_eq!(entity_type_to_table(""), None);
}

// ─── SyncedEntity ───────────────────────────────────────────────────────────

#[tokio::test]
async fn synced_entity_carries_the_kind_and_its_metadata() {
    use livtet_data::orm::EntityTrait;

    let device = Device::seeded("synced").await;
    let conn = SqlxSqliteConnector::from_sqlx_sqlite_pool(device.pool().clone());
    let work = works::Entity::find()
        .all(&conn)
        .await
        .expect("works")
        .into_iter()
        .next()
        .expect("the seed creates works");

    let kind = SyncableEntityKind::Work(work);
    let expected_id = kind.entity_id();
    let synced = SyncedEntity::new(kind, 42);

    assert_eq!(synced.version, 42);
    assert_eq!(synced.entity_type, "work");
    assert_eq!(synced.entity_id, expected_id);
    assert_eq!(
        synced.payload.get("id").and_then(|v| v.as_str()),
        Some(expected_id.as_str()),
        "the payload must be the serialised model, keyed by the same id"
    );
}

/// `from_change` is the inbound path: a `SyncChange` row off the wire, whose
/// payload is a JSON *string* rather than a value.
#[tokio::test]
async fn synced_entity_from_change_parses_the_payload_string() {
    use livtet_data::orm::EntityTrait;

    let device = Device::seeded("from-change").await;
    let conn = SqlxSqliteConnector::from_sqlx_sqlite_pool(device.pool().clone());
    let work = works::Entity::find()
        .all(&conn)
        .await
        .expect("works")
        .into_iter()
        .next()
        .expect("the seed creates works");

    let kind = SyncableEntityKind::Work(work);
    let payload = kind.to_payload().expect("to_payload");
    let change = SyncChange {
        id: 1,
        entity_type: "work".to_string(),
        entity_id: kind.entity_id(),
        operation: "update".to_string(),
        version: 9,
        payload: payload.to_string(),
        changed_at: "2026-01-01T00:00:00Z".to_string(),
        device_id: "remote".to_string(),
    };

    let synced = SyncedEntity::from_change(&change).expect("from_change");
    assert_eq!(synced.version, 9);
    assert_eq!(synced.entity_type, "work");
    assert_eq!(synced.entity_id, change.entity_id);
    assert_eq!(synced.kind.entity_type_name(), "work");
}

#[test]
fn synced_entity_from_change_rejects_a_payload_that_is_not_json() {
    let change = SyncChange {
        id: 1,
        entity_type: "work".to_string(),
        entity_id: "whatever".to_string(),
        operation: "update".to_string(),
        version: 1,
        payload: "{not json".to_string(),
        changed_at: "2026-01-01T00:00:00Z".to_string(),
        device_id: "remote".to_string(),
    };
    let err = SyncedEntity::from_change(&change).expect_err("malformed payload");
    assert!(err.to_string().contains("work"), "got {err}");
}

#[test]
fn synced_entity_from_change_rejects_an_unknown_entity_type() {
    let change = SyncChange {
        id: 1,
        entity_type: "widget".to_string(),
        entity_id: "whatever".to_string(),
        operation: "update".to_string(),
        version: 1,
        payload: "{}".to_string(),
        changed_at: "2026-01-01T00:00:00Z".to_string(),
        device_id: "remote".to_string(),
    };
    let err = SyncedEntity::from_change(&change).expect_err("unknown entity type");
    assert!(err.to_string().contains("widget"), "got {err}");
}

// The `SyncableEntity` trait's default methods are covered by the in-crate
// tests in `src/types/syncable_entity.rs`: the trait is local to this crate,
// so the orphan rule only allows implementing it for a foreign model there.
