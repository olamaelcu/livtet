//! Trigger suppression for remote sync applies.
//!
//! While a remote change is being applied, the client sets
//! `client_settings.sync_apply = '1'`.  Every `sync_*_changelog_*` audit
//! trigger checks this setting before writing to `change_log`, so applying
//! a remote row does not write a second change-log entry.
//!
//! The guard is a `WHEN` clause on each trigger body:
//!
//! ```text
//! WHEN COALESCE((SELECT value FROM client_settings WHERE key = 'sync_apply'), '0') <> '1'
//! ```

use sea_orm_migration::prelude::*;

use super::m0005_sync_triggers::TRIGGER_STATEMENTS;
use super::schema::table_exists;

const GUARD: &str =
    "COALESCE((SELECT value FROM client_settings WHERE key = 'sync_apply'), '0') <> '1'";

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0011-trigger_suppression"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for &(name, sql) in TRIGGER_STATEMENTS {
            let drop = format!("DROP TRIGGER IF EXISTS {name}");
            manager
                .get_connection()
                .execute_unprepared(&drop)
                .await
                .map(|_| ())?;

            // SQLite refuses to create a trigger for a missing table;
            // client migrations may run standalone against a fresh database
            // (m0005 skips on the same condition).
            let Some(table) = trigger_table(sql) else {
                return Err(DbErr::Custom(format!(
                    "could not determine target table for trigger `{name}`"
                )));
            };
            if !table_exists(manager, table).await? {
                continue;
            }

            // Insert the guard right after the leading ` BEGIN` of the
            // original CREATE TRIGGER body. `replacen` with count 1 targets
            // only that first occurrence.
            let guarded = sql.replacen(" BEGIN", &format!(" WHEN {GUARD} BEGIN"), 1);
            manager
                .get_connection()
                .execute_unprepared(&guarded)
                .await
                .map(|_| ())?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for &(name, sql) in TRIGGER_STATEMENTS {
            let drop = format!("DROP TRIGGER IF EXISTS {name}");
            manager
                .get_connection()
                .execute_unprepared(&drop)
                .await
                .map(|_| ())?;

            // Skip triggers whose business table no longer exists.
            let Some(table) = trigger_table(sql) else {
                return Err(DbErr::Custom(format!(
                    "could not determine target table for trigger `{name}`"
                )));
            };
            if table_exists(manager, table).await? {
                manager
                    .get_connection()
                    .execute_unprepared(sql)
                    .await
                    .map(|_| ())?;
            }
        }
        Ok(())
    }
}

/// Extract the target table from a `CREATE TRIGGER ... ON <table> BEGIN ...`
/// statement.
fn trigger_table(sql: &str) -> Option<&str> {
    sql.split_once(" ON ")
        .and_then(|(_, rest)| rest.split_whitespace().next())
}
