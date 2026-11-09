//! Applying a peer's change-log rows with `apply_changes`.

use livtet_sync::adopt_inventory;
use livtet_test_utils::sync::{Device, change};

async fn latest(device: &Device, entity_type: &str) -> (String, String, i64) {
    livtet_data::sql::query_as(livtet_data::sql::AssertSqlSafe(format!(
        "SELECT entity_id, payload, version FROM change_log \
         WHERE entity_type = '{entity_type}' ORDER BY version DESC LIMIT 1"
    )))
    .fetch_one(device.pool())
    .await
    .expect("a logged change")
}

#[tokio::test]
async fn applies_the_payload_fields_and_records_the_remote_row() {
    let device = Device::seeded("receiver").await;
    let (id, payload, version) = latest(&device, "work").await;
    let mut value: serde_json::Value = serde_json::from_str(&payload).expect("payload");
    value["title"] = serde_json::Value::String("Applied title".to_string());

    // No local history for this entity: the payload is the only thing to apply.
    livtet_data::sql::query(livtet_data::sql::AssertSqlSafe(format!(
        "DELETE FROM change_log WHERE entity_type = 'work' AND entity_id = '{id}'"
    )))
    .execute(device.pool())
    .await
    .expect("clear the local log");

    let outcome = device
        .engine
        .apply_changes(vec![change(
            "work",
            &id,
            "UPDATE",
            version + 1,
            &value.to_string(),
        )])
        .await
        .expect("apply");

    assert_eq!(outcome.applied, 1);
    assert!(outcome.conflicts.is_empty());
    assert_eq!(device.count("works WHERE title = 'Applied title'").await, 1);

    // The remote row is recorded, and the trigger echo for it is gone.
    let remote_rows: i64 =
        livtet_data::sql::query_scalar(livtet_data::sql::AssertSqlSafe(format!(
            "SELECT COUNT(*) FROM change_log WHERE entity_type = 'work' \
         AND entity_id = '{id}' AND version > {version} AND device_id = 'remote'"
        )))
        .fetch_one(device.pool())
        .await
        .expect("count");
    assert_eq!(remote_rows, 1);
}

#[tokio::test]
async fn applies_an_edition_whose_optional_ids_are_empty() {
    let device = Device::seeded("receiver").await;
    let (id, payload, version) = latest(&device, "edition").await;
    let mut value: serde_json::Value = serde_json::from_str(&payload).expect("payload");
    // What the sender's trigger writes for a NULL id: `hex(NULL)` is `''`.
    value["format_id"] = serde_json::Value::String(String::new());
    value["language_id"] = serde_json::Value::String(String::new());

    // No local history for this edition, so the apply path runs.
    livtet_data::sql::query(livtet_data::sql::AssertSqlSafe(format!(
        "DELETE FROM change_log WHERE entity_type = 'edition' AND entity_id = '{id}'"
    )))
    .execute(device.pool())
    .await
    .expect("clear the local log");

    let outcome = device
        .engine
        .apply_changes(vec![change(
            "edition",
            &id,
            "UPDATE",
            version + 1,
            &value.to_string(),
        )])
        .await
        .expect("apply");

    assert_eq!(outcome.applied, 1);
    assert!(outcome.conflicts.is_empty());

    // Stored as SQL NULL: a zero-length BLOB is not a `DbId` and would make
    // every read of the edition fail.
    let (format_null, language_null): (i64, i64) =
        livtet_data::sql::query_as(livtet_data::sql::AssertSqlSafe(format!(
            "SELECT format_id IS NULL, language_id IS NULL FROM editions \
             WHERE lower(hex(id)) = '{id}'"
        )))
        .fetch_one(device.pool())
        .await
        .expect("edition row");

    assert_eq!((format_null, language_null), (1, 1));
}

