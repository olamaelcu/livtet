//! Incremental (`pull_changes`) and full (`pull_full`) reads of the local
//! change log and syncable tables.

use livtet_sync::{SyncChange, entity_type_to_table};
use livtet_test_utils::sync::Device;
use livtet_types::DbId;

async fn pull_all(device: &Device) -> Vec<SyncChange> {
    device
        .engine
        .pull_changes(0, i64::MAX)
        .await
        .expect("pull")
        .changes
}

#[tokio::test]
async fn empty_database_has_nothing_to_pull() {
    let device = Device::new("a").await;
    assert_eq!(device.engine.get_latest_version().await.unwrap(), 0);

    let pulled = device.engine.pull_changes(0, 100).await.unwrap();
    assert!(pulled.changes.is_empty());
    assert!(!pulled.has_more);
    assert_eq!(pulled.latest_version, 0);
}

#[tokio::test]
async fn pull_returns_the_whole_change_log_in_version_order() {
    let device = Device::seeded("a").await;
    let log = device.change_log().await;
    let pulled = pull_all(&device).await;

    assert_eq!(pulled.len(), log.len());
    for (change, row) in pulled.iter().zip(&log) {
        assert_eq!(change.id, row.id);
        assert_eq!(change.entity_type, row.entity_type);
        assert_eq!(change.entity_id, row.entity_id);
        assert_eq!(change.operation, row.operation);
        assert_eq!(change.version, row.version);
        assert_eq!(change.payload, row.payload);
        assert_eq!(change.device_id, row.device_id);
    }
    assert_eq!(
        device.engine.get_latest_version().await.unwrap(),
        log.last().expect("seeded").version
    );
}

#[tokio::test]
async fn paging_visits_every_change_exactly_once() {
    let device = Device::seeded("a").await;
    let everything = pull_all(&device).await;
    let limit = 7;

    let mut since = 0;
    let mut paged = Vec::new();
    loop {
        let page = device.engine.pull_changes(since, limit).await.unwrap();
        assert!(page.changes.len() as i64 <= limit);
        assert_eq!(page.has_more, page.changes.len() as i64 == limit);
        assert_eq!(
            page.latest_version,
            page.changes.last().map_or(since, |c| c.version)
        );
        since = page.latest_version;
        paged.extend(page.changes);
        if !page.has_more {
            break;
        }
    }

    let versions = |changes: &[SyncChange]| changes.iter().map(|c| c.version).collect::<Vec<_>>();
    assert_eq!(versions(&paged), versions(&everything));
}

#[tokio::test]
async fn pull_since_the_latest_version_is_empty() {
    let device = Device::seeded("a").await;
    let latest = device.engine.get_latest_version().await.unwrap();

    for since in [latest, latest + 10] {
        let page = device.engine.pull_changes(since, 100).await.unwrap();
        assert!(page.changes.is_empty());
        assert!(!page.has_more);
        assert_eq!(page.latest_version, since);
    }
}

#[tokio::test]
async fn pull_picks_up_changes_made_after_the_last_pull() {
    let device = Device::seeded("a").await;
    let since = device.engine.get_latest_version().await.unwrap();

    device.execute("UPDATE works SET title = 'Renamed'").await;
    let works = device.count("works").await;

    let page = device.engine.pull_changes(since, 100).await.unwrap();
    assert_eq!(page.changes.len() as i64, works);
    assert!(
        page.changes
            .iter()
            .all(|c| c.entity_type == "work" && c.operation == "UPDATE")
    );
    for change in &page.changes {
        let payload: serde_json::Value = serde_json::from_str(&change.payload).unwrap();
        assert_eq!(payload["title"], "Renamed");
    }
}

#[tokio::test]
async fn full_dump_holds_every_row_of_each_dumped_table() {
    let device = Device::seeded("a").await;
    let dump = device.engine.pull_full().await.unwrap();

    assert_eq!(
        dump.version,
        device.engine.get_latest_version().await.unwrap()
    );
    assert_eq!(dump.device_id, "a");

    let entities = serde_json::to_value(&dump.entities).unwrap();
    let dumped = |key: &str| -> Vec<serde_json::Value> {
        entities[key]
            .as_array()
            .unwrap_or_else(|| panic!("dump has no {key}"))
            .clone()
    };
    for (key, entity_type) in [
        ("works", "work"),
        ("editions", "edition"),
        ("editionGroups", "edition_group"),
        ("seriesEntries", "series_entry"),
        ("digitalInventory", "digital_inventory"),
        ("ownedEditions", "owned_edition"),
        ("editionsLoans", "edition_loan"),
        ("annotations", "annotation"),
        ("readingLists", "reading_list"),
        ("readingListBook", "reading_list_book"),
        ("readingProgress", "reading_progress"),
    ] {
        let table = entity_type_to_table(entity_type).unwrap();
        assert_eq!(dumped(key).len() as i64, device.count(table).await, "{key}");
    }

    // Ids in the full dump are ULID strings of the stored ids.
    let mut ids: Vec<String> = dumped("works")
        .iter()
        .map(|w| {
            let id = w["id"].as_str().expect("work id").to_string();
            id.parse::<DbId>().expect("ULID string");
            id
        })
        .collect();
    ids.sort();
    let mut stored: Vec<String> =
        livtet_data::sql::query_scalar::<_, Vec<u8>>("SELECT id FROM works")
            .fetch_all(device.pool())
            .await
            .unwrap()
            .into_iter()
            .map(|bytes| DbId::from_bytes(bytes.try_into().expect("16-byte id")).to_string())
            .collect();
    stored.sort();
    assert_eq!(ids, stored);
}
