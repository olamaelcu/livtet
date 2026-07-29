//! HTTP error mapping for the sync protocol.
//!
//! `SyncError` is the canonical error type returned by `SyncEngine`.
//! Poem handlers need to bubble it up to a 4xx/5xx response; this
//! module provides the mapping via a newtype wrapper `ApiError` so
//! the orphan rule is satisfied (Rust does not allow implementing
//! a foreign trait for a foreign type).
//!
//! Mapping (per the agreed spec):
//! - `Db { .. }`                            → 500 Internal Server Error
//! - `UnknownEntityType { .. }`             → 400 Bad Request
//! - `Conflict { .. }`                      → 409 Conflict

use poem::{error::ResponseError, http::StatusCode};

use crate::types::SyncError;

/// Newtype wrapper around `SyncError` that carries the
/// `ResponseError` impl.  Handlers that need to bubble a sync
/// error into poem should `.map_err(ApiError::from)?` at the
/// boundary.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct ApiError(pub SyncError);

impl From<SyncError> for ApiError {
    fn from(err: SyncError) -> Self {
        Self(err)
    }
}

impl ResponseError for ApiError {
    fn status(&self) -> StatusCode {
        match &self.0 {
            SyncError::UnknownEntityType { .. } => StatusCode::BAD_REQUEST,
            SyncError::Conflict { .. } => StatusCode::CONFLICT,
            SyncError::Db { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            SyncError::Serialization { .. } | SyncError::Deserialization { .. } => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}
