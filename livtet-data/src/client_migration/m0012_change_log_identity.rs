//! Remote change_log identity deduplication.
//!
//! Remote changes arrive with a stable `(entity_type, entity_id, version, device_id, operation)`
//! tuple.  This migration:
//!
//! 1. Removes any existing duplicate remote rows.
//! 2. Adds a partial unique index on that tuple (restricted to non-`local` device IDs)
//!    to enforce uniqueness going forward.

use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0012-change_log_identity"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Remove duplicate remote rows, keeping only the first (MIN(id)) per
        // identity tuple.
        let dedupe = r#"
DELETE FROM change_log
WHERE device_id IS NOT NULL AND device_id <> 'local'
  AND id NOT IN (
    SELECT MIN(id) FROM change_log
    WHERE device_id IS NOT NULL AND device_id <> 'local'
    GROUP BY entity_type, entity_id, version, device_id, operation
  )
"#;
        manager
            .get_connection()
            .execute_unprepared(dedupe)
            .await
            .map(|_| ())?;

        // Create the partial unique index.
        let index = r#"
CREATE UNIQUE INDEX IF NOT EXISTS uq_change_log_remote_identity
ON change_log(entity_type, entity_id, version, device_id, operation)
WHERE device_id IS NOT NULL AND device_id <> 'local'
"#;
        manager
            .get_connection()
            .execute_unprepared(index)
            .await
            .map(|_| ())?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let drop = "DROP INDEX IF EXISTS uq_change_log_remote_identity";
        manager
            .get_connection()
            .execute_unprepared(drop)
            .await
            .map(|_| ())?;
        Ok(())
    }
}
