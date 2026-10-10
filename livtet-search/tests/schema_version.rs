//! Schema-version sidecar handling.
//!
//! `SearchIndex::open` must reject an on-disk schema version that doesn't
//! match the compiled `SCHEMA_VERSION`, and `migrate_to_with_progress` must
//! reject one that is strictly newer. A malformed sidecar is a hard error
//! rather than a silent fallback. These drive the guard branches that the
//! happy-path fixtures in `integration.rs` never reach.

use camino_tempfile::Utf8TempDir as TempDir;
use livtet_data::orm::{DatabaseConnection, SqlxSqliteConnector};
use livtet_data::sql::sqlite::SqlitePoolOptions;
use livtet_search::{ReindexEvent, SearchError, SearchIndex};

const META_FILE: &str = "search_schema_version.json";

fn tmp() -> TempDir {
    camino_tempfile::tempdir().expect("tempdir")
}

/// A connection with no migrations: the version guard short-circuits before
/// any query runs, so the schema need not exist.
async fn bare_db() -> DatabaseConnection {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:?cache=private")
        .await
        .expect("connect to in-memory sqlite");
    SqlxSqliteConnector::from_sqlx_sqlite_pool(pool)
}

#[test]
fn open_rejects_a_mismatched_schema_version() {
    let dir = tmp();
    std::fs::write(dir.path().join(META_FILE), br#"{"schema_version": 99}"#)
        .expect("write sidecar");
    // `SearchIndex` is not `Debug`, so match the `Result` rather than unwrap it.
    assert!(matches!(
        SearchIndex::open(dir.path()),
        Err(SearchError::SchemaVersionMismatch { found: 99, .. })
    ));
}

#[test]
fn open_rejects_a_malformed_sidecar() {
    let dir = tmp();
    std::fs::write(dir.path().join(META_FILE), b"{ this is not json").expect("write sidecar");
    assert!(
        SearchIndex::open(dir.path()).is_err(),
        "a malformed sidecar must be a hard error, not a silent fallback"
    );
}

#[tokio::test]
async fn migrate_to_rejects_a_newer_on_disk_version() {
    let dir = tmp();
    std::fs::write(dir.path().join(META_FILE), br#"{"schema_version": 99}"#)
        .expect("write sidecar");
    let db = bare_db().await;
    let mut sink = |_e: ReindexEvent| {};
    let err = SearchIndex::migrate_to_with_progress(dir.path(), &db, &mut sink)
        .await
        .expect_err("a newer on-disk version must be rejected");
    assert!(matches!(
        err,
        SearchError::SchemaVersionMismatch { found: 99, .. }
    ));
}
