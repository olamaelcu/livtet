//! The reading-progress payload the main process returns to
//! the host in [`crate::protocol::MainToHostCallback::FetchProgressResult`].
//!
//! Mirrors the row shape of the `reading_progress` table (see
//! `livtet-migration::m0007_reading_annotation`) plus a few
//! fields the host closure on the Lua side can return to the
//! plugin. The host is responsible for stamping `id` and
//! `updated_at`; everything else comes from the row.

use livtet_core::DbId;
use serde::{Deserialize, Serialize};
use specta::Type;

/// The full `reading_progress` row, in the shape the host
/// closure `host.fetch_progress(urn)` returns to the plugin.
///
/// `format_id` is always present (the dispatcher picked the
/// default format for the edition before doing the read). The
/// `id` is the primary key; callers usually don't need it but
/// the round-trip is lossless so it's included.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct ProgressEntry {
    pub id: DbId,
    pub edition_id: DbId,
    pub format_id: DbId,
    pub progress: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_location: Option<String>,
    pub total_reading_time_secs: i64,
    /// Wall-clock time the row was last written, in the same
    /// ISO-8601 form the rest of the plugin protocol uses.
    /// `None` for rows that have never been touched (the
    /// migration default is `NULL`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

impl ProgressEntry {
    // DB-bridge conversion (`from_model`) was removed alongside
    // `livtet_core::crud` and the `livtet_plugin::host_manager`
    // reading-progress handlers. The struct stays for use as the
    // Lua payload shape returned by `provider.fetch_progress()`.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_entry_round_trips_full_shape() {
        let entry = ProgressEntry {
            id: livtet_core::DbId::new(),
            edition_id: livtet_core::DbId::new(),
            format_id: livtet_core::DbId::new(),
            progress: 0.42,
            last_location: Some("loc 1".to_string()),
            total_reading_time_secs: 600,
            updated_at: Some("2026-05-28T14:30:00".to_string()),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: ProgressEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn progress_entry_omits_none_last_location_and_updated_at() {
        let entry = ProgressEntry {
            id: livtet_core::DbId::new(),
            edition_id: livtet_core::DbId::new(),
            format_id: livtet_core::DbId::new(),
            progress: 0.0,
            last_location: None,
            total_reading_time_secs: 0,
            updated_at: None,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(
            !json.contains("last_location"),
            "last_location: None should be elided; got {json}"
        );
        assert!(
            !json.contains("updated_at"),
            "updated_at: None should be elided; got {json}"
        );
    }
}
