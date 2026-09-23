//! Adds the `device_id` column to `pending_pairings`.
//!
//! `POST /sync/pair` receives the remote device's ULID, and the pairing
//! approval step needs that id to create the matching `paired_devices` row.
//! The column stores the same 16-byte ULID representation as
//! `paired_devices.device_id`, so the entity exposes it as `Option<DbId>`.
//!
//! `pending_pairings` is a STRICT table, and SQLite's STRICT type check
//! compares the declared type name exactly against its allow-list
//! (`INT`, `INTEGER`, `REAL`, `TEXT`, `BLOB`, `ANY`). SeaORM renders a
//! `DbId` column (`db_id`/`db_id_null`, i.e. `binary_len(16)`) as
//! `blob(16)`, and the `(16)` suffix is rejected by STRICT. The
//! table-creation helpers normalize that away, but `ALTER TABLE ADD COLUMN`
//! bypasses the normalization, so the column is added with the bare
//! STRICT-legal `BLOB` type. `DbId`'s byte-based conversion reads it back
//! unchanged.

use sea_orm_migration::prelude::*;

use super::schema::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0006-pending_pairings_device_id"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !column_exists(manager, "pending_pairings", "device_id").await? {
            manager
                .get_connection()
                .execute_unprepared("ALTER TABLE pending_pairings ADD COLUMN device_id BLOB")
                .await
                .map(|_| ())?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if column_exists(manager, "pending_pairings", "device_id").await? {
            manager
                .alter_table(
                    Table::alter()
                        .table(PendingPairings::Table)
                        .drop_column(PendingPairings::DeviceId)
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}
