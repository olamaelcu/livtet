//! Typed HTTP client for the livtet sync protocol.
//!
//! Wraps the 6 sync routes the FFI consumes: status, push, pull-since,
//! pull-full, resolve-conflict, plus a local-engine convenience for
//! the two operations the FFI performs directly on its own DB
//! (latest-version, resolve-local-conflict).

use std::time::Duration;

use thiserror::Error;

use crate::{
    client::engine::SyncEngine,
    types::{PullResponse, PushResponse, SyncChange, SyncError, SyncStatus},
};

/// All transport + state errors that can occur while talking to a
/// remote livtet sync server or while driving the local engine.
#[derive(Debug, Error)]
pub enum ClientError {
    #[error(transparent)]
    Sync(#[from] SyncError),
    #[error("not connected: call SyncClient::connect first")]
    NotConnected,
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("server returned HTTP {code}: {body}")]
    Status { code: u16, body: String },
    #[error("failed to deserialize response: {0}")]
    Deserialize(#[from] serde_json::Error),
    #[error("invalid base url: {0}")]
    Url(#[from] url::ParseError),
}

/// A typed HTTP client that drives a remote livtet sync server, plus
/// a local `SyncEngine` for the operations that should run against
/// the local SQLite DB instead of over the wire.
pub struct SyncClient {
    base_url: Option<String>,
    http: reqwest::Client,
    engine: SyncEngine,
}

impl SyncClient {
    pub fn new(db: &livtet_data::orm::DatabaseConnection, device_id: &str) -> Self {
        Self::with_http(
            db,
            device_id,
            reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        )
    }

    /// Build a client with an externally supplied `reqwest::Client`.
    pub fn with_http(
        db: &livtet_data::orm::DatabaseConnection,
        device_id: &str,
        http: reqwest::Client,
    ) -> Self {
        Self {
            base_url: None,
            http,
            engine: SyncEngine::new(db.clone(), device_id.to_string()),
        }
    }

    /// Verify the server is reachable before caching the URL, so a
    /// typo in `sync_connect` is surfaced immediately rather than on
    /// the next pull.
    pub async fn connect(&mut self, base_url: &str) -> Result<(), ClientError> {
        let url = join_url(base_url, "/sync/status")?;
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(ClientError::Status {
                code: resp.status().as_u16(),
                body: format!("/sync/status returned HTTP {}", resp.status()),
            });
        }
        // Touch the local DB so the connection state is consistent
        // with the previous behaviour.
        let _ = self.engine.get_latest_version().await?;
        self.base_url = Some(base_url.trim_end_matches('/').to_string());
        Ok(())
    }

    pub fn disconnect(&mut self) {
        self.base_url = None;
    }

    pub fn base_url(&self) -> Option<&str> {
        self.base_url.as_deref()
    }

    pub fn engine(&self) -> &SyncEngine {
        &self.engine
    }

    fn require_connected(&self) -> Result<&str, ClientError> {
        self.base_url.as_deref().ok_or(ClientError::NotConnected)
    }

    pub async fn status(&self) -> Result<SyncStatus, ClientError> {
        let url = join_url(self.require_connected()?, "/sync/status")?;
        let resp = self.http.get(&url).send().await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(ClientError::Status {
                code: status.as_u16(),
                body,
            });
        }
        Ok(serde_json::from_str(&body)?)
    }

    pub async fn pull_since(&self, since: i64, limit: i64) -> Result<PullResponse, ClientError> {
        let path = format!("/sync/changes?since_version={since}&limit={limit}");
        let url = join_url(self.require_connected()?, &path)?;
        let resp = self.http.get(&url).send().await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(ClientError::Status {
                code: status.as_u16(),
                body,
            });
        }
        Ok(serde_json::from_str(&body)?)
    }

    pub async fn pull_full(&self) -> Result<crate::types::FullDump, ClientError> {
        let url = join_url(self.require_connected()?, "/sync/pull-full")?;
        let resp = self.http.get(&url).send().await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(ClientError::Status {
                code: status.as_u16(),
                body,
            });
        }
        Ok(serde_json::from_str(&body)?)
    }

    pub async fn push(&self, changes: Vec<SyncChange>) -> Result<PushResponse, ClientError> {
        let url = join_url(self.require_connected()?, "/sync/push")?;
        let resp = self.http.post(&url).json(&changes).send().await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(ClientError::Status {
                code: status.as_u16(),
                body,
            });
        }
        Ok(serde_json::from_str(&body)?)
    }

    /// Drive a local-engine operation. Wraps `SyncError` into
    /// `ClientError::Sync` for caller convenience.
    pub async fn local_latest_version(&self) -> Result<i64, ClientError> {
        Ok(self.engine.get_latest_version().await?)
    }

    pub async fn local_resolve_conflict(
        &self,
        conflict_id: i64,
        resolution: &str,
        merged: Option<&str>,
    ) -> Result<bool, ClientError> {
        Ok(self
            .engine
            .resolve_conflict(conflict_id, resolution, merged)
            .await?)
    }
}

fn join_url(base: &str, path: &str) -> Result<String, ClientError> {
    let base = base.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    let url = format!("{base}/{path}");
    // Validate the URL parses.
    let _: url::Url = url::Url::parse(&url)?;
    Ok(url)
}
