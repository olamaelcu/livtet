//! `SyncableEntityKind` — the registry mapping sea-ORM models to their
//! `change_log.entity_type` strings.
//!
//! Two things are checked, and they are deliberately separate.
//!
//! The round-trip tests drive every variant from real rows read through
//! sea-ORM, so `to_payload` / `from_payload` are exercised in the models'
//! own serde format. That covers all 21 arms of each dispatch table.
//!
//! The trigger-format test pins something else: payloads written by the
//! `change_log` triggers cannot be parsed by `from_payload` at all. See the
//! comment on that test.

use livtet_data::entities::{
    annotations, digital_inventory, edition_authors, edition_genres, edition_groups,
    edition_publishers, edition_subjects, edition_tags, editions, editions_loans, owned_edition,
    reading_list_book, reading_lists, reading_progress, series_entries, work_authors, work_genres,
    work_publishers, work_subjects, work_tags, works,
};
use livtet_data::orm::{DatabaseConnection, EntityTrait, SqlxSqliteConnector};
use livtet_sync::types::syncable_entity::{ChangeLogEntity, SyncableEntityKind};
use livtet_test_utils::sync::Device;

async fn seeded_conn(device_id: &str) -> (Device, DatabaseConnection) {
    let device = Device::seeded(device_id).await;
    let conn = SqlxSqliteConnector::from_sqlx_sqlite_pool(device.pool().clone());
    (device, conn)
}

/// Checks one variant: the registry strings, a non-empty entity id, and a
/// `to_payload` → `from_payload` round trip landing on the same entity.
macro_rules! check_variant {
    ($conn:expr, $variant:ident, $entity:ty, $entity_type:literal, $table:literal) => {{
        let rows = <$entity>::find()
            .all($conn)
            .await
            .unwrap_or_else(|e| panic!("query {}: {e}", $entity_type));
        assert!(
            !rows.is_empty(),
            "the seed must create {} rows for this to test anything",
            $entity_type
        );

        let kind = SyncableEntityKind::$variant(rows.into_iter().next().expect("one row"));
        assert_eq!(kind.entity_type_name(), $entity_type);
        assert_eq!(kind.table_name(), $table);
        assert!(
            !kind.entity_id().is_empty(),
            "{} produced a blank entity_id",
            $entity_type
        );

        let payload = kind
            .to_payload()
            .unwrap_or_else(|e| panic!("to_payload {}: {e}", $entity_type));
        let back = SyncableEntityKind::from_payload($entity_type, &payload)
            .unwrap_or_else(|e| panic!("from_payload {}: {e}", $entity_type));
        assert_eq!(back.entity_type_name(), kind.entity_type_name());
        assert_eq!(
            back.entity_id(),
            kind.entity_id(),
            "{} lost its identity through the round trip",
            $entity_type
        );
    }};
}

