//! Stop syncing `digital_inventory`.
//!
//! That table describes *this device's* copy: an absolute path, a cover path, and the
//! hashes of files that exist here. Sending it tells a peer about a disk it cannot see —
//! a phone that pulls a desktop's library ends up with rows claiming files under
//! `/home/…`, which is why a shelf full of books showed none of them as readable.
//!
//! The offer path still carries everything a peer needs to make its own row: the file
//! bytes, the size, the hash, and the cover. Accepting an offer writes the local row
//! ([`crate::client_migration::m0014_inventory_not_synced`] removes the triggers, so
//! writing it does not immediately re-announce it either).
//!
//! `down` restores the triggers from the migration that installed them, so the two
//! directions cannot drift.

use sea_orm_migration::prelude::*;

use super::m0005_sync_triggers::TRIGGER_STATEMENTS;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0014-inventory-not-synced"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for &(name, _sql) in TRIGGER_STATEMENTS {
            if !name.contains("digital_inventory") {
                continue;
            }
            manager
                .get_connection()
                .execute_unprepared(&format!("DROP TRIGGER IF EXISTS {name}"))
                .await
                .map(|_| ())?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (name, sql) in TRIGGER_STATEMENTS {
            if !name.contains("digital_inventory") {
                continue;
            }
            manager
                .get_connection()
                .execute_unprepared(sql)
                .await
                .map(|_| ())?;
        }
        Ok(())
    }
}
