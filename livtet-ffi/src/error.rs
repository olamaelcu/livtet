//! The single error type crossing the FFI boundary.
//!
//! Internal error types (`CoreError`, `SearchError`, `sqlx::Error`,
//! tantivy errors) are squashed into these variants — no internal type
//! leaks into the foreign binding surface.

use livtet_data::CoreError;
use livtet_search::index::SearchError;
use thiserror::Error;

/// Errors surfaced to Kotlin/Swift consumers.
#[derive(Debug, Error, uniffi::Error)]
pub enum LivtetError {
    /// A SeaORM / sqlx database failure. See
    /// [`ConstraintViolation`](livtet_data::ConstraintViolation) for how
    /// these messages are produced.
    #[error("database error: {0}")]
    Database(String),

    /// A Tantivy / search-index failure.
    #[error("search error: {0}")]
    Search(String),

    /// The referenced entity does not exist.
    #[error("not found: {entity} with id {id}")]
    NotFound { entity: String, id: String },

    /// Caller-supplied input failed validation (malformed ISBNs,
    /// URN strings, enum names, ...).
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

impl From<CoreError> for LivtetError {
    fn from(err: CoreError) -> Self {
        match err {
            CoreError::Database(e) => Self::Database(e.to_string()),
            CoreError::DatabaseError(msg) => Self::Database(msg),
            CoreError::NotFound { entity, id } => Self::NotFound { entity, id },
            CoreError::InvalidInput(msg) => Self::InvalidInput(msg),
            // The FFI facade carries its state on `LivtetStore`; these
            // global-state variants cannot occur here in practice.
            // Map them fail-closed anyway.
            CoreError::NotInitialized => Self::Database("internal state not ready".to_string()),
            CoreError::AlreadyInitialized => Self::Database("already initialized".to_string()),
        }
    }
}

impl From<SearchError> for LivtetError {
    fn from(err: SearchError) -> Self {
        Self::Search(err.to_string())
    }
}

impl From<livtet_data::sql::Error> for LivtetError {
    fn from(err: livtet_data::sql::Error) -> Self {
        Self::Database(err.to_string())
    }
}

impl From<livtet_data::orm::DbErr> for LivtetError {
    fn from(err: livtet_data::orm::DbErr) -> Self {
        CoreError::from(err).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_error_not_found_maps_structurally() {
        let err = CoreError::NotFound {
            entity: "editions".into(),
            id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
        };
        match LivtetError::from(err) {
            LivtetError::NotFound { entity, id } => {
                assert_eq!(entity, "editions");
                assert_eq!(id, "01ARZ3NDEKTSV4RRFFQ69G5FAV");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn core_error_invalid_input_maps_message() {
        let err = CoreError::InvalidInput("bad isbn".into());
        match LivtetError::from(err) {
            LivtetError::InvalidInput(msg) => assert_eq!(msg, "bad isbn"),
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn db_err_maps_through_core_error() {
        let err = livtet_data::orm::DbErr::Custom("constraint x".into());
        match LivtetError::from(err) {
            LivtetError::Database(msg) => assert!(msg.contains("constraint x")),
            other => panic!("expected Database, got {other:?}"),
        }
    }
}