#[tokio::test]
async fn every_variant_round_trips_through_its_payload() {
    let (_device, conn) = seeded_conn("round-trip").await;
    let c = &conn;

    check_variant!(c, Work, works::Entity, "work", "works");
    check_variant!(c, Edition, editions::Entity, "edition", "editions");
    check_variant!(
        c,
        SeriesEntry,
        series_entries::Entity,
        "series_entry",
        "series_entries"
    );
    check_variant!(
        c,
        DigitalInventory,
        digital_inventory::Entity,
        "digital_inventory",
        "digital_inventory"
    );
    check_variant!(
        c,
        OwnedEdition,
        owned_edition::Entity,
        "owned_edition",
        "owned_editions"
    );
    check_variant!(
        c,
        EditionLoan,
        editions_loans::Entity,
        "edition_loan",
        "editions_loans"
    );
    check_variant!(
        c,
        Annotation,
        annotations::Entity,
        "annotation",
        "annotations"
    );
    check_variant!(
        c,
        ReadingList,
        reading_lists::Entity,
        "reading_list",
        "reading_lists"
    );
    check_variant!(
        c,
        ReadingListBook,
        reading_list_book::Entity,
        "reading_list_book",
        "reading_list_book"
    );
    check_variant!(
        c,
        ReadingProgress,
        reading_progress::Entity,
        "reading_progress",
        "reading_progress"
    );
    check_variant!(
        c,
        WorkAuthor,
        work_authors::Entity,
        "work_author",
        "work_authors"
    );
    check_variant!(c, WorkTag, work_tags::Entity, "work_tag", "work_tags");
    check_variant!(
        c,
        WorkGenre,
        work_genres::Entity,
        "work_genre",
        "work_genres"
    );
    check_variant!(
        c,
        WorkSubject,
        work_subjects::Entity,
        "work_subject",
        "work_subjects"
    );
    check_variant!(
        c,
        WorkPublisher,
        work_publishers::Entity,
        "work_publisher",
        "work_publishers"
    );
    check_variant!(
        c,
        EditionAuthor,
        edition_authors::Entity,
        "edition_author",
        "edition_authors"
    );
    check_variant!(
        c,
        EditionTag,
        edition_tags::Entity,
        "edition_tag",
        "edition_tags"
    );
    check_variant!(
        c,
        EditionGenre,
        edition_genres::Entity,
        "edition_genre",
        "edition_genres"
    );
    check_variant!(
        c,
        EditionSubject,
        edition_subjects::Entity,
        "edition_subject",
        "edition_subjects"
    );
    check_variant!(
        c,
        EditionPublisher,
        edition_publishers::Entity,
        "edition_publisher",
        "edition_publishers"
    );
    check_variant!(
        c,
        EditionGroup,
        edition_groups::Entity,
        "edition_group",
        "edition_groups"
    );
}

/// Pins a live defect rather than endorsing it.
///
/// `from_payload` cannot parse a single payload the `change_log` triggers
/// write. The triggers render a `DbId` as the 32-character hex of its
/// BINARY(16) column (`"01a120cd66cb12c9aa14921e46584eaf"`) and timestamps
/// as SQLite text (`"2025-01-01 00:00:00.0"`, `"2024-11-02"`), while the
/// models' serde impls expect a 26-character ULID string and `time`'s own
/// formats. Every one of the 21 entity types fails, with either
/// "invalid length" or "expected a `PlainDateTime`".
///
/// It has gone unnoticed because nothing calls this API: the sync engine
/// carries `SyncChange::payload` as an opaque `String` from `change_log`
/// straight to the wire and back, never materialising a model. So the
/// registry is reachable only from here.
///
/// Delete this test once the two formats agree -- whether by having the
/// triggers emit ULID strings and ISO timestamps, or by giving the sync
/// payloads their own DTOs.
#[tokio::test]
async fn trigger_payloads_do_not_parse_today() {
    let device = Device::seeded("trigger-format").await;
    let rows = device.change_log().await;
    assert!(!rows.is_empty(), "the seed must write change_log rows");

    for row in &rows {
        let payload: serde_json::Value = serde_json::from_str(&row.payload)
            .unwrap_or_else(|e| panic!("{} payload is not JSON: {e}", row.entity_type));
        assert!(
            SyncableEntityKind::from_payload(&row.entity_type, &payload).is_err(),
            "{} now parses from trigger output -- if the formats have been \
             reconciled, replace this test with a round trip over change_log",
            row.entity_type
        );
    }
}

/// The seed reaching every registered type is what makes the round-trip test
/// exhaustive. If a new entity is registered without the seed touching it,
/// this fails rather than silently leaving an arm untested.
#[tokio::test]
async fn the_seed_reaches_every_registered_entity_type() {
    let device = Device::seeded("breadth").await;
    let rows = device.change_log().await;

    let mut seen: Vec<&str> = rows.iter().map(|r| r.entity_type.as_str()).collect();
    seen.sort_unstable();
    seen.dedup();

    let missing: Vec<&str> = SyncableEntityKind::ALL_VARIANTS
        .iter()
        .copied()
        .filter(|v| !seen.contains(v))
        .collect();
    assert!(
        missing.is_empty(),
        "no change_log row exercises {missing:?}; extend full_coverage_seed"
    );

    let unregistered: Vec<&str> = seen
        .iter()
        .copied()
        .filter(|t| !SyncableEntityKind::ALL_VARIANTS.contains(t))
        .collect();
    assert!(
        unregistered.is_empty(),
        "the triggers log {unregistered:?}, which the registry does not know"
    );
}

