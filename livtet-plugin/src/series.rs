//! Typed structures for the `series` capability.
//!
//! Mirrors the shapes from
//! `docs/superpowers/plugins/2026-06-02-plugin-capability-interfaces.md`
//! §6 "Series Management". Plugins return these as Lua tables; the
//! host dispatches `provider.detect_series(edition_info)` and
//! `provider.get_series_order(series_info)` through the same IPC
//! round-trip as any other capability and the dispatcher decodes the
//! JSON payload into these types.
//!
//! The optional `detect_series_batch` function in the spec is not
//! dispatched by the host in this commit; the plan's Open Question 3
//! defers it to a follow-up.

use serde::{Deserialize, Serialize};
use specta::Type;

/// A single series detected for an edition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct Series {
    pub name: String,
    /// `"novel" | "anthology" | "omnibus" | "other"`.
    #[serde(default)]
    pub series_type: Option<String>,
    /// Unique ID for dedup.
    pub external_id: String,
    #[serde(default)]
    pub source_url: Option<String>,
    /// Position of this edition in the series.
    #[serde(default)]
    pub position: Option<i64>,
    /// Total number of books in the series, if known.
    #[serde(default)]
    pub total_entries: Option<i64>,
}

/// Full response shape for `provider.detect_series(edition_info)`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct DetectSeriesResult {
    pub series: Vec<Series>,
}

/// One ordered entry returned by `provider.get_series_order(series_info)`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct SeriesEntry {
    pub position: i64,
    pub title: String,
    pub identifiers: Vec<String>,
    #[serde(default)]
    pub published_date: Option<String>,
    /// Optional: position in in-universe chronological order. May
    /// differ from `position` (the publication order).
    #[serde(default)]
    pub in_universe_order: Option<i64>,
}

/// Input shape the host sends to `provider.get_series_order`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct SeriesOrderRequest {
    pub name: String,
    pub external_id: String,
    /// `"publication" | "chronological" | "in-universe"`.
    #[serde(default)]
    pub order_type: Option<String>,
}

/// Full response shape for `provider.get_series_order(series_info)`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct SeriesOrderResult {
    pub entries: Vec<SeriesEntry>,
    /// `"publication" | "chronological" | "in-universe"`.
    pub order_type: String,
    pub available_orders: Vec<String>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn series_round_trips_minimal() {
        let s = json!({
            "name": "Dune Chronicles",
            "external_id": "openlibrary:series:OL_works_Dune",
        });
        let parsed: Series = serde_json::from_value(s).unwrap();
        assert_eq!(parsed.name, "Dune Chronicles");
        assert_eq!(parsed.external_id, "openlibrary:series:OL_works_Dune");
        assert!(parsed.series_type.is_none());
        assert!(parsed.source_url.is_none());
        assert!(parsed.position.is_none());
        assert!(parsed.total_entries.is_none());
    }

    #[test]
    fn series_round_trips_full_shape() {
        let s = json!({
            "name": "Dune Chronicles",
            "series_type": "novel",
            "external_id": "openlibrary:series:OL_works_Dune",
            "source_url": "https://openlibrary.org/works/OL45804W",
            "position": 1,
            "total_entries": 6,
        });
        let parsed: Series = serde_json::from_value(s).unwrap();
        assert_eq!(parsed.series_type.as_deref(), Some("novel"));
        assert_eq!(
            parsed.source_url.as_deref(),
            Some("https://openlibrary.org/works/OL45804W")
        );
        assert_eq!(parsed.position, Some(1));
        assert_eq!(parsed.total_entries, Some(6));
    }

    #[test]
    fn detect_series_result_handles_empty_series() {
        let result = json!({ "series": [] });
        let parsed: DetectSeriesResult = serde_json::from_value(result).unwrap();
        assert!(parsed.series.is_empty());
    }

    #[test]
    fn series_entry_round_trips_with_optional_fields() {
        let entry = json!({
            "position": 1,
            "title": "Dune",
            "identifiers": ["urn:isbn:9780441172719"],
            "published_date": "1965-08-01",
            "in_universe_order": 2,
        });
        let parsed: SeriesEntry = serde_json::from_value(entry).unwrap();
        assert_eq!(parsed.position, 1);
        assert_eq!(parsed.title, "Dune");
        assert_eq!(parsed.identifiers, vec!["urn:isbn:9780441172719"]);
        assert_eq!(parsed.published_date.as_deref(), Some("1965-08-01"));
        assert_eq!(parsed.in_universe_order, Some(2));
    }

    #[test]
    fn series_order_result_round_trips_minimal() {
        let result = json!({
            "entries": [{
                "position": 1,
                "title": "Dune",
                "identifiers": ["urn:isbn:9780441172719"],
            }],
            "order_type": "publication",
            "available_orders": ["publication"],
        });
        let parsed: SeriesOrderResult = serde_json::from_value(result).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.order_type, "publication");
        assert_eq!(parsed.available_orders, vec!["publication"]);
    }

    #[test]
    fn series_order_request_round_trips_minimal() {
        let req = json!({
            "name": "Dune Chronicles",
            "external_id": "openlibrary:series:OL_works_Dune",
        });
        let parsed: SeriesOrderRequest = serde_json::from_value(req).unwrap();
        assert_eq!(parsed.name, "Dune Chronicles");
        assert!(parsed.order_type.is_none());
    }

    #[test]
    fn series_capability_name_is_stable() {
        assert_eq!(crate::capability::Capability::Series.as_str(), "series");
    }
}
