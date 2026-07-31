//! Regression guard for the "missing migration file" panic.
//!
//! Background: commit `a2cdd51` (which added `m0012_merge_edition_files`)
//! removed `m0010_edition_files` from the migration registry while
//! existing developer databases still had `core-0010-edition_files`
//! recorded as applied in `core_migrations`. Sea-orm-migration errors
//! when a recorded migration isn't in the `migrations()` vec:
//!
//!     Migration file of version 'core-0010-edition_files' is missing,
//!     this migration has been applied but its file is missing
//!
//! The fix restores `m0010_edition_files` in the registry. These tests
//! pin the invariant so a future "cleanup" doesn't silently drop it
//! again.

use livtet_data::orm::FromQueryResult;
use livtet_data::TestDb;

#[derive(FromQueryResult)]
struct MigrationRow {
    version: String,
}

#[derive(Debug, FromQueryResult)]
struct TableName {
    #[allow(dead_code)]
    name: String,
}

/// A fresh TestDb must record `core-0010-edition_files` as applied in
/// `core_migrations`. If this migration is ever removed from the
/// `migrations()` vec again, sea-orm-migration will refuse to run on
/// databases that already have it recorded — the exact regression
/// this guards against.
#[tokio::test]
async fn m0010_edition_files_is_recorded_as_applied() {
    let test_db = TestDb::new(None).await.unwrap();
    let db = test_db.state().db_conn();

    let row = MigrationRow::find_by_statement(sea_orm::Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Sqlite,
        "SELECT version FROM core_migrations WHERE version = ?",
        ["core-0010-edition_files".into()],
    ))
    .one(&db)
    .await
    .expect("core_migrations query must not error")
    .expect("core-0010-edition_files must be recorded as applied");

    assert_eq!(
        row.version, "core-0010-edition_files",
        "m0010_edition_files must remain in the migration registry so \
         existing databases that previously applied it can migrate \
         forward without sea-orm-migration refusing to run"
    );
}

/// The `edition_files` table must NOT exist after migrations complete,
/// because `m0012_merge_edition_files` drops it. This confirms the
/// merge migration still wins over m0010's table creation on a fresh
/// database (m0010 creates the table, m0012 drops it — idempotently
/// guarded by `has_table`).
#[tokio::test]
async fn edition_files_table_is_dropped_after_migration() {
    let test_db = TestDb::new(None).await.unwrap();
    let db = test_db.state().db_conn();

    let rows = TableName::find_by_statement(sea_orm::Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Sqlite,
        "SELECT name FROM sqlite_master WHERE type='table' AND name='edition_files'",
        [],
    ))
    .all(&db)
    .await
    .expect("sqlite_master query must not error");

    assert!(
        rows.is_empty(),
        "edition_files table should be dropped by m0012; found rows: {rows:?}"
    );
}
