//! The shelf tables predate the migrator.
//!
//! They first shipped as ad-hoc `CREATE TABLE` statements outside a
//! migration, and the write path keyed rows by `(shelf, book)`, so a book that
//! changed shelves left one row behind per shelf it had visited. Migration
//! `m0013` has to adopt that database: keep the tables, collapse the rows, and
//! put the "one shelving row per book" constraint on.

use std::str::FromStr;

use livtet_data::sql::{SqlitePool, sqlite::SqliteConnectOptions};
use livtet_data::{migration, sqlite_pool_options};

const DID: &str = "did:plc:e2ctbutx6kya6si4if5ngjmm";
const BOOK_A: &str =
    "at://did:web:biblio.livtet.olamaelcu.net/community.lexicon.book.edition/ol.OL997340M";
const BOOK_B: &str =
    "at://did:web:biblio.livtet.olamaelcu.net/community.lexicon.book.edition/ol.OL994666M";

const LEGACY_DDL: [&str; 3] = [
    "CREATE TABLE shelf (
        uri TEXT PRIMARY KEY NOT NULL,
        name TEXT NOT NULL,
        description TEXT,
        created_at TEXT NOT NULL,
        updated_at TEXT
    )",
    "CREATE TABLE book_shelf (
        uri TEXT PRIMARY KEY NOT NULL,
        shelf_uri TEXT NOT NULL REFERENCES shelf(uri),
        book_uri TEXT NOT NULL,
        status TEXT NOT NULL,
        position INTEGER,
        notes TEXT,
        emoji TEXT,
        created_at TEXT NOT NULL,
        updated_at TEXT,
        did TEXT NOT NULL
    )",
    "CREATE INDEX idx_book_shelf_book ON book_shelf(book_uri)",
];

async fn legacy_pool(name: &str) -> (SqlitePool, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "livtet-shelf-migration-{name}-{}.db",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let opts = SqliteConnectOptions::from_str(&format!("sqlite:{}", path.display()))
        .expect("sqlite url")
        .create_if_missing(true);
    let pool = sqlite_pool_options()
        .connect_with(opts)
        .await
        .expect("open the fixture database");
    (pool, path)
}

/// The tables exactly as the pre-migration write path left them: plain
/// (non-STRICT) tables, no unique index, and one row per shelf visited.
async fn seed_legacy_shelf_tables(pool: &SqlitePool) {
    for statement in LEGACY_DDL {
        livtet_data::sql::query(statement)
            .execute(pool)
            .await
            .expect("legacy DDL");
    }

    // Book A has visited three shelves; book B one.
    for (book, status) in [
        (BOOK_A, "to-read"),
        (BOOK_A, "reading"),
        (BOOK_A, "dnf"),
        (BOOK_B, "to-read"),
    ] {
        livtet_data::sql::query(
            "INSERT OR IGNORE INTO shelf (uri, name, created_at)
             VALUES (?, ?, '2026-10-24T00:00:00Z')",
        )
        .bind(format!("shelf:{status}"))
        .bind(status)
        .execute(pool)
        .await
        .expect("legacy shelf row");
        livtet_data::sql::query(
            "INSERT INTO book_shelf (uri, shelf_uri, book_uri, status, created_at, did)
             VALUES (?, ?, ?, ?, '2026-10-24T00:00:00Z', ?)",
        )
        .bind(format!("{book}:{status}"))
        .bind(format!("shelf:{status}"))
        .bind(book)
        .bind(status)
        .bind(DID)
        .execute(pool)
        .await
        .expect("legacy shelving row");
    }
}

#[tokio::test]
async fn legacy_shelf_tables_collapse_to_one_row_per_book() {
    let (pool, path) = legacy_pool("collapse").await;
    seed_legacy_shelf_tables(&pool).await;

    migration::Migrator::run(&pool).await.expect("migrate up");

    let rows: i64 = livtet_data::sql::query_scalar("SELECT COUNT(*) FROM book_shelf")
        .fetch_one(&pool)
        .await
        .expect("count rows");
    assert_eq!(rows, 2, "one surviving shelving row per book");

    // The newest row per book wins, so book A reports the last shelf it was put on.
    let status: String =
        livtet_data::sql::query_scalar("SELECT status FROM book_shelf WHERE book_uri = ?")
            .bind(BOOK_A)
            .fetch_one(&pool)
            .await
            .expect("book A row");
    assert_eq!(status, "dnf");

    let unique_indexes: i64 = livtet_data::sql::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master \
         WHERE type = 'index' AND name = 'uq_book_shelf_did_book_uri'",
    )
    .fetch_one(&pool)
    .await
    .expect("count indexes");
    assert_eq!(
        unique_indexes, 1,
        "the migration adds the per-book constraint"
    );

    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn a_second_row_for_the_same_book_is_rejected() {
    let (pool, path) = legacy_pool("unique").await;
    seed_legacy_shelf_tables(&pool).await;
    migration::Migrator::run(&pool).await.expect("migrate up");

    let duplicate = livtet_data::sql::query(
        "INSERT INTO book_shelf (uri, shelf_uri, book_uri, status, created_at, did)
         VALUES ('duplicate', 'shelf:reading', ?, 'reading', '2026-10-25T00:00:00Z', ?)",
    )
    .bind(BOOK_A)
    .bind(DID)
    .execute(&pool)
    .await;

    assert!(
        duplicate.is_err(),
        "a book must not hold two shelving rows for one account"
    );

    let _ = std::fs::remove_file(path);
}
