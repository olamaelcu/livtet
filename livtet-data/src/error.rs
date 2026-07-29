use crate::ConstraintViolation;
use miette::Diagnostic;
use thiserror::Error;

#[derive(Error, Debug, Diagnostic)]
pub enum CoreError {
    #[error("Database error: {0}")]
    #[diagnostic(code(livtet_core::db_error))]
    Database(sea_orm::DbErr),

    /// Generic database error message — used by callers that have
    /// a non-sea-orm `Error` type (e.g. `rusqlite::Error` from the
    /// session module) and want to surface it through the same
    /// `CoreError` surface. Prefer `Database(sea_orm::DbErr)` when
    /// the underlying error is convertible.
    #[error("Database error: {0}")]
    #[diagnostic(code(livtet_core::db_error))]
    DatabaseError(String),

    #[error("Not found: {entity} with id {id}")]
    #[help("Check that the entity exists and the id is correct")]
    #[diagnostic(code(livtet_core::not_found))]
    NotFound { entity: String, id: String },

    #[error("Invalid input: {0}")]
    #[help("Check the API documentation for valid input formats")]
    #[diagnostic(code(livtet_core::invalid_input))]
    InvalidInput(String),

    #[error("Not initialized — call init_state() first")]
    #[help("Call livtet_core::init_state(state) before using any other functions")]
    #[diagnostic(code(livtet_core::not_initialized))]
    NotInitialized,

    #[error("Already initialized — state can only be set once")]
    #[diagnostic(code(livtet_core::already_initialized))]
    AlreadyInitialized,
}

impl From<sea_orm::DbErr> for CoreError {
    fn from(err: sea_orm::DbErr) -> Self {
        let (message, _violation) = ConstraintViolation::enhance_db_err(err);
        Self::Database(sea_orm::DbErr::Custom(message))
    }
}

pub type Result<T> = miette::Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_error_not_found_display() {
        let err = CoreError::NotFound {
            entity: "Book".to_string(),
            id: "123".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("Not found"));
        assert!(msg.contains("Book"));
        assert!(msg.contains("123"));
    }

    #[test]
    fn core_error_invalid_input_display() {
        let err = CoreError::InvalidInput("ISBN is required".to_string());
        let msg = err.to_string();
        assert_eq!(msg, "Invalid input: ISBN is required");
    }

    #[test]
    fn core_error_not_found_variant_is_debug_and_display() {
        let err = CoreError::NotFound {
            entity: "test".to_string(),
            id: "entity".to_string(),
        };
        let display = format!("{}", err);
        let debug = format!("{:?}", err);
        assert!(display.contains("Not found"));
        assert!(debug.contains("NotFound"));
    }

    #[test]
    fn core_error_invalid_input_variant_is_debug_and_display() {
        let err = CoreError::InvalidInput("invalid value".to_string());
        let display = format!("{}", err);
        let debug = format!("{:?}", err);
        assert!(display.contains("Invalid input"));
        assert!(debug.contains("InvalidInput"));
    }

    #[test]
    fn core_error_not_initialized_has_help() {
        let err = CoreError::NotInitialized;
        let msg = err.to_string();
        assert!(msg.contains("Not initialized"));
    }

    #[test]
    fn core_error_already_initialized() {
        let err = CoreError::AlreadyInitialized;
        let msg = err.to_string();
        assert!(msg.contains("Already initialized"));
    }
}
