//! Applying a peer's change-log rows with `apply_changes`.

use livtet_sync::{ApplyOptions, InventoryPaths};
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

    let outcome = device
        .engine
        .apply_changes(
            vec![change(
                "work",
                &id,
                "UPDATE",
                version + 1,
                &value.to_string(),
            )],
            ApplyOptions::default(),
        )
        .await
        .expect("apply");

    assert_eq!(outcome.applied, 1);
    assert!(outcome.conflicts.is_empty());
    assert_eq!(
        device
            .count(&format!("works WHERE title = 'Applied title'"))
            .await,
        1
    );

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

    let outcome = device
        .engine
        .apply_changes(
            vec![change("edition", &id, "UPDATE", version + 1, &payload)],
            ApplyOptions::default(),
        )
        .await
        .expect("apply");

    assert_eq!(outcome.applied, 1);
    assert!(outcome.conflicts.is_empty());
}

#[tokio::test]
async fn inventory_paths_replace_the_sender_absolute_paths() {
    let device = Device::seeded("receiver").await;
    let (id, payload, version) = latest(&device, "digital_inventory").await;

    let mut options = ApplyOptions::default();
    options.inventory_paths.insert(
        id.clone(),
        InventoryPaths {
            file_path: Some("/receiver/books/book.epub".to_string()),
            cover_path: Some("/receiver/covers/book.png".to_string()),
        },
    );

    device
        .engine
        .apply_changes(
            vec![change(
                "digital_inventory",
                &id,
                "UPDATE",
                version + 1,
                &payload,
            )],
            options,
        )
        .await
        .expect("apply");

    let (file_path, cover_path): (String, String) =
        livtet_data::sql::query_as(livtet_data::sql::AssertSqlSafe(format!(
            "SELECT file_path, cover_path FROM digital_inventory WHERE lower(hex(id)) = '{id}'"
        )))
        .fetch_one(device.pool())
        .await
        .expect("inventory row");

    assert_eq!(file_path, "/receiver/books/book.epub");
    assert_eq!(cover_path, "/receiver/covers/book.png");
}

#[tokio::test]
async fn a_newer_local_version_becomes_a_conflict() {
    let device = Device::seeded("receiver").await;
    let (id, payload, _version) = latest(&device, "work").await;

    let outcome = device
        .engine
        .apply_changes(
            vec![change("work", &id, "UPDATE", 0, &payload)],
            ApplyOptions::default(),
        )
        .await
        .expect("apply");

    assert_eq!(outcome.applied, 0);
    assert_eq!(outcome.conflicts.len(), 1);
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
async fn offers_are_skipped_by_the_apply_path() {
    let device = Device::seeded("receiver").await;

    let outcome = device
        .engine
        .apply_changes(
            vec![change("file_offer", "00", "OFFER", 1, "{}")],
            ApplyOptions::default(),
        )
        .await
        .expect("apply");

    assert_eq!(outcome.applied, 0);
    assert_eq!(outcome.skipped, 1);
}
