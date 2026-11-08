//! Client-owned database migrations.
//!
//! Migrations create client-side tables: sync change-log, device pairing,
//! session tokens, and client settings.

pub mod m0001_change_log;
pub mod m0002_pairing_tables;
pub mod m0003_session_tokens;
pub mod m0004_client_settings;
pub mod m0005_sync_triggers;
pub mod m0006_pending_pairings_device_id;
pub mod m0007_pending_pairings_origin_addr;
pub mod m0008_paired_devices_outbound;
pub mod m0009_inbound_offers;
pub mod m0010_paired_devices_node_id;
pub mod m0011_trigger_suppression;
pub mod m0012_change_log_identity;
pub mod m0013_paired_devices_peer_token;
pub mod m0014_inventory_not_synced;
pub mod m0015_sync_reading_progress_annotations;
pub mod m0016_sync_reading_progress_suppression;
pub mod schema;

pub use sea_orm_migration::MigratorTrait;
use sea_orm_migration::{MigrationTrait, prelude::*, sea_orm as orm};

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migration_table_name() -> sea_orm::DynIden {
        "client_migrations".into_iden()
    }

    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(crate::client_migration::m0001_change_log::Migration),
            Box::new(crate::client_migration::m0002_pairing_tables::Migration),
            Box::new(crate::client_migration::m0003_session_tokens::Migration),
            Box::new(crate::client_migration::m0004_client_settings::Migration),
            Box::new(crate::client_migration::m0005_sync_triggers::Migration),
            Box::new(crate::client_migration::m0006_pending_pairings_device_id::Migration),
            Box::new(crate::client_migration::m0007_pending_pairings_origin_addr::Migration),
            Box::new(crate::client_migration::m0008_paired_devices_outbound::Migration),
            Box::new(crate::client_migration::m0009_inbound_offers::Migration),
            Box::new(crate::client_migration::m0010_paired_devices_node_id::Migration),
            Box::new(crate::client_migration::m0011_trigger_suppression::Migration),
            Box::new(crate::client_migration::m0012_change_log_identity::Migration),
            Box::new(crate::client_migration::m0013_paired_devices_peer_token::Migration),
            Box::new(crate::client_migration::m0014_inventory_not_synced::Migration),
            Box::new(crate::client_migration::m0015_sync_reading_progress_annotations::Migration),
            Box::new(crate::client_migration::m0016_sync_reading_progress_suppression::Migration),
        ]
    }
}

impl Migrator {
    pub async fn run(pool: &sqlx::SqlitePool) -> Result<(), sqlx::Error> {
        let db = orm::SqlxSqliteConnector::from_sqlx_sqlite_pool(pool.clone());
        Migrator::up(&db, None)
            .await
            .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use sea_orm_migration::MigratorTrait;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    use super::Migrator;

    /// A single-connection in-memory pool. With more than one connection each
    /// pooled connection would get its own empty `:memory:` database, so the
    /// schema written by one statement would be invisible to the next.
    async fn memory_pool() -> sqlx::SqlitePool {
        let opts =
            SqliteConnectOptions::from_str("sqlite::memory:").expect("valid sqlite in-memory URL");
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("open in-memory sqlite pool")
    }

    #[tokio::test]
    async fn client_migrations_rerun_safe_and_install_triggers() {
        let pool = memory_pool().await;

        // The sync audit triggers target business tables, and SQLite refuses
        // to create a trigger for a table that does not exist, so the
        // business schema must be present before the triggers are installed.
        crate::migration::Migrator::run(&pool)
            .await
            .expect("business migrations");

        Migrator::run(&pool).await.expect("first client run");
        Migrator::run(&pool).await.expect("second client run");

        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master WHERE type = 'trigger' AND name LIKE 'sync_%'",
        )
        .fetch_one(&pool)
        .await
        .expect("count sync triggers");

        assert!(count > 0, "expected sync audit triggers, got {count}");
    }

    async fn work_log_rows(pool: &sqlx::SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM change_log WHERE entity_type = 'work'")
            .fetch_one(pool)
            .await
            .expect("count work change-log rows")
    }

