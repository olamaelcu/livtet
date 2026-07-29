//! Types for the `Pull` plugin capability.
//!
//! These types are used by plugins that declare `pull = true` in their
//! manifest. The plugin's `provider.pull(config)` function returns a
//! list of `RawPullEntry` values, which the host stores in the inbox
//! and passes through the enrichment pipeline to produce `EnrichedPullEntry`.

use livtet_types::{DbId, Identifier};
use serde::{Deserialize, Serialize};
use specta::Type;

/// A raw book entry returned by a `Pull` plugin.
///
/// The plugin computes `external_id` as a stable identifier per
/// (source, item) — it's the de-duplication key in the inbox.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RawPullEntry {
    /// Stable identifier per (source, item). The host treats this as
    /// the de-dup key (`UNIQUE(source_id, external_id)` in the inbox).
    pub external_id: String,
    /// The book's title.
    pub title: String,
    /// The book's authors (names only).
    pub authors: Vec<String>,
    /// Identifiers for the book (ISBN, OCLC, LCCN, etc.).
    pub identifiers: Vec<Identifier>,
    /// URLs for the book (detail page, cover, etc.).
    pub urls: Vec<String>,
    /// Publication date, if available (ISO 8601 string).
    pub published_at: Option<String>,
    /// Full plugin-decoded payload, kept as an audit copy.
    pub raw: serde_json::Value,
}

/// An enriched book entry produced by the enrichment pipeline.
///
/// The host runs the `lookup_via_plugins` chain on a `RawPullEntry`
/// and produces this enriched version for the operator review queue.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct EnrichedPullEntry {
    /// The external ID from the original `RawPullEntry`.
    pub external_id: String,
    /// The book's title (may be enriched).
    pub title: String,
    /// The book's authors (may be enriched).
    pub authors: Vec<String>,
    /// Identifiers (may be enriched with additional identifiers).
    pub identifiers: Vec<Identifier>,
    /// URLs (may be enriched).
    pub urls: Vec<String>,
    /// Publication date (may be enriched).
    pub published_at: Option<String>,
    /// Description (from enrichment).
    pub description: Option<String>,
    /// Cover URL (from enrichment).
    pub cover_url: Option<String>,
    /// Publisher (from enrichment).
    pub publisher: Option<String>,
    /// Language (from enrichment).
    pub language: Option<String>,
    /// Page count (from enrichment).
    pub page_count: Option<i32>,
    /// Confidence score from the enrichment chain (0.0 to 1.0).
    pub confidence: f64,
    /// The plugin that provided the enrichment (e.g., \"googlebooks\").
    pub enrichment_source: String,
    /// The raw payload from the original `RawPullEntry`.
    pub raw: serde_json::Value,
}

/// Result of a pull operation.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PullResult {
    /// Number of entries successfully fetched.
    pub entries_fetched: u32,
    /// Number of new entries added to the inbox (vs duplicates).
    pub entries_enqueued: u32,
    /// The source ID.
    pub source_id: DbId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_pull_entry_serializes_correctly() {
        let entry = RawPullEntry {
            external_id: "test:123".to_string(),
            title: "Test Book".to_string(),
            authors: vec!["Author One".to_string(), "Author Two".to_string()],
            identifiers: vec![],
            urls: vec!["https://example.com/book".to_string()],
            published_at: Some("2024-01-01".to_string()),
            raw: serde_json::json!({"test": "data"}),
        };

        let json = serde_json::to_string(&entry).unwrap();
        let parsed: RawPullEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.external_id, entry.external_id);
        assert_eq!(parsed.title, entry.title);
        assert_eq!(parsed.authors, entry.authors);
    }

    #[test]
    fn enriched_pull_entry_serializes_correctly() {
        let entry = EnrichedPullEntry {
            external_id: "test:123".to_string(),
            title: "Enriched Test Book".to_string(),
            authors: vec!["Author One".to_string()],
            identifiers: vec![],
            urls: vec![],
            published_at: Some("2024-01-01".to_string()),
            description: Some("A test description".to_string()),
            cover_url: Some("https://example.com/cover.jpg".to_string()),
            publisher: Some("Test Publisher".to_string()),
            language: Some("en".to_string()),
            page_count: Some(300),
            confidence: 0.85,
            enrichment_source: "googlebooks".to_string(),
            raw: serde_json::json!({"test": "data"}),
        };

        let json = serde_json::to_string(&entry).unwrap();
        let parsed: EnrichedPullEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.confidence, 0.85);
        assert_eq!(parsed.enrichment_source, "googlebooks");
        assert!(parsed.description.is_some());
    }

    #[test]
    fn pull_result_serializes_correctly() {
        let result = PullResult {
            entries_fetched: 10,
            entries_enqueued: 7,
            source_id: DbId::new(),
        };

        let json = serde_json::to_string(&result).unwrap();
        let parsed: PullResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.entries_fetched, 10);
        assert_eq!(parsed.entries_enqueued, 7);
    }
}
