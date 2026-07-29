use sea_orm_migration::prelude::*;

use super::schema::*;

/// Adds the `session_token` column to `paired_devices`.
///
/// The Tauri `approve_pairing` command mints a 26-char Crockford ULID and
/// emits it via SSE (`PairingDecision.session_token`) so the mobile client
/// can authenticate subsequent sync requests. Persisting the token on the
/// paired row makes it round-trippable from `get_paired_devices` without
/// a separate minting step, and the UNIQUE index prevents accidental
/// collisions if the mobile client ever retries the approval flow.
///
/// SQLite <= 3.37 cannot add a `UNIQUE` constraint inline via `ALTER TABLE
/// ADD COLUMN`; the column must be added first, then a separate `CREATE
/// UNIQUE INDEX` enforces uniqueness. We follow that two-step recipe here.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0003-session_tokens"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(PairedDevices::Table)
                    .add_column(ColumnDef::new(PairedDevices::SessionToken).text().null())
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("uq_paired_devices_session_token")
                    .table(PairedDevices::Table)
                    .col(PairedDevices::SessionToken)
                    .unique()
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("uq_paired_devices_session_token")
                    .table(PairedDevices::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(PairedDevices::Table)
                    .drop_column(PairedDevices::SessionToken)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}
