//! Canonical error type for the sync protocol.
//!
//! `SyncError` is returned by `SyncEngine` and `SyncClient` in place
//! of `livtet_database::orm::DbErr`.  The `Db` variant carries the human-readable
//! error message (FK and composite primary-key violations are
//! pre-enhanced by `livtet_database::ConstraintViolation::enhance_db_err`)
//! plus the optional [`ConstraintViolation`] value when one is recognised.

use livtet_database::ConstraintViolation;
use livtet_types::DbId;
use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum SyncError {
    #[error("database error: {message}")]
    #[diagnostic(code(livtet_sync::db_error))]
    Db {
        message: String,
        violation: Option<ConstraintViolation>,
    },

    #[error("unknown sync entity type: {type_name}")]
    #[diagnostic(code(livtet_sync::unknown_entity_type))]
    UnknownEntityType { type_name: String },

    #[error("sync conflict on {entity_type}/{entity_id}: {message}")]
    #[diagnostic(code(livtet_sync::conflict))]
    Conflict {
        entity_type: String,
        entity_id: DbId,
        message: String,
    },

    #[error("serialization error for {entity_type}: {message}")]
    #[diagnostic(code(livtet_sync::serialization))]
    Serialization {
        entity_type: String,
        message: String,
    },

    #[error("deserialization error for {entity_type}: {message}")]
    #[diagnostic(code(livtet_sync::deserialization))]
    Deserialization {
        entity_type: String,
        message: String,
    },
}

impl From<livtet_database::orm::DbErr> for SyncError {
    fn from(err: livtet_database::orm::DbErr) -> Self {
        let (message, violation) = ConstraintViolation::enhance_db_err(err);
        Self::Db { message, violation }
    }
}

pub type Result<T> = miette::Result<T, SyncError>;

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;

    #[test]
    fn db_variant_display_includes_message() {
        let err = SyncError::Db {
            message: "constraint violation".to_string(),
            violation: None,
        };
        let msg = err.to_string();
        assert!(!msg.is_empty(), "Db Display must be non-empty");
        assert!(
            msg.contains("constraint violation"),
            "Db Display must include the inner message, got: {msg}"
        );
        assert!(
            msg.starts_with("database error:"),
            "Db Display must use the documented prefix, got: {msg}"
        );
    }

    #[test]
    fn unknown_entity_type_display_includes_type_name() {
        let err = SyncError::UnknownEntityType {
            type_name: "fake_table".to_string(),
        };
        let msg = err.to_string();
        assert!(
            !msg.is_empty(),
            "UnknownEntityType Display must be non-empty"
        );
        assert!(
            msg.contains("fake_table"),
            "UnknownEntityType Display must include the type_name, got: {msg}"
        );
    }

    #[test]
    fn conflict_display_includes_entity_type_id_and_message() {
        let err = SyncError::Conflict {
            entity_type: "work".to_string(),
            entity_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("DbId parses"),
            message: "version mismatch".to_string(),
        };
        let msg = err.to_string();
        assert!(!msg.is_empty(), "Conflict Display must be non-empty");
        assert!(
            msg.contains("work"),
            "Conflict Display must include entity_type, got: {msg}"
        );
        assert!(
            msg.contains("01ARZ3NDEKTSV4RRFFQ69G5FAV"),
            "Conflict Display must include entity_id, got: {msg}"
        );
        assert!(
            msg.contains("version mismatch"),
            "Conflict Display must include the message, got: {msg}"
        );
    }

    #[test]
    fn dberr_converts_via_from_into_db_variant() {
        fn returns_dberr() -> std::result::Result<(), livtet_database::orm::DbErr> {
            Err(livtet_database::orm::DbErr::RecordNotFound("missing".to_string()))
        }
        fn propagate() -> std::result::Result<(), SyncError> {
            returns_dberr()?;
            Ok(())
        }

        let err = propagate().expect_err("DbErr must propagate as SyncError");
        match &err {
            SyncError::Db { message, .. } => {
                assert!(
                    message.contains("missing"),
                    "wrapped DbErr must preserve its message, got: {message}"
                );
            }
            other => panic!("expected SyncError::Db, got {other:?}"),
        }

        let dberr = livtet_database::orm::DbErr::Custom("explicit".to_string());
        let converted: SyncError = dberr.into();
        assert_matches!(converted, SyncError::Db { .. });
    }
}
