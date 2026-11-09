use sea_orm_migration::prelude::*;

use super::schema::column_exists;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0017-pending-pairings-claimant-token"
    }
}

/// The token the claimant minted for this device, held until a human approves
/// and it can be stored as the paired row's outbound token.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !column_exists(manager, "pending_pairings", "claimant_token").await? {
            manager
                .get_connection()
                .execute_unprepared("ALTER TABLE pending_pairings ADD COLUMN claimant_token TEXT")
                .await
                .map(|_| ())?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if column_exists(manager, "pending_pairings", "claimant_token").await? {
            manager
                .get_connection()
                .execute_unprepared("ALTER TABLE pending_pairings DROP COLUMN claimant_token")
                .await
                .map(|_| ())?;
        }
        Ok(())
    }
}
