//! Adds the peer's `node_id` to `paired_devices`.
//!
//! Sync data moves onto iroh streams, where a peer is addressed by its node id
//! and nothing else: iroh resolves that id to a direct address, a hole-punched
//! path, or the relay. The row only knew an HTTP `listen_on` endpoint, which
//! goes stale every time the peer rebinds — and cannot be used at all by a
//! transport that never opens a port.
//!
//! Existing rows keep `NULL` and stay reachable over HTTP until the peer is
//! paired again; a claim or an approval records the id from then on.

use sea_orm_migration::prelude::*;

use super::schema::column_exists;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0010-paired_devices_node_id"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !column_exists(manager, "paired_devices", "node_id").await? {
            manager
                .get_connection()
                .execute_unprepared("ALTER TABLE paired_devices ADD COLUMN node_id TEXT NULL")
                .await
                .map(|_| ())?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if column_exists(manager, "paired_devices", "node_id").await? {
            manager
                .get_connection()
                .execute_unprepared("ALTER TABLE paired_devices DROP COLUMN node_id")
                .await
                .map(|_| ())?;
        }
        Ok(())
    }
}
