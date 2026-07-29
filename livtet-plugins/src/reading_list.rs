//! Typed structures for the `reading_list` capability.
//!
//! Mirrors the shapes from
//! `docs/superpowers/plugins/2026-06-02-plugin-capability-interfaces.md`
//! §5 "Reading Lists / Collections". Plugins return these as Lua
//! tables; the host dispatches `provider.list_sources()` and
//! `provider.fetch_lists(source_id, config)` through the same IPC
//! round-trip as any other capability and the dispatcher decodes the
//! JSON payload into these types.

use serde::{Deserialize, Serialize};
use specta::Type;

/// A single importable reading-list source returned by
/// `provider.list_sources()`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct ReadingListSource {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Whether this source can host smart lists. Plugins that
    /// implement `evaluate_filter` set this to true.
    #[serde(default)]
    pub supports_smart: bool,
    /// Whether this source supports bidirectional sync.
    #[serde(default)]
    pub supports_sync: bool,
    /// `"pull_only" | "push_only" | "bidirectional"`.
    #[serde(default)]
    pub sync_direction: Option<String>,
    #[serde(default)]
    pub config_fields: Vec<ReadingListConfigField>,
}

/// One user-facing field the plugin needs configured before a fetch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct ReadingListConfigField {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub field_type: String,
    #[serde(default)]
    pub accept: Option<String>,
}

/// One item in a list returned by `provider.fetch_lists()`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct ReadingListItem {
    pub identifiers: Vec<String>,
    #[serde(default)]
    pub position: Option<i64>,
    #[serde(default)]
    pub added_at: Option<String>,
}

/// Full response shape for `provider.fetch_lists()`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct FetchListsResult {
    pub lists: Vec<ReadingListList>,
}

/// One list entry returned inside [`FetchListsResult::lists`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct ReadingListList {
    pub name: String,
    /// `"static" | "smart" | "synced"`.
    pub list_type: String,
    /// Unique ID for dedup on re-sync.
    pub external_id: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub items: Vec<ReadingListItem>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn reading_list_source_round_trips_minimal() {
        let src = json!({
            "id": "goodreads",
            "label": "GoodReads Shelves",
        });
        let parsed: ReadingListSource = serde_json::from_value(src).unwrap();
        assert_eq!(parsed.id, "goodreads");
        assert_eq!(parsed.label, "GoodReads Shelves");
        assert!(!parsed.supports_smart);
        assert!(!parsed.supports_sync);
        assert!(parsed.sync_direction.is_none());
        assert!(parsed.config_fields.is_empty());
    }

    #[test]
    fn reading_list_source_round_trips_with_sync_metadata() {
        let src = json!({
            "id": "goodreads",
            "label": "GoodReads Shelves",
            "description": "Sync your GoodReads shelves as reading lists",
            "supports_smart": false,
            "supports_sync": true,
            "sync_direction": "pull_only",
        });
        let parsed: ReadingListSource = serde_json::from_value(src).unwrap();
        assert!(!parsed.supports_smart);
        assert!(parsed.supports_sync);
        assert_eq!(parsed.sync_direction.as_deref(), Some("pull_only"));
    }

    #[test]
    fn reading_list_item_round_trips_with_optional_position() {
        let item = json!({
            "identifiers": ["urn:isbn:9780441013593"],
            "position": 1,
            "added_at": "2026-01-15T10:00:00Z",
        });
        let parsed: ReadingListItem = serde_json::from_value(item).unwrap();
        assert_eq!(parsed.identifiers, vec!["urn:isbn:9780441013593"]);
        assert_eq!(parsed.position, Some(1));
        assert_eq!(parsed.added_at.as_deref(), Some("2026-01-15T10:00:00Z"));
    }

    #[test]
    fn fetch_lists_result_round_trips_minimal() {
        let result = json!({
            "lists": [{
                "name": "To Read",
                "list_type": "synced",
                "external_id": "goodreads:shelf:to-read",
                "items": [],
            }],
        });
        let parsed: FetchListsResult = serde_json::from_value(result).unwrap();
        assert_eq!(parsed.lists.len(), 1);
        assert_eq!(parsed.lists[0].name, "To Read");
        assert_eq!(parsed.lists[0].list_type, "synced");
        assert!(parsed.lists[0].description.is_none());
        assert!(parsed.lists[0].items.is_empty());
    }

    #[test]
    fn reading_list_capability_name_is_stable() {
        assert_eq!(
            crate::capability::Capability::ReadingList.as_str(),
            "reading_list"
        );
    }
}