/// One seeded inventory row, as a peer's change row would carry it.
async fn seeded_inventory(device: &Device) -> (String, String, serde_json::Value, String) {
    let row: (String, String, String, String) =
        livtet_data::sql::query_as(livtet_data::sql::AssertSqlSafe(
            "SELECT lower(hex(id)), lower(hex(edition_id)), file_path, json_object(\
             'id', lower(hex(id)), 'edition_id', lower(hex(edition_id)), \
             'file_path', file_path, 'cover_path', cover_path, \
             'file_hash', file_hash, 'file_size_bytes', file_size_bytes, \
             'added_at', added_at, 'updated_at', updated_at) \
             FROM digital_inventory LIMIT 1"
                .to_string(),
        ))
        .fetch_one(device.pool())
        .await
        .expect("inventory row");
    let (id, edition, path, payload): (String, String, String, String) = row;
    (
        id,
        edition,
        serde_json::from_str(&payload).expect("payload"),
        path,
    )
}

#[tokio::test]
async fn a_peer_inventory_row_is_ignored() {
    let device = Device::seeded("receiver").await;
    let (id, _edition, mut payload, before) = seeded_inventory(&device).await;
    // What a sender would write: its own absolute path.
    payload["file_path"] = serde_json::Value::String("/home/peer/books/book.epub".to_string());

    let outcome = device
        .engine
        .apply_changes(vec![change(
            "digital_inventory",
            &id,
            "UPDATE",
            90_000,
            &payload.to_string(),
        )])
        .await
        .expect("apply");

    assert_eq!(outcome.applied, 0);
    assert_eq!(outcome.skipped, 1);

    // The sender's path never lands: the row still names this device's file.
    let after: String = livtet_data::sql::query_scalar(livtet_data::sql::AssertSqlSafe(format!(
        "SELECT file_path FROM digital_inventory WHERE lower(hex(id)) = '{id}'"
    )))
    .fetch_one(device.pool())
    .await
    .expect("inventory row");
    assert_eq!(after, before);
}

#[tokio::test]
async fn accepting_an_offer_writes_the_local_inventory_row() {
    let device = Device::seeded("receiver").await;
    let (id, edition, _payload, _before) = seeded_inventory(&device).await;
    let inventory_id = livtet_types::DbId::from_hex(&id).expect("inventory id");
    let edition_id = livtet_types::DbId::from_hex(&edition).expect("edition id");

    let logged_before = device
        .count("change_log WHERE entity_type = 'digital_inventory'")
        .await;

    adopt_inventory(
        device.engine.db(),
        inventory_id,
        edition_id,
        "/data/user/0/net.olamaelcu.livtet/files/books/received.epub",
        Some("/data/user/0/net.olamaelcu.livtet/files/covers/received.png"),
        Some("hash"),
        Some(1234),
    )
    .await
    .expect("adopt");

    let (file_path, cover_path, size): (String, String, i64) =
        livtet_data::sql::query_as(livtet_data::sql::AssertSqlSafe(format!(
            "SELECT file_path, cover_path, file_size_bytes FROM digital_inventory \
             WHERE lower(hex(id)) = '{id}'"
        )))
        .fetch_one(device.pool())
        .await
        .expect("inventory row");
    assert_eq!(
        file_path,
        "/data/user/0/net.olamaelcu.livtet/files/books/received.epub"
    );
    assert_eq!(
        cover_path,
        "/data/user/0/net.olamaelcu.livtet/files/covers/received.png"
    );
    assert_eq!(size, 1234);

    // Writing the row is not an announcement: the inventory triggers are dropped.
    let logged_after = device
        .count("change_log WHERE entity_type = 'digital_inventory'")
        .await;
    assert_eq!(logged_after, logged_before);
}

