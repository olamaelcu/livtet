//! The client migrations' audit triggers are the only producer of
//! `change_log` rows for local edits. These tests drive every syncable
//! table through insert / update / delete and check what gets logged.

use std::collections::BTreeSet;

use livtet_sync::{SUPPORTED_ENTITY_TYPES, entity_type_to_table};
use livtet_test_utils::sync::Device;

/// Entity types keyed by a single `id` column; the rest use composite keys.
const SINGLE_KEY_TYPES: &[&str] = &[
    "work",
    "edition",
    "edition_group",
    "annotation",
    "reading_list",
    "reading_progress",
    "digital_inventory",
    "owned_edition",
    "edition_loan",
];

/// Composite-key types whose rows carry attributes and so are logged on
/// update too. The plain link tables (`work_author`, `edition_tag`, ...)
/// only ever insert or delete.
const UPDATABLE_COMPOSITE_TYPES: &[&str] = &["series_entry", "reading_list_book"];

fn table(entity_type: &str) -> &'static str {
    entity_type_to_table(entity_type).expect("supported entity type")
}

/// One deterministic row of a table, addressed by its primary key so every
/// table is handled the same way regardless of key shape (several are
/// `WITHOUT ROWID`).
struct RowKey {
    /// First primary-key column; `SET col = col` is a no-op update.
    column: String,
    /// `WHERE` clause matching exactly this row.
    filter: String,
}

async fn first_row(device: &Device, table: &str) -> RowKey {
    let pk: Vec<String> = livtet_data::sql::query_scalar(
        "SELECT name FROM pragma_table_info(?) WHERE pk > 0 ORDER BY pk",
    )
    .bind(table)
    .fetch_all(device.pool())
    .await
    .expect("read primary key");
    assert!(!pk.is_empty(), "{table} has no primary key");
    let cols = pk.join(", ");
    RowKey {
        filter: format!("({cols}) IN (SELECT {cols} FROM {table} ORDER BY {cols} LIMIT 1)"),
        column: pk[0].clone(),
    }
}

async fn hex_id_where(device: &Device, table: &str, filter: &str) -> String {
    livtet_data::sql::query_scalar(livtet_data::sql::AssertSqlSafe(format!(
        "SELECT lower(hex(id)) FROM {table} WHERE {filter}"
    )))
    .fetch_one(device.pool())
    .await
    .expect("read id")
}

#[tokio::test]
async fn every_inserted_row_is_logged_once() {
    let device = Device::seeded("a").await;
    for entity_type in SUPPORTED_ENTITY_TYPES {
        let rows = device.count(table(entity_type)).await;
        assert!(rows > 0, "fixture left {entity_type} empty");
        assert_eq!(
            device.logged(entity_type, "INSERT").await.len() as i64,
            rows,
            "{entity_type}"
        );
    }
}

#[tokio::test]
async fn single_key_rows_are_logged_under_their_lowercase_hex_id() {
    let device = Device::seeded("a").await;
    for entity_type in SINGLE_KEY_TYPES {
        let logged: BTreeSet<String> = device
            .logged(entity_type, "INSERT")
            .await
            .into_iter()
            .map(|row| row.entity_id)
            .collect();
        let ids: BTreeSet<String> =
            livtet_data::sql::query_scalar(livtet_data::sql::AssertSqlSafe(format!(
                "SELECT lower(hex(id)) FROM {}",
                table(entity_type)
            )))
            .fetch_all(device.pool())
            .await
            .expect("read ids")
            .into_iter()
            .collect();
        assert_eq!(logged, ids, "{entity_type}");
    }
}

#[tokio::test]
async fn composite_key_rows_are_logged_under_a_json_key() {
    let device = Device::seeded("a").await;
    let single: BTreeSet<&str> = SINGLE_KEY_TYPES.iter().copied().collect();
    for entity_type in SUPPORTED_ENTITY_TYPES
        .iter()
        .filter(|t| !single.contains(*t) && **t != "series_entry")
    {
        for row in device.logged(entity_type, "INSERT").await {
            let key: serde_json::Value = serde_json::from_str(&row.entity_id)
                .unwrap_or_else(|e| panic!("{entity_type} key {:?}: {e}", row.entity_id));
            assert!(key.is_object(), "{entity_type} key {key}");
        }
    }
}

#[tokio::test]
async fn updates_are_logged_for_entities_but_not_link_tables() {
    let device = Device::seeded("a").await;
    let updatable: BTreeSet<&str> = SINGLE_KEY_TYPES
        .iter()
        .chain(UPDATABLE_COMPOSITE_TYPES)
        .copied()
        .collect();

    for entity_type in SUPPORTED_ENTITY_TYPES {
        let table = table(entity_type);
        let RowKey { column, filter } = first_row(&device, table).await;
        let id = if SINGLE_KEY_TYPES.contains(entity_type) {
            Some(hex_id_where(&device, table, &filter).await)
        } else {
            None
        };
        let before = device.logged(entity_type, "UPDATE").await.len();
        device
            .execute(&format!(
                "UPDATE {table} SET {column} = {column} WHERE {filter}"
            ))
            .await;
        let after = device.logged(entity_type, "UPDATE").await;

        let expected = usize::from(updatable.contains(entity_type));
        assert_eq!(after.len() - before, expected, "{entity_type}");
        if let Some(id) = id {
            assert_eq!(after.last().expect("update logged").entity_id, id);
        }
    }
}

#[tokio::test]
async fn deletes_are_logged_for_every_type() {
    use livtet_data::sql::{self, AssertSqlSafe};

    let device = Device::seeded("a").await;
    for entity_type in SUPPORTED_ENTITY_TYPES {
        let table = table(entity_type);
        let RowKey { filter, .. } = first_row(&device, table).await;
        let id = if SINGLE_KEY_TYPES.contains(entity_type) {
            Some(hex_id_where(&device, table, &filter).await)
        } else {
            None
        };

        // Each delete runs in a transaction that is rolled back, so the
        // cascades from one type's delete never empty the next table.
        let mut txn = device.pool().begin().await.expect("begin");
        sql::query(AssertSqlSafe(format!("DELETE FROM {table} WHERE {filter}")))
            .execute(&mut *txn)
            .await
            .unwrap_or_else(|e| panic!("delete from {table}: {e}"));
        let deleted: Vec<String> = sql::query_scalar(
            "SELECT entity_id FROM change_log WHERE entity_type = ? AND operation = 'DELETE'",
        )
        .bind(entity_type)
        .fetch_all(&mut *txn)
        .await
        .expect("read change_log");
        txn.rollback().await.expect("rollback");

        assert!(!deleted.is_empty(), "{entity_type}");
        if let Some(id) = id {
            assert!(
                deleted.contains(&id),
                "{entity_type} delete of {id} not logged"
            );
        }
    }
}

#[tokio::test]
async fn versions_increase_by_one_per_logged_change() {
    let device = Device::seeded("a").await;
    device
        .execute("UPDATE works SET title = title || '!'")
        .await;
    device.execute("DELETE FROM work_tags").await;

    let versions: Vec<i64> = device
        .change_log()
        .await
        .into_iter()
        .map(|row| row.version)
        .collect();
    let expected: Vec<i64> = (1..=versions.len() as i64).collect();
    assert_eq!(versions, expected);
}
