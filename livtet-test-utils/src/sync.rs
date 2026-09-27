//! Fixtures for sync-engine and sync-transport tests.
//!
//! A [`Device`] is one simulated Livtet install: a temp-file SQLite
//! database migrated with both the business and client schemas (so the
//! `change_log` audit triggers are live) and a [`SyncEngine`] over it.
//! Fixture data is written through the real schema, so every row the
//! tests create is logged by the triggers exactly as it would be in the
//! app.

use livtet_data::{
    Kind, SeedConfig, TestDb,
    orm::{DatabaseConnection, SqlxSqliteConnector},
    seed_database,
    sql::{self, AssertSqlSafe, SqlitePool},
};
use livtet_sync::{SyncChange, SyncEngine};

/// One `change_log` row, read back verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeLogRow {
    pub id: i64,
    pub entity_type: String,
    pub entity_id: String,
    pub operation: String,
    pub version: i64,
    pub payload: String,
    pub device_id: String,
}

/// A seed configuration that populates every syncable table the seeder
/// knows about (every optional percentage pinned to 100%), kept small so
/// tests stay fast.
pub fn full_coverage_seed() -> SeedConfig {
    SeedConfig {
        num_works: 4,
        editions_per_work: (2, 2),
        actively_reading_pct: 0.5,
        did_not_finish_pct: 0.25,
        edition_group_pct: 1.0,
        annotation_pct: 1.0,
        loan_pct: 1.0,
        digital_inventory_pct: 1.0,
    }
}

/// `(table, columns, values)` inserting one row into each link table from
/// rows the seeder always creates.
const LINK_TABLE_TOP_UP: &[(&str, &str, &str)] = &[
    (
        "work_authors",
        "work_id, author_id, role",
        "(SELECT id FROM works LIMIT 1), (SELECT id FROM authors LIMIT 1), 'author'",
    ),
    (
        "work_tags",
        "work_id, tag_id",
        "(SELECT id FROM works LIMIT 1), (SELECT id FROM tags LIMIT 1)",
    ),
    (
        "work_genres",
        "work_id, genre_id",
        "(SELECT id FROM works LIMIT 1), (SELECT id FROM genres LIMIT 1)",
    ),
    (
        "work_subjects",
        "work_id, subject_id",
        "(SELECT id FROM works LIMIT 1), (SELECT id FROM subjects LIMIT 1)",
    ),
    (
        "work_publishers",
        "work_id, publisher_id",
        "(SELECT id FROM works LIMIT 1), (SELECT id FROM publishers LIMIT 1)",
    ),
    (
        "edition_authors",
        "edition_id, author_id, role",
        "(SELECT id FROM editions LIMIT 1), (SELECT id FROM authors LIMIT 1), 'author'",
    ),
    (
        "edition_tags",
        "edition_id, tag_id",
        "(SELECT id FROM editions LIMIT 1), (SELECT id FROM tags LIMIT 1)",
    ),
    (
        "edition_genres",
        "edition_id, genre_id",
        "(SELECT id FROM editions LIMIT 1), (SELECT id FROM genres LIMIT 1)",
    ),
    (
        "edition_subjects",
        "edition_id, subject_id",
        "(SELECT id FROM editions LIMIT 1), (SELECT id FROM subjects LIMIT 1)",
    ),
    (
        "edition_publishers",
        "edition_id, publisher_id",
        "(SELECT id FROM editions LIMIT 1), (SELECT id FROM publishers LIMIT 1)",
    ),
];

/// Build a [`SyncChange`] as a remote device would send it.
pub fn change(
    entity_type: &str,
    entity_id: &str,
    operation: &str,
    version: i64,
    payload: &str,
) -> SyncChange {
    SyncChange {
        id: 0,
        entity_type: entity_type.to_string(),
        entity_id: entity_id.to_string(),
        operation: operation.to_string(),
        version,
        payload: payload.to_string(),
        changed_at: String::new(),
        device_id: "remote".to_string(),
    }
}

/// One simulated device: a migrated temp database plus its sync engine.
pub struct Device {
    pub conn: DatabaseConnection,
    pub engine: SyncEngine,
    db: TestDb,
}

impl Device {
    /// An empty device with business + client migrations applied.
    pub async fn new(device_id: &str) -> Self {
        let db = TestDb::new(&[Kind::Business, Kind::Client])
            .await
            .expect("migrate temp database");
        let conn = SqlxSqliteConnector::from_sqlx_sqlite_pool(db.pool.clone());
        let engine = SyncEngine::new(conn.clone(), device_id.to_string());
        Self { conn, engine, db }
    }

    /// A device seeded so that every syncable table holds at least one
    /// row. The seeder picks link rows at random (and never links
    /// editions to genres or subjects), so any link table it left empty
    /// gets one row here; the result is deterministic in shape.
    pub async fn seeded(device_id: &str) -> Self {
        let device = Self::new(device_id).await;
        seed_database(&device.conn, &full_coverage_seed())
            .await
            .expect("seed database");
        for (table, columns, values) in LINK_TABLE_TOP_UP {
            if device.count(table).await == 0 {
                device
                    .execute(&format!(
                        "INSERT INTO {table} ({columns}) VALUES ({values})"
                    ))
                    .await;
            }
        }
        device
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.db.pool
    }

    /// Run a statement built by the test itself (never from input).
    pub async fn execute(&self, statement: &str) -> u64 {
        sql::query(AssertSqlSafe(statement.to_string()))
            .execute(self.pool())
            .await
            .unwrap_or_else(|e| panic!("{statement}: {e}"))
            .rows_affected()
    }

    /// `SELECT COUNT(*)` over a test-chosen table or subquery.
    pub async fn count(&self, from: &str) -> i64 {
        let statement = format!("SELECT COUNT(*) FROM {from}");
        sql::query_scalar(AssertSqlSafe(statement.clone()))
            .fetch_one(self.pool())
            .await
            .unwrap_or_else(|e| panic!("{statement}: {e}"))
    }

    /// Every `change_log` row in version order.
    pub async fn change_log(&self) -> Vec<ChangeLogRow> {
        self.change_log_where("1 = 1", &[]).await
    }

    /// `change_log` rows for one entity type and operation.
    pub async fn logged(&self, entity_type: &str, operation: &str) -> Vec<ChangeLogRow> {
        self.change_log_where(
            "entity_type = ? AND operation = ?",
            &[entity_type, operation],
        )
        .await
    }

    async fn change_log_where(&self, filter: &str, binds: &[&str]) -> Vec<ChangeLogRow> {
        type Row = (i64, String, String, String, i64, String, String);
        let statement = format!(
            "SELECT id, entity_type, entity_id, operation, version, payload, device_id \
             FROM change_log WHERE {filter} ORDER BY version, id"
        );
        let mut query = sql::query_as::<_, Row>(AssertSqlSafe(statement));
        for bind in binds {
            query = query.bind(*bind);
        }
        query
            .fetch_all(self.pool())
            .await
            .expect("read change_log")
            .into_iter()
            .map(
                |(id, entity_type, entity_id, operation, version, payload, device_id)| {
                    ChangeLogRow {
                        id,
                        entity_type,
                        entity_id,
                        operation,
                        version,
                        payload,
                        device_id,
                    }
                },
            )
            .collect()
    }
}
