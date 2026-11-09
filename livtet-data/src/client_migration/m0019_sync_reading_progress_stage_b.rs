//! Include the progress arbitration columns in sync payloads.
//!
//! Stage B stores `updated_at` and `updated_by` on `reading_progress`; peers
//! need them to decide which progress fields win. The trigger is recreated with
//! the two extra JSON fields, keeping the sync-apply suppression guard.

use sea_orm_migration::prelude::*;

use super::m0015_sync_reading_progress_annotations::SYNC_APPLY_GUARD;

pub struct Migration;

/// The reading-progress audit trigger, now carrying arbitration metadata.
pub(crate) fn trigger(name: &str, event: &str) -> String {
    let (operation, row, id) = match name {
        "insert" => ("INSERT", "new", "new"),
        "update" => ("UPDATE", "new", "new"),
        _ => ("DELETE", "old", "old"),
    };
    format!(
        r#"
CREATE TRIGGER IF NOT EXISTS sync_reading_progress_changelog_{name} {event} ON reading_progress
WHEN {guard}
BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'reading_progress', lower(hex({id}.id)), '{operation}',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex({row}.id)),
            'edition_id', lower(hex({row}.edition_id)),
            'format_id', lower(hex({row}.format_id)),
            'progress', {row}.progress,
            'progress_unit', {row}.progress_unit,
            'last_location', {row}.last_location,
            'total_reading_time_secs', {row}.total_reading_time_secs,
            'annotations', {row}.annotations,
            'created_at', {row}.created_at,
            'updated_at', {row}.updated_at,
            'updated_by', {row}.updated_by
        ),
        datetime('now'), 'local'
    );
END
"#,
        guard = SYNC_APPLY_GUARD,
    )
}

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0019-sync-reading-progress-stage-b"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        if !manager.has_table("reading_progress").await? {
            return Ok(());
        }
        for (name, event) in [
            ("insert", "AFTER INSERT"),
            ("update", "AFTER UPDATE"),
            ("delete", "AFTER DELETE"),
        ] {
            db.execute_unprepared(&format!(
                "DROP TRIGGER IF EXISTS sync_reading_progress_changelog_{name}"
            ))
            .await?;
            db.execute_unprepared(&trigger(name, event)).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        if !manager.has_table("reading_progress").await? {
            return Ok(());
        }
        for (name, event) in [
            ("insert", "AFTER INSERT"),
            ("update", "AFTER UPDATE"),
            ("delete", "AFTER DELETE"),
        ] {
            db.execute_unprepared(&format!(
                "DROP TRIGGER IF EXISTS sync_reading_progress_changelog_{name}"
            ))
            .await?;
            db.execute_unprepared(
                &super::m0015_sync_reading_progress_annotations::reading_progress_trigger(
                    name, event,
                ),
            )
            .await?;
        }
        Ok(())
    }
}
