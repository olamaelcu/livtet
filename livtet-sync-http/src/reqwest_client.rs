//! `reqwest` implementation of [`SyncHttpClient`].

use std::time::Duration;

use async_trait::async_trait;
use livtet_sync::{FullDump, PullResponse, PushResponse, SyncChange, SyncStatus};

use crate::{
    client::SyncHttpClient,
    error::SyncHttpError,
    routes::{
        CHANGES_PATH, CONFLICTS_PATH, PULL_FULL_PATH, PUSH_PATH, ResolveRequest, STATUS_PATH,
        join_url,
    },
    session::SyncSession,
};

/// The default request timeout, matching the source client.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// A [`SyncHttpClient`] backed by `reqwest`.
pub struct ReqwestHttpClient {
    base_url: Option<String>,
    http: reqwest::Client,
}

impl ReqwestHttpClient {
    /// Build a client with the default 30s timeout.
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self::with_http(http)
    }

    /// Build a client with an externally supplied [`reqwest::Client`].
    pub fn with_http(http: reqwest::Client) -> Self {
        Self {
            base_url: None,
            http,
        }
    }

    fn require_connected(&self) -> Result<&str, SyncHttpError> {
        self.base_url.as_deref().ok_or(SyncHttpError::NotConnected)
    }
}

impl Default for ReqwestHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SyncHttpClient for ReqwestHttpClient {
    async fn connect(&mut self, base_url: &str) -> Result<(), SyncHttpError> {
        let url = join_url(base_url, STATUS_PATH)?;
        let _parsed: url::Url = url.parse()?;
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(SyncHttpError::Status {
                code: resp.status().as_u16(),
                body: format!("{STATUS_PATH} returned HTTP {}", resp.status()),
            });
        }
        self.base_url = Some(base_url.trim_end_matches('/').to_string());
        Ok(())
    }

    async fn status(&self) -> Result<SyncStatus, SyncHttpError> {
        let url = join_url(self.require_connected()?, STATUS_PATH)?;
        let resp = self.http.get(&url).send().await?;
        let code = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !code.is_success() {
            return Err(SyncHttpError::Status {
                code: code.as_u16(),
                body,
            });
        }
        Ok(serde_json::from_str(&body)?)
    }

    async fn pull_since(&self, since: i64, limit: i64) -> Result<PullResponse, SyncHttpError> {
        let path = format!("{CHANGES_PATH}?since_version={since}&limit={limit}");
        let url = join_url(self.require_connected()?, &path)?;
        let resp = self.http.get(&url).send().await?;
        let code = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !code.is_success() {
            return Err(SyncHttpError::Status {
                code: code.as_u16(),
                body,
            });
        }
        Ok(serde_json::from_str(&body)?)
    }

    async fn pull_full(&self) -> Result<FullDump, SyncHttpError> {
        let url = join_url(self.require_connected()?, PULL_FULL_PATH)?;
        let resp = self.http.get(&url).send().await?;
        let code = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !code.is_success() {
            return Err(SyncHttpError::Status {
                code: code.as_u16(),
                body,
            });
        }
        Ok(serde_json::from_str(&body)?)
    }

    async fn push(&self, changes: Vec<SyncChange>) -> Result<PushResponse, SyncHttpError> {
        let url = join_url(self.require_connected()?, PUSH_PATH)?;
        let resp = self.http.post(&url).json(&changes).send().await?;
        let code = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !code.is_success() {
            return Err(SyncHttpError::Status {
                code: code.as_u16(),
                body,
            });
        }
        Ok(serde_json::from_str(&body)?)
    }

    async fn resolve_conflict(
        &self,
        conflict_id: i64,
        resolution: &str,
        merged: Option<&str>,
    ) -> Result<bool, SyncHttpError> {
        let path = format!("{CONFLICTS_PATH}/{conflict_id}/resolve");
        let url = join_url(self.require_connected()?, &path)?;
        let body = ResolveRequest {
            resolution: resolution.to_string(),
            merged_payload: merged.map(str::to_string),
        };
        let resp = self.http.post(&url).json(&body).send().await?;
        let code = resp.status();
        // The server answers 404 when the conflict no longer exists; that
        // is a valid "not resolved here" outcome rather than an error.
        if code.as_u16() == 404 {
            return Ok(false);
        }
        if !code.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(SyncHttpError::Status {
                code: code.as_u16(),
                body,
            });
        }
        Ok(true)
    }

    fn base_url(&self) -> Option<&str> {
        self.base_url.as_deref()
    }
}

/// The `reqwest` + [`SyncEngine`](livtet_sync::SyncEngine) session,
/// mirroring the source crate's `SyncClient`.
pub type ReqwestSyncClient = SyncSession<ReqwestHttpClient>;

// The inherent methods are declared on the concrete session type (not the
// `ReqwestSyncClient` alias) so the impl is unambiguously on a nominal type;
// they remain callable through the alias.
impl SyncSession<ReqwestHttpClient> {
    /// Build a session over `db` with a default [`ReqwestHttpClient`].
    pub fn with_default_http(db: &livtet_data::orm::DatabaseConnection, device_id: &str) -> Self {
        Self::new(db, device_id, ReqwestHttpClient::new())
    }

    /// Build a session with an externally supplied [`reqwest::Client`].
    pub fn with_http(
        db: &livtet_data::orm::DatabaseConnection,
        device_id: &str,
        http: reqwest::Client,
    ) -> Self {
        Self::new(db, device_id, ReqwestHttpClient::with_http(http))
    }
}
