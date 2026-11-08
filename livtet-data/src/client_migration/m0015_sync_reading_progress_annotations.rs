use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0015-sync-reading-progress-annotations"
    }
}

/// Notes live in `reading_progress.annotations`, so the change-log payload has
/// to carry that column (and the unit the progress is measured in) or peers
/// never see an edit. The retired `annotation` triggers are dropped.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        // A note queued before the upgrade would push an entity type no peer
        // knows any more; the note itself already rode along with its progress.
        db.execute_unprepared("DELETE FROM change_log WHERE entity_type = 'annotation'")
            .await?;
        // A client-only database (migrations run without the business schema)
        // has no tables to put triggers on.
        let has_table = manager.has_table("reading_progress").await?;
        if !has_table {
            return Ok(());
        }
        for trigger in [
            "sync_annotation_changelog_insert",
            "sync_annotation_changelog_update",
            "sync_annotation_changelog_delete",
        ] {
            db.execute_unprepared(&format!("DROP TRIGGER IF EXISTS {trigger}"))
                .await?;
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
            let (operation, row, id) = match name {
                "insert" => ("INSERT", "new", "new"),
                "update" => ("UPDATE", "new", "new"),
                _ => ("DELETE", "old", "old"),
            };
            let payload = format!(
                r#"
CREATE TRIGGER IF NOT EXISTS sync_reading_progress_changelog_{name} {event} ON reading_progress BEGIN
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
            'created_at', {row}.created_at
        ),
        datetime('now'), 'local'
    );
END
"#
            );
            db.execute_unprepared(&payload).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Restoring the old payload only drops the notes from future changes;
        // it cannot un-send them.
        let db = manager.get_connection();
        for name in ["insert", "update", "delete"] {
            db.execute_unprepared(&format!(
                "DROP TRIGGER IF EXISTS sync_reading_progress_changelog_{name}"
            ))
            .await?;
        }
        Ok(())
    }
}
