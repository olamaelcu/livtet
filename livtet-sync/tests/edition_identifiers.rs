//! An edition's identifiers travel as a set of value-keyed links.
//!
//! Every device mints its own `identifiers.id`, so the change log keys a link by
//! the identifier's *value* and the apply path resolves it locally. These tests
//! pin the three cases that legacy id-keyed handling gets wrong: the link lands,
//! a value both devices already know does not fail the page, and a delete drops
//! the link without dropping the edition.

use livtet_data::sql::{self, AssertSqlSafe};
use livtet_sync::SyncChange;
use livtet_test_utils::sync::{Device, change};

/// The ISBN both devices' catalogues hold.
const ISBN: &str = "urn:isbn:9781635575637";

async fn insert_work(device: &Device, id: &[u8], title: &str) {
    sql::query(AssertSqlSafe(
        "INSERT INTO works (id, title, created_at) VALUES (?, ?, datetime('now'))".to_string(),
    ))
    .bind(id)
    .bind(title)
    .execute(device.pool())
    .await
    .unwrap_or_else(|error| panic!("insert work: {error}"));
}

async fn insert_edition(device: &Device, id: &[u8], work_id: &[u8], title: &str) {
    sql::query(AssertSqlSafe(
        "INSERT INTO editions (id, work_id, title, created_at) \
         VALUES (?, ?, ?, datetime('now'))"
            .to_string(),
    ))
    .bind(id)
    .bind(work_id)
    .bind(title)
    .execute(device.pool())
    .await
    .unwrap_or_else(|error| panic!("insert edition: {error}"));
}

async fn insert_identifier(device: &Device, id: &[u8], value: &str, kind: &str) {
    sql::query(AssertSqlSafe(
        "INSERT INTO identifiers (id, value, kind) VALUES (?, ?, ?)".to_string(),
    ))
    .bind(id)
    .bind(value)
    .bind(kind)
    .execute(device.pool())
    .await
    .unwrap_or_else(|error| panic!("insert identifier: {error}"));
}

async fn link(device: &Device, edition_id: &[u8], identifier_id: &[u8]) {
    sql::query(AssertSqlSafe(
        "INSERT INTO edition_identifiers (edition_id, identifier_id) VALUES (?, ?)".to_string(),
    ))
    .bind(edition_id)
    .bind(identifier_id)
    .execute(device.pool())
    .await
    .unwrap_or_else(|error| panic!("link identifier: {error}"));
}

/// One A-side row of setup: a work, an edition, and the identifier `ISBN`.
async fn library(device: &Device, work: &[u8], edition: &[u8], identifier: &[u8]) {
    insert_work(device, work, "Piranesi").await;
    insert_edition(device, edition, work, "Piranesi").await;
    insert_identifier(device, identifier, ISBN, "isbn").await;
    link(device, edition, identifier).await;
}

/// The sender's rows for the types a library pull carries, in apply order.
async fn page(device: &Device) -> Vec<SyncChange> {
    let mut changes = Vec::new();
    for entity_type in ["work", "edition", "edition_identifier"] {
        for row in device.logged(entity_type, "INSERT").await {
            changes.push(change(
                entity_type,
                &row.entity_id,
                &row.operation,
                row.version,
                &row.payload,
            ));
        }
    }
    assert_eq!(changes.len(), 3, "the fixture must log one row per type");
    changes
}

/// `SELECT COUNT(*)` with BLOB binds — ids, which is what most of these read.
async fn count_ids(device: &Device, statement: &str, binds: &[&[u8]]) -> i64 {
    let mut query = sql::query_scalar::<_, i64>(AssertSqlSafe(statement.to_string()));
    for bind in binds {
        query = query.bind(*bind);
    }
    query
        .fetch_one(device.pool())
        .await
        .unwrap_or_else(|error| panic!("{statement}: {error}"))
}

/// `SELECT COUNT(*)` with a text bind: `identifiers.value` is TEXT, and a BLOB
/// bind compares as a different type and never matches.
async fn count_text(device: &Device, statement: &str, bind: &str) -> i64 {
    sql::query_scalar::<_, i64>(AssertSqlSafe(statement.to_string()))
        .bind(bind)
        .fetch_one(device.pool())
        .await
        .unwrap_or_else(|error| panic!("{statement}: {error}"))
}

#[tokio::test]
async fn an_identifier_link_lands_on_the_receivers_edition() {
    let sender = Device::new("sender").await;
    let receiver = Device::new("receiver").await;
    let (work, edition) = ([0x11u8; 16], [0x12u8; 16]);
    library(&sender, &work, &edition, &[0x13u8; 16]).await;

    let outcome = receiver
        .engine
        .apply_changes(page(&sender).await)
        .await
        .expect("apply the page");

    assert_eq!(outcome.applied, 3);
    assert!(outcome.conflicts.is_empty(), "a link is not a conflict");
    assert_eq!(
        count_text(
            &receiver,
            "SELECT COUNT(*) FROM identifiers WHERE value = ?",
            ISBN
        )
        .await,
        1,
        "the receiver holds exactly one row for the value"
    );
    assert_eq!(
        count_ids(
            &receiver,
            "SELECT COUNT(*) FROM edition_identifiers WHERE edition_id = ?",
            &[&edition]
        )
        .await,
        1,
        "the receiver's copy of the edition carries the link"
    );
}

