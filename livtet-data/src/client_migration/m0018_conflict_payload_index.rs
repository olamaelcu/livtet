use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0018-conflict-payload-index"
    }
}

/// The old rows are duplicated, non-actionable, and cannot be replayed; the
/// unique open-conflict index makes a repeated pull update nothing.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DELETE FROM conflicts").await?;
        db.execute_unprepared(
            "CREATE UNIQUE INDEX IF NOT EXISTS uq_conflicts_open_payloads \
             ON conflicts(entity_type, entity_id, local_payload, remote_payload) \
             WHERE resolved = 0",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP INDEX IF EXISTS uq_conflicts_open_payloads")
            .await?;
        Ok(())
    }
}
