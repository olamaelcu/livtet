//! Adds the `origin_addr` column to `pending_pairings`.
//!
//! `POST /sync/pair` records the requesting peer's socket address so the
//! daemon can tell a ticket it minted itself (no origin) from a request that
//! arrived from another host. Locally minted tickets keep `origin_addr` null.
//!
//! `pending_pairings` is a STRICT table, and SQLite's STRICT type check
//! compares the declared type name exactly against its allow-list, so the
//! column is added with the bare STRICT-legal `TEXT` type. `Address` stores
//! as `ip:port` text, matching `listen_on`.

use sea_orm_migration::prelude::*;

use super::schema::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0007-pending_pairings_origin_addr"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !column_exists(manager, "pending_pairings", "origin_addr").await? {
            manager
                .get_connection()
                .execute_unprepared("ALTER TABLE pending_pairings ADD COLUMN origin_addr TEXT")
                .await
                .map(|_| ())?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if column_exists(manager, "pending_pairings", "origin_addr").await? {
            manager
                .alter_table(
                    Table::alter()
                        .table(PendingPairings::Table)
                        .drop_column(PendingPairings::OriginAddr)
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}
