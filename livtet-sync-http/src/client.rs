//! Transport-agnostic client trait for the sync protocol.

use async_trait::async_trait;
use livtet_sync::{FullDump, PullResponse, PushResponse, SyncChange, SyncStatus};

use crate::error::SyncHttpError;

/// The HTTP transport the sync protocol needs, independent of which HTTP
/// library implements it.
///
/// Implementations own their own base-url state plus whatever client
/// handle they need; [`crate::session::SyncSession`] pairs an
/// implementation with a [`livtet_sync::SyncEngine`] for the operations
/// that run against the local database.
#[async_trait]
pub trait SyncHttpClient: Send + Sync {
    /// Verify `base_url` is reachable and remember it.
    async fn connect(&mut self, base_url: &str) -> Result<(), SyncHttpError>;

    /// Fetch the remote server's status.
    async fn status(&self) -> Result<SyncStatus, SyncHttpError>;

    /// Fetch changes after `since`, at most `limit`.
    async fn pull_since(&self, since: i64, limit: i64) -> Result<PullResponse, SyncHttpError>;

    /// Fetch a full dump.
    async fn pull_full(&self) -> Result<FullDump, SyncHttpError>;

    /// Push local changes, returning the server's response.
    async fn push(&self, changes: Vec<SyncChange>) -> Result<PushResponse, SyncHttpError>;

    /// Resolve a remote conflict; `Ok(false)` when the conflict is gone.
    async fn resolve_conflict(
        &self,
        conflict_id: i64,
        resolution: &str,
        merged: Option<&str>,
    ) -> Result<bool, SyncHttpError>;

    /// The connected base URL, if any.
    fn base_url(&self) -> Option<&str>;
}
