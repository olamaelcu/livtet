//! Adds the `outbound` marker to `paired_devices`.
//!
//! A pairing row can mean two different things: this desktop approved the
//! peer (the token was issued by us, so the peer can pull), or this desktop
//! claimed the peer (the token was issued by them, so we can pull). The
//! receiver loop must only use the second kind, because the first token is
//! meaningless on the peer's server. Existing rows are inbound approvals, so
//! the default is `0`.

use sea_orm_migration::prelude::*;

use super::schema::column_exists;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0008-paired_devices_outbound"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !column_exists(manager, "paired_devices", "outbound").await? {
            manager
                .get_connection()
                .execute_unprepared(
                    "ALTER TABLE paired_devices ADD COLUMN outbound INTEGER NOT NULL DEFAULT 0",
                )
                .await
                .map(|_| ())?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if column_exists(manager, "paired_devices", "outbound").await? {
            manager
                .get_connection()
                .execute_unprepared("ALTER TABLE paired_devices DROP COLUMN outbound")
                .await
                .map(|_| ())?;
        }
        Ok(())
    }
}
