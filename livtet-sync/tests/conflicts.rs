//! Conflict detection on push, and the conflict list / resolve
//! bookkeeping.

use livtet_sync::Conflict;
use livtet_test_utils::sync::{Device, change};

/// Edit one seeded work locally and return its change-log id, latest
/// version and latest payload.
async fn locally_edited_work(device: &Device) -> (String, i64, String) {
    device
        .execute(
            "UPDATE works SET title = 'Local edit' \
             WHERE id = (SELECT id FROM works ORDER BY id LIMIT 1)",
        )
        .await;
    let latest = device
        .logged("work", "UPDATE")
        .await
        .pop()
        .expect("update logged");
    (latest.entity_id, latest.version, latest.payload)
}

/// Push a change older than the local edit, producing one conflict.
async fn conflicting_push(device: &Device) -> Conflict {
    let (id, version, _) = locally_edited_work(device).await;
    let response = device
        .engine
        .push_changes(vec![change(
            "work",
            &id,
            "DELETE",
            version - 1,
            "{\"remote\":true}",
        )])
        .await
        .unwrap();
    assert!(!response.accepted);
    let [conflict] = <[Conflict; 1]>::try_from(response.conflicts).expect("one conflict");
    conflict
}

#[tokio::test]
async fn a_change_older_than_the_local_edit_is_a_conflict_and_is_not_applied() {
    let device = Device::seeded("a").await;
    let (id, version, local_payload) = locally_edited_work(&device).await;
    let log_before = device.change_log().await;

    let response = device
        .engine
        .push_changes(vec![change(
            "work",
            &id,
            "DELETE",
            version - 1,
            "{\"remote\":true}",
        )])
        .await
        .unwrap();

    assert!(!response.accepted);
    assert_eq!(response.conflicts.len(), 1);
    let conflict = &response.conflicts[0];
    assert_eq!(conflict.entity_type, "work");
    assert_eq!(conflict.entity_id, id);
    assert_eq!(conflict.local_payload, local_payload);
    assert_eq!(conflict.remote_payload, "{\"remote\":true}");
    assert!(!conflict.resolved);

    // Neither the row nor the change log was touched.
    assert_eq!(
        device
            .count(&format!("works WHERE lower(hex(id)) = '{id}'"))
            .await,
        1
    );
    assert_eq!(device.change_log().await, log_before);
}

#[tokio::test]
async fn unresolved_conflicts_are_listed() {
    let device = Device::seeded("a").await;
    assert!(device.engine.list_conflicts().await.unwrap().is_empty());

    let conflict = conflicting_push(&device).await;
    let listed = device.engine.list_conflicts().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, conflict.id);
    assert_eq!(listed[0].remote_payload, conflict.remote_payload);
}

#[tokio::test]
async fn resolving_records_the_resolution_and_clears_the_conflict() {
    let device = Device::seeded("a").await;
    let conflict = conflicting_push(&device).await;

    assert!(
        device
            .engine
            .resolve_conflict(conflict.id, "merged", Some("{\"merged\":true}"))
            .await
            .unwrap()
    );
    assert!(device.engine.list_conflicts().await.unwrap().is_empty());

    let (resolved, resolution, merged): (i64, String, Option<String>) = livtet_data::sql::query_as(
        "SELECT resolved, resolution, merged_payload FROM conflicts WHERE id = ?",
    )
    .bind(conflict.id)
    .fetch_one(device.pool())
    .await
    .unwrap();
    assert_eq!(resolved, 1);
    assert_eq!(resolution, "merged");
    assert_eq!(merged.as_deref(), Some("{\"merged\":true}"));
}

#[tokio::test]
async fn resolving_twice_or_resolving_an_unknown_conflict_reports_false() {
    let device = Device::seeded("a").await;
    let conflict = conflicting_push(&device).await;

    assert!(
        device
            .engine
            .resolve_conflict(conflict.id, "local", None)
            .await
            .unwrap()
    );
    assert!(
        !device
            .engine
            .resolve_conflict(conflict.id, "remote", None)
            .await
            .unwrap()
    );
    assert!(
        !device
            .engine
            .resolve_conflict(conflict.id + 1000, "local", None)
            .await
            .unwrap()
    );
}
