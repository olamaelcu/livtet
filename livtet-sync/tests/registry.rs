//! The sync entity registry is spelled out in several places that must
//! agree: the engine's supported list, the `SyncableEntityKind` macro,
//! the entity-type → table map, and the audit triggers the client
//! migrations install. A new entity wired into only some of them would
//! sync partially, so this test pins them together.

use std::collections::BTreeSet;

use livtet_sync::{
    ENTITY_DUMP_TYPES, SUPPORTED_ENTITY_TYPES, SyncableEntityKind, entity_type_to_table,
};
use livtet_test_utils::sync::Device;

fn set(items: &[&'static str]) -> BTreeSet<&'static str> {
    let set: BTreeSet<_> = items.iter().copied().collect();
    assert_eq!(set.len(), items.len(), "duplicate entries in {items:?}");
    set
}

#[test]
fn supported_types_match_the_entity_kind_registry() {
    assert_eq!(
        set(SUPPORTED_ENTITY_TYPES),
        set(SyncableEntityKind::ALL_VARIANTS)
    );
}

#[test]
fn every_supported_type_maps_to_a_distinct_table() {
    let tables: Vec<&str> = SUPPORTED_ENTITY_TYPES
        .iter()
        .map(|t| entity_type_to_table(t).unwrap_or_else(|| panic!("{t} has no table")))
        .collect();
    set(&tables);
    assert_eq!(entity_type_to_table("not_an_entity"), None);
}

#[test]
fn full_dump_types_are_supported_types() {
    let supported = set(SUPPORTED_ENTITY_TYPES);
    for dump_type in ENTITY_DUMP_TYPES {
        assert!(supported.contains(dump_type), "{dump_type} is not syncable");
    }
}

#[tokio::test]
async fn every_supported_type_has_insert_and_delete_triggers_on_its_table() {
    let device = Device::new("registry").await;
    let triggers: Vec<(String, String)> = livtet_data::sql::query_as(
        "SELECT name, tbl_name FROM sqlite_master \
         WHERE type = 'trigger' AND name LIKE 'sync\\_%\\_changelog\\_%' ESCAPE '\\'",
    )
    .fetch_all(device.pool())
    .await
    .expect("list triggers");

    let mut triggered = BTreeSet::new();
    for (name, table) in &triggers {
        let entity_type = name
            .strip_prefix("sync_")
            .and_then(|rest| rest.rsplit_once("_changelog_"))
            .map(|(entity_type, _op)| entity_type)
            .unwrap_or_else(|| panic!("unexpected trigger name {name}"));
        assert_eq!(
            entity_type_to_table(entity_type),
            Some(table.as_str()),
            "trigger {name} is on the wrong table"
        );
        triggered.insert(entity_type.to_string());
    }
    let supported: BTreeSet<String> = SUPPORTED_ENTITY_TYPES
        .iter()
        .map(|t| t.to_string())
        .collect();
    assert_eq!(triggered, supported);

    for entity_type in SUPPORTED_ENTITY_TYPES {
        for op in ["insert", "delete"] {
            let name = format!("sync_{entity_type}_changelog_{op}");
            assert!(
                triggers.iter().any(|(n, _)| *n == name),
                "missing trigger {name}"
            );
        }
    }
}
