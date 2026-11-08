use sea_orm_migration::prelude::*;

use super::m0015_sync_reading_progress_annotations::reading_progress_trigger;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0016-sync-reading-progress-suppression"
    }
}

/// m0015 recreated the `reading_progress` triggers without the m0011
/// sync-apply guard, so a change applied from a peer was logged again as
/// local. Recreate the three triggers with the guard.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !manager.has_table("reading_progress").await? {
            return Ok(());
        }
        let db = manager.get_connection();
        for (name, event) in [
            ("insert", "AFTER INSERT"),
            ("update", "AFTER UPDATE"),
            ("delete", "AFTER DELETE"),
        ] {
            db.execute_unprepared(&format!(
                "DROP TRIGGER IF EXISTS sync_reading_progress_changelog_{name}"
            ))
            .await?;
            let payload = reading_progress_trigger(name, event);
            db.execute_unprepared(&payload).await?;
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // The previous triggers are the bug this migration repairs.
        Ok(())
    }
}
