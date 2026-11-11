//! Small key/value preferences that belong to the library, not a screen.
//!
//! Reader display choices are per installation and must survive the app being
//! killed, which is what `client_settings` exists for: the daemon already keeps
//! its pull cursors there. Storing them here rather than in a preferences file
//! means one store to back up, inspect and reset, and it is the same table the
//! cursor work already writes, so there is no second convention.
//!
//! Keys are owned by the caller. `sync.cursor.*` belongs to sync; the reader uses
//! `reader.display`.

use livtet_core::data::SharedState;
use livtet_core::data::migrator::Kind;
use livtet_core::data::sql::Row;

use crate::error::LivtetError;

/// Read a stored preference, or `None` when it was never set.
#[uniffi::export(async_runtime = "tokio")]
pub async fn settings_get(db_path: String, key: String) -> Result<Option<String>, LivtetError> {
    let state = SharedState::connect(&db_path, &[Kind::Business, Kind::Client])
        .await
        .map_err(|error| LivtetError::Database(error.to_string()))?;
    let row = livtet_core::data::sql::query("SELECT value FROM client_settings WHERE key = ?")
        .bind(key)
        .fetch_optional(&state.pool)
        .await
        .map_err(|error| LivtetError::Database(error.to_string()))?;

    Ok(row.and_then(|row| row.try_get::<String, _>("value").ok()))
}

/// Store a preference, replacing whatever was there.
#[uniffi::export(async_runtime = "tokio")]
pub async fn settings_set(db_path: String, key: String, value: String) -> Result<(), LivtetError> {
    let state = SharedState::connect(&db_path, &[Kind::Business, Kind::Client])
        .await
        .map_err(|error| LivtetError::Database(error.to_string()))?;
    livtet_core::data::sql::query(
        "INSERT INTO client_settings (key, value, updated_at) VALUES (?, ?, datetime('now')) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(key)
    .bind(value)
    .execute(&state.pool)
    .await
    .map_err(|error| LivtetError::Database(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[tokio::test]
    async fn a_preference_round_trips_and_replaces() {
        let tmp = camino_tempfile::tempdir().unwrap();
        let db = tmp.path().join("livtet.db").to_string();

        assert_eq!(
            settings_get(db.clone(), "reader.display".to_string())
                .await
                .unwrap(),
            None
        );

        settings_set(
            db.clone(),
            "reader.display".to_string(),
            "SEPIA|1.1|1".to_string(),
        )
        .await
        .unwrap();
        assert_eq!(
            settings_get(db.clone(), "reader.display".to_string())
                .await
                .unwrap(),
            Some("SEPIA|1.1|1".to_string())
        );

        // A second write replaces rather than duplicating the key.
        settings_set(
            db.clone(),
            "reader.display".to_string(),
            "DARK|0.9|2".to_string(),
        )
        .await
        .unwrap();
        assert_eq!(
            settings_get(db, "reader.display".to_string())
                .await
                .unwrap(),
            Some("DARK|0.9|2".to_string())
        );
    }

    #[tokio::test]
    async fn keys_do_not_collide() {
        let tmp = camino_tempfile::tempdir().unwrap();
        let db = tmp.path().join("livtet.db").to_string();

        settings_set(db.clone(), "sync.cursor.abc".to_string(), "12".to_string())
            .await
            .unwrap();
        settings_set(
            db.clone(),
            "reader.display".to_string(),
            "LIGHT|1.0|0".to_string(),
        )
        .await
        .unwrap();

        assert_eq!(
            settings_get(db.clone(), "sync.cursor.abc".to_string())
                .await
                .unwrap(),
            Some("12".to_string())
        );
        assert_eq!(
            settings_get(db, "reader.display".to_string())
                .await
                .unwrap(),
            Some("LIGHT|1.0|0".to_string())
        );
    }
}