#[test]
fn all_variants_is_unique() {
    let mut deduped = SyncableEntityKind::ALL_VARIANTS.to_vec();
    deduped.sort_unstable();
    deduped.dedup();
    assert_eq!(
        deduped.len(),
        SyncableEntityKind::ALL_VARIANTS.len(),
        "two variants claim the same entity_type"
    );
}

/// The `ChangeLogEntity` consts are written out by hand rather than generated
/// by the macro -- the module comment says why -- so they can drift from the
/// registry. Cross-check both directions.
#[test]
fn change_log_entity_consts_match_the_registry() {
    let consts: [&str; 21] = [
        <works::Entity as ChangeLogEntity>::ENTITY_TYPE,
        <editions::Entity as ChangeLogEntity>::ENTITY_TYPE,
        <series_entries::Entity as ChangeLogEntity>::ENTITY_TYPE,
        <digital_inventory::Entity as ChangeLogEntity>::ENTITY_TYPE,
        <owned_edition::Entity as ChangeLogEntity>::ENTITY_TYPE,
        <editions_loans::Entity as ChangeLogEntity>::ENTITY_TYPE,
        <annotations::Entity as ChangeLogEntity>::ENTITY_TYPE,
        <reading_lists::Entity as ChangeLogEntity>::ENTITY_TYPE,
        <reading_list_book::Entity as ChangeLogEntity>::ENTITY_TYPE,
        <reading_progress::Entity as ChangeLogEntity>::ENTITY_TYPE,
        <work_authors::Entity as ChangeLogEntity>::ENTITY_TYPE,
        <work_tags::Entity as ChangeLogEntity>::ENTITY_TYPE,
        <work_genres::Entity as ChangeLogEntity>::ENTITY_TYPE,
        <work_subjects::Entity as ChangeLogEntity>::ENTITY_TYPE,
        <work_publishers::Entity as ChangeLogEntity>::ENTITY_TYPE,
        <edition_authors::Entity as ChangeLogEntity>::ENTITY_TYPE,
        <edition_tags::Entity as ChangeLogEntity>::ENTITY_TYPE,
        <edition_genres::Entity as ChangeLogEntity>::ENTITY_TYPE,
        <edition_subjects::Entity as ChangeLogEntity>::ENTITY_TYPE,
        <edition_publishers::Entity as ChangeLogEntity>::ENTITY_TYPE,
        <edition_groups::Entity as ChangeLogEntity>::ENTITY_TYPE,
    ];

    let mut from_consts = consts.to_vec();
    from_consts.sort_unstable();
    let mut from_registry = SyncableEntityKind::ALL_VARIANTS.to_vec();
    from_registry.sort_unstable();
    assert_eq!(
        from_consts, from_registry,
        "the hand-written ChangeLogEntity impls and the macro registry disagree"
    );
}

#[test]
fn from_payload_rejects_an_unknown_entity_type() {
    let err = SyncableEntityKind::from_payload("not_an_entity", &serde_json::json!({}))
        .expect_err("an unregistered entity type must not parse");
    assert!(
        err.to_string().contains("not_an_entity"),
        "the error must name the type it did not recognise: {err}"
    );
}

#[test]
fn from_payload_rejects_a_payload_of_the_wrong_shape() {
    let err = SyncableEntityKind::from_payload("work", &serde_json::json!({"nope": true}))
        .expect_err("a mismatched payload must not parse");
    assert!(
        err.to_string().contains("work"),
        "the error must name the entity type: {err}"
    );
}
