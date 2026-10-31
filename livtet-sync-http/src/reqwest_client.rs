//! `reqwest` implementation of [`SyncHttpClient`].

use std::time::Duration;

use async_trait::async_trait;
use livtet_sync::{FullDump, PullResponse, PushResponse, SyncChange, SyncStatus};

use crate::{
    client::SyncHttpClient,
    error::SyncHttpError,
    routes::{
        CHANGES_PATH, CONFLICTS_PATH, PULL_FULL_PATH, PUSH_PATH, ResolveRequest, SESSION_HEADER,
        STATUS_PATH, join_url,
    },
    session::SyncSession,
};

/// The default request timeout, matching the source client.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// A [`SyncHttpClient`] backed by `reqwest`.
pub struct ReqwestHttpClient {
    base_url: Option<String>,
    http: reqwest::Client,
    /// Session token minted at pairing approval; sent on every request.
    session_token: Option<String>,
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
            session_token: None,
        }
    }

    /// Attach the session token minted when this device was approved.
    ///
    /// Must be set before [`SyncHttpClient::connect`], which validates the
    /// session against `GET /sync/status`.
    pub fn with_session_token(mut self, token: impl Into<String>) -> Self {
        self.session_token = Some(token.into());
        self
    }

    /// Connect and authenticate in one step.
    pub async fn connect_with_session(
        &mut self,
        base_url: &str,
        token: impl Into<String>,
    ) -> Result<(), SyncHttpError> {
        self.session_token = Some(token.into());
        self.connect(base_url).await
    }

    /// Add the session header, when a token is present.
    fn auth(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self.session_token.as_deref() {
            Some(token) => request.header(SESSION_HEADER, token),
            None => request,
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
        let resp = self.auth(self.http.get(&url)).send().await?;
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
        let resp = self.auth(self.http.get(&url)).send().await?;
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
        let resp = self.auth(self.http.get(&url)).send().await?;
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
        let resp = self.auth(self.http.get(&url)).send().await?;
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
        let resp = self
            .auth(self.http.post(&url))
            .json(&changes)
            .send()
            .await?;
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
        let resp = self.auth(self.http.post(&url)).json(&body).send().await?;
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

    /// Connect and authenticate with the session token minted at pairing
    /// approval.
    pub async fn connect_with_session(
        &mut self,
        base_url: &str,
        token: impl Into<String>,
    ) -> Result<(), SyncHttpError> {
        self.client_mut()
            .connect_with_session(base_url, token)
            .await?;
        let _ = self.engine().get_latest_version().await?;
        Ok(())
    }
}