#[tokio::test]
async fn a_newer_local_version_becomes_a_conflict() {
    let device = Device::seeded("receiver").await;
    let (id, payload, _version) = latest(&device, "work").await;
    let mut value: serde_json::Value = serde_json::from_str(&payload).expect("payload");
    value["title"] = serde_json::Value::String("Diverged title".to_string());
    let divergent = value.to_string();

    let outcome = device
        .engine
        .apply_changes(vec![change("work", &id, "UPDATE", 0, &divergent)])
        .await
        .expect("apply");

    // A repeated pull of the same divergence dedupes to one open row.
    let again = device
        .engine
        .apply_changes(vec![change("work", &id, "UPDATE", 0, &divergent)])
        .await
        .expect("apply again");

    assert_eq!(outcome.applied, 0);
    assert_eq!(outcome.conflicts.len(), 1);
    assert_eq!(again.conflicts.len(), 1);
    assert_eq!(
        device
            .count(&format!(
                "conflicts WHERE entity_type = 'work' AND entity_id = '{id}' AND resolved = 0"
            ))
            .await,
        1
    );
}

#[tokio::test]
async fn an_equal_payload_with_a_different_operation_is_not_a_conflict() {
    let device = Device::seeded("receiver").await;
    let (id, payload, _version) = latest(&device, "work").await;

    let outcome = device
        .engine
        .apply_changes(vec![change("work", &id, "UPDATE", 1, &payload)])
        .await
        .expect("apply");

    assert_eq!(outcome.applied, 0);
    assert!(outcome.conflicts.is_empty());
    assert_eq!(outcome.skipped, 1);
}

#[tokio::test]
async fn offers_are_skipped_by_the_apply_path() {
    let device = Device::seeded("receiver").await;

    let outcome = device
        .engine
        .apply_changes(vec![change("file_offer", "00", "OFFER", 1, "{}")])
        .await
        .expect("apply");

    assert_eq!(outcome.applied, 0);
    assert_eq!(outcome.skipped, 1);
}

#[tokio::test]
async fn an_accepted_offer_outranks_a_newer_local_tombstone() {
    let device = Device::seeded("receiver").await;
    let (id, payload, _version) = latest(&device, "work").await;
    let mut value: serde_json::Value = serde_json::from_str(&payload).expect("payload");
    value["title"] = serde_json::Value::String("Accepted title".to_string());

    // Version 0 is older than everything local: a background sync would file this as a
    // conflict. The reader accepted the book, so the offer's rows win.
    let outcome = device
        .engine
        .apply_offer_changes(vec![change("work", &id, "UPDATE", 0, &value.to_string())])
        .await
        .expect("apply");

    assert_eq!(outcome.applied, 1);
    assert!(outcome.conflicts.is_empty());
    assert_eq!(
        device.count("works WHERE title = 'Accepted title'").await,
        1
    );
}

#[tokio::test]
async fn a_cover_can_create_its_own_inventory_row() {
    use livtet_sync::adopt_cover;

    let device = Device::seeded("receiver").await;
    // An edition the seed left without an inventory row: covers travel for books whose
    // file lives on another device.
    let edition: Vec<u8> = livtet_data::sql::query_scalar(
        "SELECT id FROM editions WHERE id NOT IN (SELECT edition_id FROM digital_inventory) LIMIT 1",
    )
    .fetch_one(device.pool())
    .await
    .expect("an edition without inventory");
    let edition = livtet_types::DbId::from_bytes(edition.try_into().expect("16 bytes"));

    adopt_cover(
        device.engine.db(),
        edition,
        "/data/user/0/net.olamaelcu.livtet/files/covers/cover.png",
    )
    .await
    .expect("adopt");

    let (file_path, cover_path): (Option<String>, String) = livtet_data::sql::query_as(
        "SELECT file_path, cover_path FROM digital_inventory WHERE edition_id = ?",
    )
    .bind(edition.to_bytes().to_vec())
    .fetch_one(device.pool())
    .await
    .expect("inventory row");
    assert_eq!(file_path, None);
    assert_eq!(
        cover_path,
        "/data/user/0/net.olamaelcu.livtet/files/covers/cover.png"
    );
}