    /// Insert one `change_log` row; `device` of `Some("remote")` marks it
    /// as a pushed change, which the partial unique index covers.
    async fn insert_log_row(
        pool: &sqlx::SqlitePool,
        device: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO change_log \
                 (entity_type, entity_id, operation, version, payload, changed_at, device_id) \
             VALUES ('work', 'aa', 'INSERT', 1, '{}', datetime('now'), ?)",
        )
        .bind(device)
        .execute(pool)
        .await
        .map(|_| ())
    }

    #[tokio::test]
    async fn client_migrations_install_the_sync_apply_guard() {
        let pool = memory_pool().await;
        crate::migration::Migrator::run(&pool)
            .await
            .expect("business migrations");
        Migrator::run(&pool).await.expect("client migrations");

        let sql: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master \
             WHERE type = 'trigger' AND name = 'sync_work_changelog_insert'",
        )
        .fetch_one(&pool)
        .await
        .expect("trigger definition");

        assert!(
            sql.contains("sync_apply"),
            "the audit triggers must honour the apply flag: {sql}"
        );
    }

    #[tokio::test]
    async fn sync_apply_guard_suppresses_change_log() {
        let pool = memory_pool().await;
        crate::migration::Migrator::run(&pool)
            .await
            .expect("business migrations");
        Migrator::run(&pool).await.expect("client migrations");

        sqlx::query(
            "INSERT INTO client_settings (key, value, updated_at) \
             VALUES ('sync_apply', '1', datetime('now'))",
        )
        .execute(&pool)
        .await
        .expect("set the apply flag");

        let before = work_log_rows(&pool).await;
        sqlx::query(
            "INSERT INTO works (id, title, created_at) VALUES (?, 'Guarded', datetime('now'))",
        )
        .bind(&[1u8; 16][..])
        .execute(&pool)
        .await
        .expect("insert a works row with the flag set");
        assert_eq!(
            work_log_rows(&pool).await,
            before,
            "the guard must suppress the audit trigger"
        );

        sqlx::query("DELETE FROM client_settings WHERE key = 'sync_apply'")
            .execute(&pool)
            .await
            .expect("clear the apply flag");
        sqlx::query(
            "INSERT INTO works (id, title, created_at) VALUES (?, 'Logged', datetime('now'))",
        )
        .bind(&[2u8; 16][..])
        .execute(&pool)
        .await
        .expect("insert a works row with the flag cleared");
        assert_eq!(work_log_rows(&pool).await, before + 1);
    }

    #[tokio::test]
    async fn remote_change_log_identity_is_unique() {
        let pool = memory_pool().await;
        crate::migration::Migrator::run(&pool)
            .await
            .expect("business migrations");
        Migrator::run(&pool).await.expect("client migrations");

        insert_log_row(&pool, Some("remote"))
            .await
            .expect("first remote row");
        let duplicate = insert_log_row(&pool, Some("remote")).await;
        assert!(
            duplicate.is_err(),
            "a replayed remote change must violate the identity index"
        );

        insert_log_row(&pool, None).await.expect("first local row");
        insert_log_row(&pool, None).await.expect("second local row");
    }

    /// A pairing made before the token split keeps working: the single token it
    /// stored is copied into the outbound slot, which is where the old code read
    /// it from when dialling.
    #[tokio::test]
    async fn peer_session_token_is_added_and_backfilled() {
        let pool = memory_pool().await;
        crate::migration::Migrator::run(&pool)
            .await
            .expect("business migrations");

        let db =
            sea_orm_migration::sea_orm::SqlxSqliteConnector::from_sqlx_sqlite_pool(pool.clone());
        // Stop one migration short, then pair a device the way the old code did:
        // one token, no peer slot.
        Migrator::up(&db, Some(12))
            .await
            .expect("client migrations to 12");
        sqlx::query(
            "INSERT INTO paired_devices (device_id, paired_at, session_token) \
             VALUES (x'00', datetime('now'), 'tok-before-the-split')",
        )
        .execute(&pool)
        .await
        .expect("insert a pre-split pairing");

        Migrator::up(&db, None).await.expect("the rest");

        let peer: Option<String> = sqlx::query_scalar(
            "SELECT peer_session_token FROM paired_devices WHERE device_id = x'00'",
        )
        .fetch_one(&pool)
        .await
        .expect("read the peer token");
        assert_eq!(peer.as_deref(), Some("tok-before-the-split"));

        let issued: Option<String> =
            sqlx::query_scalar("SELECT session_token FROM paired_devices WHERE device_id = x'00'")
                .fetch_one(&pool)
                .await
                .expect("read the issued token");
        assert_eq!(
            issued.as_deref(),
            Some("tok-before-the-split"),
            "the inbound slot is left as it was"
        );
    }
}
