//! Shared wire types and route paths for the sync protocol.
//!
//! These are the request/response shapes exchanged over HTTP; both the
//! `reqwest` client and the `poem` server speak them. Keeping them here
//! (rather than in either backend) is what lets the crate compile with
//! neither backend enabled.

use serde::{Deserialize, Serialize};

use crate::error::SyncHttpError;

/// `GET` the server's sync status.
pub const STATUS_PATH: &str = "/sync/status";
/// `GET` incremental changes since a version.
pub const CHANGES_PATH: &str = "/sync/changes";
/// `GET` a full dump of all syncable tables.
pub const PULL_FULL_PATH: &str = "/sync/pull-full";
/// `POST` a batch of local changes.
pub const PUSH_PATH: &str = "/sync/push";
/// `POST` a pairing request.
pub const PAIR_PATH: &str = "/sync/pair";
/// `GET` (SSE) the pairing decision for a token.
pub const PAIR_STATUS_PATH: &str = "/sync/pair/status/:token";
/// `GET` unresolved conflicts.
pub const CONFLICTS_PATH: &str = "/sync/conflicts";
/// `POST` a conflict resolution.
pub const RESOLVE_CONFLICT_PATH: &str = "/sync/conflicts/:id/resolve";
/// `GET` a file by inventory id.
pub const FILE_PATH: &str = "/sync/files/:inventory_id";

/// Query parameters for [`CHANGES_PATH`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullQuery {
    pub since_version: i64,
    pub limit: Option<i64>,
}

/// Body for [`PAIR_PATH`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairRequest {
    pub device_id: String,
    pub name: String,
    pub device_type: String,
    pub token: String,
}

/// Body for [`RESOLVE_CONFLICT_PATH`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveRequest {
    pub resolution: String,
    pub merged_payload: Option<String>,
}

/// Join a base URL and a path, trimming the boundary slash.
///
/// Validates only that the base looks like an absolute `http(s)` URL;
/// the `reqwest` backend re-parses it with the `url` crate before use.
pub fn join_url(base: &str, path: &str) -> Result<String, SyncHttpError> {
    let base = base.trim();
    if base.is_empty() {
        return Err(SyncHttpError::Url("base url is empty".to_string()));
    }
    if !base.starts_with("http://") && !base.starts_with("https://") {
        return Err(SyncHttpError::Url(format!(
            "base url must start with http:// or https://: {base}"
        )));
    }
    Ok(format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    ))
}
