//! Transport-level error type for the sync HTTP layer.

use thiserror::Error;

use livtet_sync::SyncError;

/// Errors raised by the HTTP transport, independent of the backend.
///
/// Replaces the source crate's `ClientError`, which wrapped
/// `reqwest::Error` directly; here the transport backend is erased so the
/// generic layer stays dependency-free.
#[derive(Debug, Error)]
pub enum SyncHttpError {
    /// An engine/domain error bubbling up from `livtet-sync`.
    #[error(transparent)]
    Sync(#[from] SyncError),

    /// No base URL has been set (call `connect` first).
    #[error("not connected: call connect first")]
    NotConnected,

    /// The server returned a non-success HTTP status.
    #[error("server returned HTTP {code}: {body}")]
    Status { code: u16, body: String },

    /// The request itself failed (connection, timeout, body).
    #[error("transport error: {0}")]
    Transport(String),

    /// A success response body could not be deserialized.
    #[error("failed to deserialize response: {0}")]
    Deserialize(String),

    /// The base URL was invalid.
    #[error("invalid base url: {0}")]
    Url(String),
}

impl From<serde_json::Error> for SyncHttpError {
    fn from(err: serde_json::Error) -> Self {
        Self::Deserialize(err.to_string())
    }
}

#[cfg(feature = "reqwest")]
impl From<reqwest::Error> for SyncHttpError {
    fn from(err: reqwest::Error) -> Self {
        Self::Transport(err.to_string())
    }
}

#[cfg(feature = "reqwest")]
impl From<url::ParseError> for SyncHttpError {
    fn from(err: url::ParseError) -> Self {
        Self::Url(err.to_string())
    }
}
