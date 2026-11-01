//! Creates the `inbound_offers` table.
//!
//! A paired device offers a book by appending a `file_offer` change row; the
//! receiver used to download it on sight. The row recorded here is the offer
//! as the reader sees it — who sent it, what the book looks like, and the
//! catalog rows held back until they decide — so pulling no longer means
//! accepting.

use sea_orm_migration::prelude::*;

use super::schema::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0009-inbound_offers"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        create_strict_table(
            manager,
            &Table::create()
                .table(InboundOffers::Table)
                .if_not_exists()
                .col(string(InboundOffers::OfferId).primary_key())
                .col(string(InboundOffers::FromDeviceId))
                .col(string(InboundOffers::InventoryId))
                .col(string(InboundOffers::EditionId))
                .col(string(InboundOffers::WorkId))
                .col(string(InboundOffers::Title))
                .col(text(InboundOffers::Authors))
                .col(string_null(InboundOffers::FileName))
                .col(big_integer_null(InboundOffers::FileSizeBytes))
                .col(string_null(InboundOffers::FileHash))
                .col(boolean(InboundOffers::HasCover))
                .col(string(InboundOffers::OfferedAt))
                .col(string(InboundOffers::ReceivedAt))
                .col(string(InboundOffers::Status))
                .col(string_null(InboundOffers::DecidedAt))
                .col(text(InboundOffers::DeferredChanges))
                .to_owned(),
        )
        .await?;

        // The panel lists pending offers and the pull pass looks them up by
        // the inventory they carry, so both get an index.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS ix_inbound_offers_status \
                 ON inbound_offers (status, received_at)",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP INDEX IF EXISTS ix_inbound_offers_status")
            .await?;
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS inbound_offers")
            .await?;
        Ok(())
    }
}
