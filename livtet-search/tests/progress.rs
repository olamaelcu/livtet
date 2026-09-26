//! Tests for the reindex progress hook.
//!
//! `SearchIndex::reindex_with_progress` must report a `Loading`
//! event while streaming rows from the DB, then `Indexing`
//! events whose final `done` equals `total` (editions +
//! authors).

use camino_tempfile::Utf8TempDir as TempDir;
use livtet_data::migration::Migrator;
use livtet_data::orm::{ActiveModelTrait, DatabaseConnection, Set};
use livtet_data::sql::{AssertSqlSafe, sqlite::SqlitePoolOptions};
use livtet_search::{ReindexEvent, SearchIndex};
use livtet_types::DbId;

async fn fresh_db() -> DatabaseConnection {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:?cache=private")
        .await
        .expect("connect to in-memory sqlite");
    livtet_data::sql::query(AssertSqlSafe("PRAGMA foreign_keys=ON"))
        .execute(&pool)
        .await
        .expect("enable foreign keys");
    Migrator::run(&pool).await.expect("run migrations");
    livtet_data::orm::SqlxSqliteConnector::from_sqlx_sqlite_pool(pool)
}

fn now_p() -> time::PrimitiveDateTime {
    let now = time::OffsetDateTime::now_utc();
    time::PrimitiveDateTime::new(now.date(), now.time())
}

/// One work, one edition, one author linked to the edition.
/// Expected index total: 1 edition doc + 1 author doc = 2.
async fn seed_minimal(db: &DatabaseConnection) {
    use livtet_data::entities::{authors, edition_authors, editions, works};

    let now = now_p();
    let work_id = DbId::new();
    let edition_id = DbId::new();
    let author_id = DbId::new();

    works::ActiveModel {
        id: Set(work_id),
        title: Set("Progress Test Work".into()),
        description: Set(None),
        sort_title: Set(None),
        series_type: Set(None),
        language_id: Set(None),
        created_at: Set(now),
        updated_at: Set(None),
        preferred_edition_id: Set(None),
    }
    .insert(db)
    .await
    .expect("insert work");

    editions::ActiveModel {
        id: Set(edition_id),
        work_id: Set(work_id),
        group_id: Set(None),
        title: Set(Some("Progress Test Edition".into())),
        published_date: Set(None),
        format_id: Set(None),
        language_id: Set(None),
        notes: Set(None),
        description: Set(None),
        format_metadata: Set(None),
        created_at: Set(now),
        updated_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert edition");

    authors::ActiveModel {
        id: Set(author_id),
        name: Set("Progress Author".into()),
    }
    .insert(db)
    .await
    .expect("insert author");

    edition_authors::ActiveModel {
        edition_id: Set(edition_id),
        author_id: Set(author_id),
        role: Set("author".into()),
    }
    .insert(db)
    .await
    .expect("insert edition_authors");
}

#[tokio::test]
async fn reindex_with_progress_ends_with_done_equals_total() {
    let db = fresh_db().await;
    seed_minimal(&db).await;
    let dir = TempDir::new().expect("tempdir");
    let index = SearchIndex::open(dir.path()).expect("open fresh index");

    let mut events = Vec::new();
    index
        .reindex_with_progress(&db, &mut |e| events.push(e))
        .await
        .expect("reindex with progress");

    assert!(
        events.iter().any(|e| matches!(e, ReindexEvent::Loading)),
        "expected at least one Loading event, got {events:?}"
    );
    let last = events.last().expect("at least one event");
    assert!(
        matches!(last, ReindexEvent::Indexing { done: 2, total: 2 }),
        "expected final Indexing {{ done: 2, total: 2 }}, got {last:?}"
    );
}

#[tokio::test]
async fn reindex_without_progress_still_works() {
    let db = fresh_db().await;
    seed_minimal(&db).await;
    let dir = TempDir::new().expect("tempdir");
    let index = SearchIndex::open(dir.path()).expect("open fresh index");

    index.reindex(&db).await.expect("plain reindex");
    let searcher = index.index().reader().expect("reader").searcher();
    assert_eq!(searcher.num_docs(), 2);
}
