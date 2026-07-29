//! Typed structures for the `reading_progress` capability.
//!
//! Mirrors the shapes from
//! `docs/superpowers/plugins/2026-06-02-plugin-capability-interfaces.md`
//! §3 "Reading Progress Trackers". Plugins return these as Lua tables;
//! the host dispatches `provider.progress_sources()` and
//! `provider.fetch_progress(source_id, config)` through the same IPC
//! round-trip as any other capability and the dispatcher decodes the
//! JSON payload into these types.
//!
//! The `source` field on each entry is filled in by the dispatcher
//! after deserialization (not by the plugin) so the DB row knows
//! which provider the entry came from.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use specta::Type;

/// A single importable source returned by `provider.progress_sources()`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct ProgressSource {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub config_fields: Vec<ProgressConfigField>,
}

/// One user-facing field the plugin needs configured before a fetch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct ProgressConfigField {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub field_type: String,
    #[serde(default)]
    pub accept: Option<String>,
}

/// One progress entry returned by `provider.fetch_progress()`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct ProgressEntry {
    pub identifiers: Vec<String>,
    pub progress: f64,
    /// `"percentage" | "page" | "chapter" | "timestamp"`.
    /// Plugins use a string today; a future migration may turn
    /// this into a typed enum.
    pub progress_type: String,
    #[serde(default)]
    pub last_location: Option<String>,
    #[serde(default)]
    pub total_reading_time_secs: Option<i64>,
    #[serde(default)]
    pub last_read_at: Option<String>,
    #[serde(default)]
    pub device_info: Option<HashMap<String, String>>,
}

/// Full response shape for `provider.fetch_progress()`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct FetchProgressResult {
    pub entries: Vec<ProgressEntry>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn progress_source_round_trips_minimal() {
        let src = json!({
            "id": "koreader-kosync",
            "label": "KOReader (Kosync)",
        });
        let parsed: ProgressSource = serde_json::from_value(src).unwrap();
        assert_eq!(parsed.id, "koreader-kosync");
        assert_eq!(parsed.label, "KOReader (Kosync)");
        assert!(parsed.description.is_none());
        assert!(parsed.icon.is_none());
        assert!(parsed.config_fields.is_empty());
    }

    #[test]
    fn progress_source_round_trips_with_config_fields() {
        let src = json!({
            "id": "koreader-kosync",
            "label": "KOReader",
            "description": "Sync via Kosync protocol",
            "icon": "koreader.svg",
            "config_fields": [
                { "key": "server_url", "label": "Server URL", "type": "url" },
                { "key": "username", "label": "Username", "type": "text" },
            ],
        });
        let parsed: ProgressSource = serde_json::from_value(src).unwrap();
        assert_eq!(parsed.config_fields.len(), 2);
        assert_eq!(parsed.config_fields[0].key, "server_url");
        assert_eq!(parsed.config_fields[0].field_type, "url");
    }

    #[test]
    fn progress_entry_round_trips_full_shape() {
        let entry = json!({
            "identifiers": ["urn:isbn:9780441172719"],
            "progress": 0.65,
            "progress_type": "percentage",
            "last_location": "65%",
            "total_reading_time_secs": 3600,
            "last_read_at": "2026-05-28T14:30:00Z",
            "device_info": {
                "device_name": "KOReader",
                "app_version": "2024.03",
            },
        });
        let parsed: ProgressEntry = serde_json::from_value(entry).unwrap();
        assert_eq!(parsed.identifiers, vec!["urn:isbn:9780441172719"]);
        assert!((parsed.progress - 0.65).abs() < f64::EPSILON);
        assert_eq!(parsed.progress_type, "percentage");
        let device = parsed.device_info.expect("device_info present");
        assert_eq!(
            device.get("device_name").map(String::as_str),
            Some("KOReader")
        );
    }

    #[test]
    fn fetch_progress_result_omits_optional_fields() {
        let result = json!({
            "entries": [{
                "identifiers": ["urn:isbn:111"],
                "progress": 0.1,
                "progress_type": "percentage",
            }],
        });
        let parsed: FetchProgressResult = serde_json::from_value(result).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        assert!(!parsed.has_more);
        assert!(parsed.cursor.is_none());
        assert!(parsed.entries[0].last_location.is_none());
        assert!(parsed.entries[0].device_info.is_none());
    }

    #[test]
    fn reading_progress_capability_name_is_stable() {
        assert_eq!(
            crate::capability::Capability::ReadingProgress.as_str(),
            "reading_progress"
        );
    }
}
