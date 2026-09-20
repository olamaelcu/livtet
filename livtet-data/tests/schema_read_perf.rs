//! Schema-invariant guards for the read-performance schema work.
//!
//! Pins three properties of the migrated schema:
//! - junction tables are `WITHOUT ROWID` (PK is the clustered index),
//! - `ANALYZE` has produced planner statistics,
//! - hot lookup paths use the new secondary indexes instead of scans.

use livtet_data::sql::AssertSqlSafe;
use livtet_data::{Kind, TestDb};
use sqlx::Row;

async fn query_col(pool: &sqlx::SqlitePool, sql: &str, col: &str) -> Vec<String> {
    sqlx::query(AssertSqlSafe(sql.to_string()))
        .fetch_all(pool)
        .await
        .expect("query must run")
        .iter()
        .filter_map(|row| row.try_get::<Option<String>, _>(col).ok().flatten())
        .collect()
}

#[tokio::test]
async fn junction_tables_are_without_rowid() {
    let test_db = TestDb::new(&[Kind::Business]).await.unwrap();

    for table in [
        "work_authors",
        "work_tags",
        "edition_identifiers",
        "series_entries",
        "edition_group_identifiers",
        "reading_list_book",
    ] {
        let ddl = query_col(
            &test_db.pool,
            &format!("SELECT sql FROM sqlite_master WHERE type='table' AND name='{table}'"),
            "sql",
        )
        .await
        .pop()
        .unwrap_or_else(|| panic!("table {table} must exist"));
        assert!(
            ddl.contains("WITHOUT ROWID"),
            "{table} must be created WITHOUT ROWID, got: {ddl}"
        );
        assert!(ddl.contains("STRICT"), "{table} must remain STRICT");
    }
}

#[tokio::test]
async fn analyze_stats_are_collected() {
    let test_db = TestDb::new(&[Kind::Business]).await.unwrap();

    // Only tables with rows land in sqlite_stat1; the seeded
    // vocabulary tables are the reliable witnesses.
    let rows = query_col(
        &test_db.pool,
        "SELECT DISTINCT tbl FROM sqlite_stat1 WHERE tbl IN ('languages', 'genres', 'subjects')",
        "tbl",
    )
    .await;
    assert_eq!(
        rows.len(),
        3,
        "ANALYZE must have produced statistics; got {rows:?}"
    );
}

#[tokio::test]
async fn hot_paths_use_indexes() {
    let test_db = TestDb::new(&[Kind::Business]).await.unwrap();

    let cases: &[(&str, &str)] = &[
        (
            "EXPLAIN QUERY PLAN SELECT id FROM editions WHERE work_id = 'x'",
            "idx_editions_work_id",
        ),
        (
            "EXPLAIN QUERY PLAN SELECT work_id FROM work_tags WHERE tag_id = 'x'",
            "idx_work_tags_tag_id",
        ),
        (
            "EXPLAIN QUERY PLAN SELECT id FROM authors WHERE name = 'x'",
            "idx_authors_name",
        ),
        (
            "EXPLAIN QUERY PLAN SELECT id FROM annotations WHERE edition_id = 'x'",
            "idx_annotations_edition_id",
        ),
    ];

    for (sql, expected_index) in cases {
        let plan = query_col(&test_db.pool, sql, "detail").await.join(" | ");
        assert!(
            plan.contains(expected_index),
            "expected {expected_index} in plan for `{sql}`; got: {plan}"
        );
    }
}