#[tokio::test]
async fn a_value_both_devices_know_applies_under_the_local_id() {
    let sender = Device::new("sender").await;
    let receiver = Device::new("receiver").await;
    let (sender_work, sender_edition) = ([0x21u8; 16], [0x22u8; 16]);
    library(&sender, &sender_work, &sender_edition, &[0x23u8; 16]).await;

    // The receiver imported the same ISBN first, under its own identifier id,
    // on its own edition of the book.
    let (local_work, local_edition, local_identifier) = ([0x31u8; 16], [0x32u8; 16], [0x33u8; 16]);
    library(&receiver, &local_work, &local_edition, &local_identifier).await;
    let before = receiver.logged("edition_identifier", "INSERT").await.len();

    let outcome = receiver
        .engine
        .apply_changes(page(&sender).await)
        .await
        .expect("a value collision must not fail the page");

    assert_eq!(outcome.applied, 3);
    assert_eq!(
        count_text(
            &receiver,
            "SELECT COUNT(*) FROM identifiers WHERE value = ?",
            ISBN
        )
        .await,
        1,
        "the receiver keeps its own identifier row for the value"
    );
    assert_eq!(
        count_ids(
            &receiver,
            "SELECT COUNT(*) FROM edition_identifiers WHERE identifier_id = ?",
            &[&local_identifier]
        )
        .await,
        2,
        "both editions are linked to the one row"
    );
    let after = receiver.logged("edition_identifier", "INSERT").await.len();
    assert_eq!(after - before, 1, "exactly the arriving link was logged");
}

#[tokio::test]
async fn a_deleted_link_drops_the_link_and_keeps_the_edition() {
    let sender = Device::new("sender").await;
    let receiver = Device::new("receiver").await;
    let (work, edition) = ([0x41u8; 16], [0x42u8; 16]);
    library(&sender, &work, &edition, &[0x43u8; 16]).await;

    receiver
        .engine
        .apply_changes(page(&sender).await)
        .await
        .expect("apply the page");

    // Withdrawing a link is the sender's delete of the junction row.
    sender.execute("DELETE FROM edition_identifiers").await;
    let deleted = sender
        .logged("edition_identifier", "DELETE")
        .await
        .pop()
        .expect("the delete is logged");
    let outcome = receiver
        .engine
        .apply_changes(vec![change(
            "edition_identifier",
            &deleted.entity_id,
            &deleted.operation,
            deleted.version,
            &deleted.payload,
        )])
        .await
        .expect("apply the delete");

    assert_eq!(outcome.applied, 1);
    assert_eq!(
        count_ids(
            &receiver,
            "SELECT COUNT(*) FROM edition_identifiers WHERE edition_id = ?",
            &[&edition]
        )
        .await,
        0,
        "the link is gone"
    );
    assert_eq!(
        count_ids(
            &receiver,
            "SELECT COUNT(*) FROM editions WHERE id = ?",
            &[&edition]
        )
        .await,
        1,
        "the edition it described stays"
    );
}

/// The engine names a type it applied, so an unknown type is a skip and not a
/// silent success: `edition_identifier` must be a table-mapped entity type.
#[test]
fn edition_identifier_maps_to_its_table() {
    assert_eq!(
        livtet_sync::entity_type_to_table("edition_identifier"),
        Some("edition_identifiers")
    );
}

/// A `SyncEngine` the type's absence would break: the apply branch is reached
/// for these rows, so the engine must also rank them after the edition.
#[tokio::test]
async fn a_link_for_an_edition_the_receiver_lacks_is_skipped() {
    let sender = Device::new("sender").await;
    let receiver = Device::new("receiver").await;
    let (work, edition) = ([0x51u8; 16], [0x52u8; 16]);
    library(&sender, &work, &edition, &[0x53u8; 16]).await;

    // The edition row is withheld: a link to a book that is not here would fail
    // the foreign key and lose the whole page.
    let link_row = sender
        .logged("edition_identifier", "INSERT")
        .await
        .pop()
        .expect("the link is logged");
    let outcome = receiver
        .engine
        .apply_changes(vec![change(
            "edition_identifier",
            &link_row.entity_id,
            &link_row.operation,
            link_row.version,
            &link_row.payload,
        )])
        .await
        .expect("apply");

    assert_eq!(outcome.applied, 0);
    assert_eq!(outcome.skipped, 1);
    assert_eq!(
        count_ids(&receiver, "SELECT COUNT(*) FROM identifiers", &[]).await,
        0,
        "no identifier row is minted for a link that cannot land"
    );
}
