//! Typed structures for the `annotations` capability.
//!
//! Mirrors the shapes from
//! `docs/superpowers/plugins/2026-06-02-plugin-capability-interfaces.md`
//! §4 "Annotation Importers". Plugins return these as Lua tables; the
//! host dispatches `provider.annotation_sources()` and
//! `provider.fetch_annotations(source_id, config)` through the same IPC
//! round-trip as any other capability and the dispatcher decodes the
//! JSON payload into these types.

use serde::{Deserialize, Serialize};
use specta::Type;

/// A single importable annotation source returned by
/// `provider.annotation_sources()`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct AnnotationSource {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub config_fields: Vec<AnnotationConfigField>,
}

/// One user-facing field the plugin needs configured before a fetch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct AnnotationConfigField {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub field_type: String,
    #[serde(default)]
    pub accept: Option<String>,
}

/// One annotation entry returned by `provider.fetch_annotations()`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct AnnotationEntry {
    pub identifiers: Vec<String>,
    /// Required by the spec but may be the empty string when the
    /// annotation is a note-only entry; the host's storage mapping
    /// (see spec §4) folds `content` + `note` into the row's
    /// `annotations.content` column.
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    /// `"range" | "page" | "cfi" | "css_selector" | "offset" | "timestamp"`.
    /// Plugins use a string today; a future migration may turn this
    /// into a typed enum.
    #[serde(default)]
    pub location_type: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// Full response shape for `provider.fetch_annotations()`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct FetchAnnotationsResult {
    pub annotations: Vec<AnnotationEntry>,
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
    fn annotation_source_round_trips_minimal() {
        let src = json!({
            "id": "kindle-clippings-file",
            "label": "Kindle Clippings File",
        });
        let parsed: AnnotationSource = serde_json::from_value(src).unwrap();
        assert_eq!(parsed.id, "kindle-clippings-file");
        assert_eq!(parsed.label, "Kindle Clippings File");
        assert!(parsed.description.is_none());
        assert!(parsed.config_fields.is_empty());
    }

    #[test]
    fn annotation_source_round_trips_with_config_fields() {
        let src = json!({
            "id": "kindle-clippings-file",
            "label": "Kindle Clippings File",
            "description": "Import from My Clippings.txt",
            "config_fields": [
                { "key": "file_path", "label": "Clippings File", "type": "file",
                  "accept": ".txt" },
            ],
        });
        let parsed: AnnotationSource = serde_json::from_value(src).unwrap();
        assert_eq!(parsed.config_fields.len(), 1);
        assert_eq!(parsed.config_fields[0].key, "file_path");
        assert_eq!(parsed.config_fields[0].field_type, "file");
        assert_eq!(parsed.config_fields[0].accept.as_deref(), Some(".txt"));
    }

    #[test]
    fn annotation_entry_round_trips_full_shape() {
        let entry = json!({
            "identifiers": ["urn:isbn:9780441013593", "urn:kindle:B00ISCH6SI"],
            "content": "I must not fear.",
            "title": "Dune",
            "author": "Frank Herbert",
            "note": "Great opening line",
            "location": "Loc. 45-47",
            "location_type": "range",
            "color": "yellow",
            "tags": ["favorite"],
            "created_at": "2026-05-28T14:30:00Z",
        });
        let parsed: AnnotationEntry = serde_json::from_value(entry).unwrap();
        assert_eq!(parsed.identifiers.len(), 2);
        assert_eq!(parsed.content, "I must not fear.");
        assert_eq!(parsed.title.as_deref(), Some("Dune"));
        assert_eq!(parsed.note.as_deref(), Some("Great opening line"));
        assert_eq!(parsed.location.as_deref(), Some("Loc. 45-47"));
        assert_eq!(parsed.location_type.as_deref(), Some("range"));
        assert_eq!(parsed.color.as_deref(), Some("yellow"));
        assert_eq!(parsed.tags, vec!["favorite"]);
    }

    #[test]
    fn annotation_entry_handles_note_only() {
        let entry = json!({
            "identifiers": ["urn:isbn:9780441013593"],
            "note": "Just a thought",
        });
        let parsed: AnnotationEntry = serde_json::from_value(entry).unwrap();
        assert_eq!(parsed.content, "");
        assert!(parsed.title.is_none());
        assert!(parsed.location.is_none());
        assert!(parsed.tags.is_empty());
        assert_eq!(parsed.note.as_deref(), Some("Just a thought"));
    }

    #[test]
    fn fetch_annotations_result_omits_optional_fields() {
        let result = json!({
            "annotations": [{
                "identifiers": ["urn:isbn:111"],
                "content": "highlighted text",
            }],
        });
        let parsed: FetchAnnotationsResult = serde_json::from_value(result).unwrap();
        assert_eq!(parsed.annotations.len(), 1);
        assert!(!parsed.has_more);
        assert!(parsed.cursor.is_none());
        assert!(parsed.annotations[0].title.is_none());
        assert!(parsed.annotations[0].note.is_none());
        assert!(parsed.annotations[0].location.is_none());
        assert!(parsed.annotations[0].tags.is_empty());
    }

    #[test]
    fn annotations_capability_name_is_stable() {
        assert_eq!(
            crate::capability::Capability::Annotations.as_str(),
            "annotations"
        );
    }
}
