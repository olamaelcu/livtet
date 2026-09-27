//! Applying a remote device's changes with `push_changes`.

use livtet_sync::SyncError;
use livtet_test_utils::sync::{Device, change};

/// A seeded work's id as the change log spells it, plus the version of
/// its latest logged change.
async fn some_work(device: &Device) -> (String, i64) {
    livtet_data::sql::query_as(
        "SELECT entity_id, MAX(version) FROM change_log \
         WHERE entity_type = 'work' GROUP BY entity_id ORDER BY entity_id LIMIT 1",
    )
    .fetch_one(device.pool())
    .await
    .expect("seeded work")
}

async fn work_exists(device: &Device, hex_id: &str) -> bool {
    device
        .count(&format!("works WHERE lower(hex(id)) = '{hex_id}'"))
        .await
        == 1
}

#[tokio::test]
async fn empty_push_is_accepted_and_changes_nothing() {
    let device = Device::seeded("a").await;
    let latest = device.engine.get_latest_version().await.unwrap();

    let response = device.engine.push_changes(vec![]).await.unwrap();
    assert!(response.accepted);
    assert!(response.conflicts.is_empty());
    assert_eq!(response.latest_version, latest);
    assert_eq!(device.change_log().await.len() as i64, latest);
}

#[tokio::test]
async fn delete_removes_the_row() {
    let device = Device::seeded("a").await;
    let (id, version) = some_work(&device).await;
    assert!(work_exists(&device, &id).await);

    let response = device
        .engine
        .push_changes(vec![change("work", &id, "DELETE", version + 1, "{}")])
        .await
        .unwrap();

    assert!(response.accepted);
    assert!(response.conflicts.is_empty());
    assert!(!work_exists(&device, &id).await);
    assert_eq!(
        response.latest_version,
        device.engine.get_latest_version().await.unwrap()
    );
}

#[tokio::test]
async fn unknown_entity_type_rejects_the_whole_batch() {
    let device = Device::seeded("a").await;
    let (id, version) = some_work(&device).await;
    let log_before = device.change_log().await;

    let result = device
        .engine
        .push_changes(vec![
            change("work", &id, "DELETE", version + 1, "{}"),
            change("not_an_entity", "00", "INSERT", version + 2, "{}"),
        ])
        .await;

    assert!(
        matches!(result, Err(SyncError::UnknownEntityType { ref type_name }) if type_name == "not_an_entity"),
        "{result:?}"
    );
    // The valid delete earlier in the batch was rolled back with it.
    assert!(work_exists(&device, &id).await);
    assert_eq!(device.change_log().await, log_before);
}
