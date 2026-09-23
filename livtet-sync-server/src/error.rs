//! Error type for the sync server daemon and library entry point.

use thiserror::Error;

/// Errors surfaced by [`crate::run`], the daemon loop, and the RPC layer.
#[derive(Debug, Error)]
pub enum ServerError {
    /// Low-level I/O failure (stdio, filesystem, process spawn).
    #[error("io error: {0}")]
    Io(String),

    /// Database / migration failure.
    #[error("database error: {0}")]
    Db(String),

    /// HTTP server bind / serve failure.
    #[error("http error: {0}")]
    Http(String),

    /// JSON-RPC encode/decode failure.
    #[error("rpc error: {0}")]
    Rpc(String),

    /// Invalid configuration or CLI arguments.
    #[error("config error: {0}")]
    Config(String),

    /// Server lifecycle failure.
    #[error("server error: {0}")]
    Server(String),
}

impl From<std::io::Error> for ServerError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

impl From<serde_json::Error> for ServerError {
    fn from(err: serde_json::Error) -> Self {
        Self::Rpc(err.to_string())
    }
}

impl From<livtet_sync::SyncError> for ServerError {
    fn from(err: livtet_sync::SyncError) -> Self {
        Self::Db(err.to_string())
    }
}
