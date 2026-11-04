//! Splits the pairing token column into the two directions a pairing has.
//!
//! A pairing mints a token on *each* side. The side that approves issues one and
//! checks every inbound request from the peer against it; the side that files a
//! claim is handed one by the peer and must present it when it dials out. Both
//! used to share `session_token`, so whichever direction was written last won and
//! the other direction's requests were rejected as `Unauthorized`.
//!
//! `session_token` keeps its meaning: the token this installation issued, which
//! inbound requests are matched against. `peer_session_token` holds the one to
//! present outbound. Existing rows are copied across because the single stored
//! token was previously used for both reads, so copying preserves exactly the
//! access a device already had.

use sea_orm_migration::prelude::*;

use super::schema::column_exists;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0013-paired_devices_peer_token"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !column_exists(manager, "paired_devices", "peer_session_token").await? {
            manager
                .get_connection()
                .execute_unprepared("ALTER TABLE paired_devices ADD COLUMN peer_session_token TEXT")
                .await
                .map(|_| ())?;
        }

        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE paired_devices SET peer_session_token = session_token \
                 WHERE peer_session_token IS NULL AND session_token IS NOT NULL \
                   AND session_token != ''",
            )
            .await
            .map(|_| ())?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if column_exists(manager, "paired_devices", "peer_session_token").await? {
            manager
                .get_connection()
                .execute_unprepared("ALTER TABLE paired_devices DROP COLUMN peer_session_token")
                .await
                .map(|_| ())?;
        }
        Ok(())
    }
}
