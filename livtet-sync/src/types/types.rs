//! Wire DTOs for the livtet sync protocol.
//!
//! Every type here is serde-serialised for either the HTTP wire
//! shape (`/sync/push`, `/sync/changes`, `/sync/pull-full`,
//! `/sync/status`) or the in-process FFI return type used by the
//! mobile client.
//!
//! `SyncChange::id` is the `change_log` row's
//! `INTEGER PRIMARY KEY AUTOINCREMENT` — distinct from
//! `livtet_types::DbId`, which is a 16-byte ULID.  The FFI's
//! `push_changes` writes `0` for the autoincrement column when
//! constructing a `SyncChange` to push; the engine ignores the
//! value on the insert path (it doesn't appear in the
//! `INSERT INTO change_log ...` statement) and synthesises a
//! fresh one when reading.

use livtet_types::DbId;
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncChange {
    pub id: i64,
    pub entity_type: String,
    pub entity_id: String,
    pub operation: String,
    pub version: i64,
    pub payload: String,
    pub changed_at: String,
    pub device_id: String,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    pub id: DbId,
    pub entity_type: String,
    pub entity_id: String,
    pub local_payload: String,
    pub remote_payload: String,
    pub resolved: bool,
    pub resolution: Option<String>,
    pub merged_payload: Option<String>,
    pub detected_at: String,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[must_use]
pub struct PullResponse {
    pub changes: Vec<SyncChange>,
    pub has_more: bool,
    pub latest_version: i64,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[must_use]
pub struct PushResponse {
    pub accepted: bool,
    pub conflicts: Vec<Conflict>,
    pub latest_version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FullDump {
    pub version: i64,
    pub device_id: String,
    pub entities: EntityDump,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityDump {
    pub works: Vec<serde_json::Value>,
    pub editions: Vec<serde_json::Value>,
    pub edition_groups: Vec<serde_json::Value>,
    pub series_entries: Vec<serde_json::Value>,
    pub digital_inventory: Vec<serde_json::Value>,
    pub owned_editions: Vec<serde_json::Value>,
    pub editions_loans: Vec<serde_json::Value>,
    pub annotations: Vec<serde_json::Value>,
    pub reading_lists: Vec<serde_json::Value>,
    pub reading_list_book: Vec<serde_json::Value>,
    pub reading_progress: Vec<serde_json::Value>,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub latest_version: i64,
    pub device_id: String,
}
