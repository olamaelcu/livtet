//! Typed return shapes for the `import_*` capability family.
//!
//! The host uses these types to deserialize plugin responses for
//! `provider.import_detect`, `provider.import_list_items`, and
//! `provider.import_items`. `specta::Type` is derived so the Tauri
//! command layer can hand the shapes directly to the specta
//! exporter without a separate DTO wrapper.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Result of `provider.import_detect(source)`.
/// Plugins that can't handle the source return `nil` (the host
/// surface treats this as "declined").
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ImportDetection {
    /// 0.0–1.0 confidence that this plugin can import the given source.
    pub confidence: f32,
    /// Human-readable format name (e.g. "Calibre SQLite Database").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_name: Option<String>,
    /// Estimated number of importable items (for UI preview).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_count: Option<u32>,
}

/// One row in the preview list returned by `provider.import_list_items`.
/// Lightweight — no file metadata, no series data. The `id` field is
/// used by the frontend to track selections across the multi-step wizard.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ImportPreviewItem {
    /// Plugin-side id; opaque to the host. Used for selection tracking.
    pub id: String,
    /// Display title.
    pub title: String,
    /// Display authors.
    #[serde(default)]
    pub authors: Vec<String>,
    /// URN-format identifiers (e.g. `urn:isbn:978...`).
    #[serde(default)]
    pub identifiers: Vec<String>,
    /// Cover image URL, if the source has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
}

/// One file attached to an import record. Calibre typically has one
/// file per book; other sources (StoryGraph CSV, etc.) may have zero.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ImportFile {
    /// Absolute path to the file in the source library.
    pub path: String,
    /// Format: "epub", "mobi", "azw3", "pdf", etc.
    pub format: String,
    /// File size in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// ISO 8601 last-modified timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
}

/// Canonical import record returned by `provider.import_items`.
/// Carries all the fields `PluginHit` carries, plus file metadata
/// and series information. The host translates each record into
/// Work + Edition + `edition_files` rows.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ImportRecord {
    // --- Canonical PluginHit fields (flattened for wire simplicity) ---
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default)]
    pub identifiers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[specta(type = specta_typescript::Unknown<serde_json::Value>)]
    pub extra: Option<serde_json::Value>,

    // --- Import-specific fields ---
    /// Files attached to this record (may be empty for non-file sources).
    #[serde(default)]
    pub files: Vec<ImportFile>,
    /// Series name, if the source models series.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_name: Option<String>,
    /// Position within the series.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_position: Option<u32>,
    /// Source-side id for re-import tracking (e.g. `urn:calibre:uuid:...`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_detection_serde_round_trip() {
        let det = ImportDetection {
            confidence: 0.95,
            format_name: Some("Calibre SQLite".into()),
            estimated_count: Some(142),
        };
        let json = serde_json::to_string(&det).unwrap();
        let back: ImportDetection = serde_json::from_str(&json).unwrap();
        assert!((back.confidence - 0.95).abs() < 0.001);
        assert_eq!(back.format_name.as_deref(), Some("Calibre SQLite"));
        assert_eq!(back.estimated_count, Some(142));
    }

    #[test]
    fn import_record_defaults_to_empty_files() {
        let rec: ImportRecord =
            serde_json::from_str(r#"{"title":"Test Book","authors":["A. Author"]}"#).unwrap();
        assert!(rec.files.is_empty());
        assert_eq!(rec.title, "Test Book");
    }

    #[test]
    fn import_record_with_files_deserializes() {
        let json = r#"{
            "title": "1984",
            "authors": ["George Orwell"],
            "identifiers": ["urn:isbn:9780451524935"],
            "files": [{"path": "/tmp/1984.epub", "format": "epub", "size": 123456}],
            "series_name": "Classics",
            "series_position": 1
        }"#;
        let rec: ImportRecord = serde_json::from_str(json).unwrap();
        assert_eq!(rec.files.len(), 1);
        assert_eq!(rec.files[0].format, "epub");
        assert_eq!(rec.series_name.as_deref(), Some("Classics"));
    }
}
