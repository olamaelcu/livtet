//! Reading a peer's change log: what a page carries, and what a dump does not.

use livtet_test_utils::sync::Device;

#[tokio::test]
async fn change_pages_skip_device_inventory_rows() {
    let device = Device::seeded("sender").await;
    // An old peer's log can still hold inventory rows (the triggers that wrote
    // them are dropped only on this device's schema). They must never travel.
    livtet_data::sql::query("INSERT INTO change_log \
         (entity_type, entity_id, operation, version, payload, changed_at, device_id) \
         VALUES ('digital_inventory', '00', 'UPDATE', 90000, '{}', datetime('now'), 'remote')")
        .execute(device.pool())
        .await
        .expect("insert an inventory change");

    let page = device
        .engine
        .pull_changes(0, 10_000)
        .await
        .expect("page");

    assert!(!page.changes.is_empty(), "the page still carries the library");
    assert!(
        page
            .changes
            .iter()
            .all(|change| change.entity_type != "digital_inventory"),
        "a device's own file paths must not be sent to a peer"
    );
}

#[tokio::test]
async fn full_dumps_carry_no_device_inventory_rows() {
    let device = Device::seeded("sender").await;

    let dump = device.engine.pull_full().await.expect("dump");

    assert!(
        dump.entities.digital_inventory.is_empty(),
        "a dump is a peer's library, not its disk"
    );
    assert!(!dump.entities.works.is_empty(), "the library still travels");
}

#[tokio::test]
async fn an_all_inventory_page_still_advances_the_cursor() {
    let device = Device::seeded("sender").await;
    livtet_data::sql::query(
        "INSERT INTO change_log \
         (entity_type, entity_id, operation, version, payload, changed_at, device_id) \
         VALUES ('digital_inventory', '00', 'UPDATE', 90000, '{}', datetime('now'), 'remote')",
    )
    .execute(device.pool())
    .await
    .expect("insert an inventory change");

    // A one-row page that holds only a device-specific row: it must look empty to the
    // peer, but report where to resume, or every pull would stall on it.
    let page = device
        .engine
        .pull_changes(89_999, 1)
        .await
        .expect("page");

    assert!(page.changes.is_empty());
    assert!(page.has_more);
    assert!(page.latest_version > 89_999);
}
