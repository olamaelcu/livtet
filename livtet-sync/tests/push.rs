//! Applying a remote device's changes with `push_changes`.

use livtet_data::sql::{AssertSqlSafe, query_as, query_scalar};
use livtet_sync::SyncError;
use livtet_test_utils::sync::{Device, change};
use serde_json::json;

/// A hex id that no seeded row uses.
const NEW_WORK: &str = "00112233445566778899aabbccddeeff";

/// A seeded work's id as the change log spells it, plus the version of
/// its latest logged change.
async fn some_work(device: &Device) -> (String, i64) {
    query_as(
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

async fn work_title(device: &Device, hex_id: &str) -> Option<String> {
    let statement = format!("SELECT title FROM works WHERE lower(hex(id)) = '{hex_id}'");
    query_scalar::<_, String>(AssertSqlSafe(statement))
        .fetch_optional(device.pool())
        .await
        .expect("read works.title")
}

/// A work/author pair the seed left unlinked, as lowercase hex ids.
async fn unlinked_pair(device: &Device) -> (String, String) {
    query_as::<_, (String, String)>(
        "SELECT lower(hex(works.id)), lower(hex(authors.id)) \
         FROM works CROSS JOIN authors \
         WHERE NOT EXISTS ( \
             SELECT 1 FROM work_authors wa \
             WHERE wa.work_id = works.id AND wa.author_id = authors.id \
         ) \
         LIMIT 1",
    )
    .fetch_one(device.pool())
    .await
    .expect("an unlinked work/author pair")
}

async fn work_payload(title: &str) -> String {
    json!({
        "id": NEW_WORK,
        "title": title,
        "created_at": "2026-01-01 00:00:00",
    })
    .to_string()
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
async fn insert_materializes_a_work() {
    let device = Device::new("a").await;

    let response = device
        .engine
        .push_changes(vec![change(
            "work",
            NEW_WORK,
            "INSERT",
            1,
            &work_payload("Pushed").await,
        )])
        .await
        .unwrap();

    assert!(response.accepted);
    assert!(response.conflicts.is_empty());
    assert_eq!(
        work_title(&device, NEW_WORK).await.as_deref(),
        Some("Pushed")
    );
}

#[tokio::test]
async fn insert_materializes_a_composite_key_link() {
    let device = Device::seeded("a").await;
    let (work, author) = unlinked_pair(&device).await;
    let version = device.engine.get_latest_version().await.unwrap() + 1;
    let payload = json!({ "work_id": work, "author_id": author, "role": "author" }).to_string();

    let response = device
        .engine
        .push_changes(vec![change(
            "work_author",
            &payload,
            "INSERT",
            version,
            &payload,
        )])
        .await
        .unwrap();

    assert!(response.accepted);
    let link = format!(
        "work_authors WHERE lower(hex(work_id)) = '{work}' \
         AND lower(hex(author_id)) = '{author}' AND role = 'author'"
    );
    assert_eq!(device.count(&link).await, 1);
}

#[tokio::test]
async fn delete_removes_a_composite_row() {
    let device = Device::seeded("a").await;
    let (work, author) = unlinked_pair(&device).await;
    let version = device.engine.get_latest_version().await.unwrap() + 1;
    let payload = json!({ "work_id": work, "author_id": author, "role": "author" }).to_string();
    let _ = device
        .engine
        .push_changes(vec![change(
            "work_author",
            &payload,
            "INSERT",
            version,
            &payload,
        )])
        .await
        .unwrap();

    let response = device
        .engine
        .push_changes(vec![change(
            "work_author",
            &payload,
            "DELETE",
            version + 1,
            "{}",
        )])
        .await
        .unwrap();

    assert!(response.accepted);
    let link = format!(
        "work_authors WHERE lower(hex(work_id)) = '{work}' AND lower(hex(author_id)) = '{author}'"
    );
    assert_eq!(device.count(&link).await, 0);
}

#[tokio::test]
async fn replayed_change_is_applied_once() {
    let device = Device::new("a").await;
    let pushed = change("work", NEW_WORK, "INSERT", 1, &work_payload("Once").await);

    let _ = device
        .engine
        .push_changes(vec![pushed.clone()])
        .await
        .unwrap();
    let response = device.engine.push_changes(vec![pushed]).await.unwrap();

    assert!(response.accepted);
    assert_eq!(work_title(&device, NEW_WORK).await.as_deref(), Some("Once"));
    let logged = device
        .count(&format!(
            "change_log WHERE entity_type = 'work' AND entity_id = '{NEW_WORK}' \
             AND version = 1 AND device_id = 'remote' AND operation = 'INSERT'"
        ))
        .await;
    assert_eq!(logged, 1);
}

#[tokio::test]
async fn equal_version_divergent_payload_is_a_conflict() {
    let device = Device::new("a").await;
    let _ = device
        .engine
        .push_changes(vec![change(
            "work",
            NEW_WORK,
            "INSERT",
            1,
            &work_payload("Original").await,
        )])
        .await
        .unwrap();

    let response = device
        .engine
        .push_changes(vec![change(
            "work",
            NEW_WORK,
            "UPDATE",
            1,
            &work_payload("Divergent").await,
        )])
        .await
        .unwrap();

    assert!(!response.accepted);
    assert_eq!(response.conflicts.len(), 1);
    assert_eq!(
        work_title(&device, NEW_WORK).await.as_deref(),
        Some("Original")
    );
    assert_eq!(device.count("conflicts").await, 1);
}

#[tokio::test]
async fn malformed_payload_rejects_the_batch() {
    let device = Device::new("a").await;
    let log_before = device.change_log().await;

    let result = device
        .engine
        .push_changes(vec![change("work", NEW_WORK, "INSERT", 1, "not json")])
        .await;

    assert!(
        matches!(result, Err(SyncError::InvalidPayload { .. })),
        "{result:?}"
    );
    assert!(!work_exists(&device, NEW_WORK).await);
    assert_eq!(device.change_log().await, log_before);
}

#[tokio::test]
async fn insert_and_delete_apply() {
    let device = Device::new("a").await;
    let response = device
        .engine
        .push_changes(vec![
            change(
                "work",
                NEW_WORK,
                "INSERT",
                1,
                &work_payload("Transient").await,
            ),
            change("work", NEW_WORK, "DELETE", 2, "{}"),
        ])
        .await
        .unwrap();

    assert!(response.accepted);
    assert!(response.conflicts.is_empty());
    assert!(!work_exists(&device, NEW_WORK).await);
}

#[tokio::test]
async fn unknown_operation_rejects_the_batch() {
    let device = Device::new("a").await;
    let log_before = device.change_log().await;

    let result = device
        .engine
        .push_changes(vec![
            change(
                "work",
                NEW_WORK,
                "INSERT",
                1,
                &work_payload("Rolled back").await,
            ),
            change("work", NEW_WORK, "WEIRD", 2, "{}"),
        ])
        .await;

    assert!(
        matches!(result, Err(SyncError::InvalidOperation { ref operation, .. }) if operation == "WEIRD"),
        "{result:?}"
    );
    assert!(!work_exists(&device, NEW_WORK).await);
    assert_eq!(device.change_log().await, log_before);
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
