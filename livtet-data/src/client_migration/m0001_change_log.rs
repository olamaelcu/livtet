use sea_orm_migration::{prelude::*, schema::*};

use super::schema::ChangeLog;

/// DDL for the client-owned `conflicts` table.
///
/// Sync conflict rows are written by the sync engine when a remote change
/// clashes with a local one. This replicates the `CONFLICTS_TABLE` constant
/// from the original sync crate verbatim so the client schema owns both
/// sync-side tables (`change_log` and `conflicts`) and no downstream crate
/// needs to create them.
const CONFLICTS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS conflicts (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_type    TEXT    NOT NULL,
    entity_id      TEXT    NOT NULL,
    local_payload  TEXT    NOT NULL,
    remote_payload TEXT    NOT NULL,
    resolved       INTEGER NOT NULL DEFAULT 0,
    resolution     TEXT,
    merged_payload TEXT,
    detected_at    TEXT    NOT NULL
)
"#;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0001-change_log"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ChangeLog::Table)
                    .if_not_exists()
                    .col(pk_auto(ChangeLog::Id))
                    .col(string(ChangeLog::EntityType).not_null())
                    .col(string(ChangeLog::EntityId).not_null())
                    .col(string(ChangeLog::Operation).not_null())
                    .col(integer(ChangeLog::Version).not_null())
                    .col(text(ChangeLog::Payload).not_null())
                    .col(timestamp_with_time_zone(ChangeLog::ChangedAt).not_null())
                    .col(string_null(ChangeLog::DeviceId))
                    .to_owned(),
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(CONFLICTS_TABLE)
            .await
            .map(|_| ())?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS change_log")
            .await
            .map(|_| ())?;

        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS conflicts")
            .await
            .map(|_| ())?;

        Ok(())
    }
}
