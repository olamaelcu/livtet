//! Pairs a [`livtet_sync::SyncEngine`] with a [`SyncHttpClient`].

use livtet_sync::{FullDump, PullResponse, PushResponse, SyncChange, SyncEngine, SyncStatus};

use crate::client::SyncHttpClient;
use crate::error::SyncHttpError;

/// A sync session: a local engine plus a remote HTTP client.
///
/// Reproduces the ergonomics of the source crate's `SyncClient` while
/// keeping the engine out of the transport. The `status` / `pull_*` /
/// `push` / `resolve_conflict` methods delegate to the client; the
/// `local_*` methods run against the local database.
pub struct SyncSession<C: SyncHttpClient> {
    engine: SyncEngine,
    client: C,
}

impl<C: SyncHttpClient> SyncSession<C> {
    /// Build a session over a local database and a transport client.
    pub fn new(db: &livtet_data::orm::DatabaseConnection, device_id: &str, client: C) -> Self {
        Self {
            engine: SyncEngine::new(db.clone(), device_id.to_string()),
            client,
        }
    }

    /// Connect the underlying client and touch the local DB so the
    /// connection state matches the source behaviour.
    pub async fn connect(&mut self, base_url: &str) -> Result<(), SyncHttpError> {
        self.client.connect(base_url).await?;
        let _ = self.engine.get_latest_version().await?;
        Ok(())
    }

    pub async fn status(&self) -> Result<SyncStatus, SyncHttpError> {
        self.client.status().await
    }

    pub async fn pull_since(&self, since: i64, limit: i64) -> Result<PullResponse, SyncHttpError> {
        self.client.pull_since(since, limit).await
    }

    pub async fn pull_full(&self) -> Result<FullDump, SyncHttpError> {
        self.client.pull_full().await
    }

    pub async fn push(&self, changes: Vec<SyncChange>) -> Result<PushResponse, SyncHttpError> {
        self.client.push(changes).await
    }

    pub async fn resolve_conflict(
        &self,
        conflict_id: i64,
        resolution: &str,
        merged: Option<&str>,
    ) -> Result<bool, SyncHttpError> {
        self.client
            .resolve_conflict(conflict_id, resolution, merged)
            .await
    }

    /// The local engine, for operations the caller wants to run directly.
    pub fn engine(&self) -> &SyncEngine {
        &self.engine
    }

    /// The local database's latest change-log version.
    pub async fn local_latest_version(&self) -> Result<i64, SyncHttpError> {
        Ok(self.engine.get_latest_version().await?)
    }

    /// Resolve a conflict against the local database.
    pub async fn local_resolve_conflict(
        &self,
        conflict_id: i64,
        resolution: &str,
        merged: Option<&str>,
    ) -> Result<bool, SyncHttpError> {
        Ok(self
            .engine
            .resolve_conflict(conflict_id, resolution, merged)
            .await?)
    }

    /// The connected base URL, if any.
    pub fn base_url(&self) -> Option<&str> {
        self.client.base_url()
    }
}
